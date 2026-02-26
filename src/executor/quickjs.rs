//! QuickJS-based CodeMode executor.
//!
//! ## Security: Network Isolation
//!
//! QuickJS does **not** include `fetch`, `XMLHttpRequest`, `setTimeout`,
//! or any browser/Node I/O primitives. The only way sandboxed code can
//! interact with the outside world is through the injected `codemode.*`
//! functions (see the `codemode` module). This is equivalent to Cloudflare's
//! `globalOutbound: null` isolation mode.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rquickjs::{
    async_with, prelude::*, AsyncContext, AsyncRuntime, CatchResultExt, Function, Object,
    Value as JsValue,
};
use serde_json::Value;
use tokio::sync::{Mutex, Semaphore};

use crate::transit::TransitProvider;

use super::codemode;

/// Default execution timeout (matches Cloudflare's 30s default).
const DEFAULT_EXECUTION_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_EXECUTION_QUEUE_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_MAX_PARALLEL_EXECUTIONS: usize = 8;
const DEFAULT_MEMORY_LIMIT_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_MAX_STACK_BYTES: usize = 512 * 1024;
const DEFAULT_GC_THRESHOLD_BYTES: usize = 8 * 1024 * 1024;

/// Maximum allowed code length in characters.
const MAX_CODE_LENGTH: usize = 100_000;

/// The result of executing JS code in the sandbox.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ExecuteResult {
    pub result: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub logs: Vec<String>,
}

/// Runtime limits for sandboxed execution.
#[derive(Debug, Clone)]
pub struct ExecutorLimits {
    pub execution_timeout: Duration,
    pub queue_timeout: Duration,
    pub max_parallel_executions: usize,
    pub memory_limit_bytes: usize,
    pub max_stack_bytes: usize,
    pub gc_threshold_bytes: usize,
}

impl Default for ExecutorLimits {
    fn default() -> Self {
        Self {
            execution_timeout: duration_from_env_ms(
                "CODEMODE_EXECUTION_TIMEOUT_MS",
                DEFAULT_EXECUTION_TIMEOUT,
            ),
            queue_timeout: duration_from_env_ms(
                "CODEMODE_EXECUTION_QUEUE_TIMEOUT_MS",
                DEFAULT_EXECUTION_QUEUE_TIMEOUT,
            ),
            max_parallel_executions: usize_from_env(
                "CODEMODE_MAX_PARALLEL_EXECUTIONS",
                DEFAULT_MAX_PARALLEL_EXECUTIONS,
            )
            .max(1),
            memory_limit_bytes: usize_from_env(
                "CODEMODE_MEMORY_LIMIT_BYTES",
                DEFAULT_MEMORY_LIMIT_BYTES,
            ),
            max_stack_bytes: usize_from_env("CODEMODE_MAX_STACK_BYTES", DEFAULT_MAX_STACK_BYTES),
            gc_threshold_bytes: usize_from_env(
                "CODEMODE_GC_THRESHOLD_BYTES",
                DEFAULT_GC_THRESHOLD_BYTES,
            ),
        }
    }
}

/// The mode determines which `codemode.*` functions are available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Search,
    Execute,
}

/// CodeMode executor backed by QuickJS via rquickjs.
/// Mirrors Cloudflare's `Executor` interface: execute(code, fns) → { result, error?, logs? }
#[derive(Clone)]
pub struct QuickJsExecutor {
    provider: Arc<dyn TransitProvider>,
    catalog_json: Value,
    limits: ExecutorLimits,
    concurrency_gate: Arc<Semaphore>,
}

impl QuickJsExecutor {
    pub fn new(provider: Arc<dyn TransitProvider>, catalog_json: Value) -> Self {
        Self::with_limits(provider, catalog_json, ExecutorLimits::default())
    }

    pub fn with_limits(
        provider: Arc<dyn TransitProvider>,
        catalog_json: Value,
        limits: ExecutorLimits,
    ) -> Self {
        Self {
            provider,
            catalog_json,
            concurrency_gate: Arc::new(Semaphore::new(limits.max_parallel_executions)),
            limits,
        }
    }

    /// Execute user-supplied JS code in an isolated QuickJS context.
    /// Enforces a code length limit and execution timeout.
    pub async fn execute(&self, mode: Mode, code: &str) -> ExecuteResult {
        // Gap 2: Validate code length.
        if code.is_empty() {
            return ExecuteResult {
                result: Value::Null,
                error: Some("Code cannot be empty.".to_string()),
                logs: vec![],
            };
        }
        if code.len() > MAX_CODE_LENGTH {
            return ExecuteResult {
                result: Value::Null,
                error: Some(format!(
                    "Code exceeds maximum length of {} characters (got {}).",
                    MAX_CODE_LENGTH,
                    code.len()
                )),
                logs: vec![],
            };
        }

        let permit = match tokio::time::timeout(
            self.limits.queue_timeout,
            self.concurrency_gate.clone().acquire_owned(),
        )
        .await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => {
                return ExecuteResult {
                    result: Value::Null,
                    error: Some("Server is shutting down.".to_string()),
                    logs: vec![],
                };
            }
            Err(_) => {
                return ExecuteResult {
                    result: Value::Null,
                    error: Some(format!(
                        "Server is overloaded (execution queue exceeded {}ms).",
                        self.limits.queue_timeout.as_millis()
                    )),
                    logs: vec![],
                };
            }
        };

        let logs = Arc::new(Mutex::new(Vec::<String>::new()));
        let provider = self.provider.clone();
        let catalog = self.catalog_json.clone();
        let limits = self.limits.clone();
        let code = code.to_string();
        let logs_clone = logs.clone();

        // Gap 1: Wrap execution in a timeout.
        let execution = tokio::task::spawn_blocking(move || {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(async move {
                run_in_quickjs(mode, &code, provider, catalog, logs_clone, limits).await
            })
        });

        let result = execution.await;

        let captured_logs = logs.lock().await.clone();
        drop(permit);

        match result {
            Ok(Ok(value)) => ExecuteResult {
                result: value,
                error: None,
                logs: captured_logs,
            },
            Ok(Err(e)) => ExecuteResult {
                result: Value::Null,
                error: Some(e.to_string()),
                logs: captured_logs,
            },
            Err(e) => ExecuteResult {
                result: Value::Null,
                error: Some(format!("executor panicked: {}", e)),
                logs: captured_logs,
            },
        }
    }
}

async fn run_in_quickjs(
    mode: Mode,
    code: &str,
    provider: Arc<dyn TransitProvider>,
    catalog: Value,
    logs: Arc<Mutex<Vec<String>>>,
    limits: ExecutorLimits,
) -> anyhow::Result<Value> {
    let rt = AsyncRuntime::new()?;
    rt.set_memory_limit(limits.memory_limit_bytes).await;
    rt.set_max_stack_size(limits.max_stack_bytes).await;
    rt.set_gc_threshold(limits.gc_threshold_bytes).await;
    let started = Instant::now();
    let execution_timeout = limits.execution_timeout;
    rt.set_interrupt_handler(Some(Box::new(move || {
        started.elapsed() >= execution_timeout
    })))
    .await;
    let ctx = AsyncContext::full(&rt).await?;

    let code_owned = code.to_string();

    let result: Result<Value, anyhow::Error> = async_with!(ctx => |ctx| {
        // Inject console.log/warn/error.
        let globals = ctx.globals();
        let console = Object::new(ctx.clone())?;

        let logs_log = logs.clone();
        console.set("log", Function::new(ctx.clone(), MutFn::new(move |args: Rest<JsValue>| {
            let parts: Vec<String> = args.0.iter().map(|v| format_js_value(v)).collect();
            let msg = parts.join(" ");
            let logs = logs_log.clone();
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    logs.lock().await.push(msg);
                });
            });
        })))?;

        let logs_warn = logs.clone();
        console.set("warn", Function::new(ctx.clone(), MutFn::new(move |args: Rest<JsValue>| {
            let parts: Vec<String> = args.0.iter().map(|v| format_js_value(v)).collect();
            let msg = format!("[warn] {}", parts.join(" "));
            let logs = logs_warn.clone();
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    logs.lock().await.push(msg);
                });
            });
        })))?;

        let logs_err = logs.clone();
        console.set("error", Function::new(ctx.clone(), MutFn::new(move |args: Rest<JsValue>| {
            let parts: Vec<String> = args.0.iter().map(|v| format_js_value(v)).collect();
            let msg = format!("[error] {}", parts.join(" "));
            let logs = logs_err.clone();
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    logs.lock().await.push(msg);
                });
            });
        })))?;

        globals.set("console", console)?;

        // Inject codemode object based on mode.
        codemode::inject_codemode(&ctx, mode, provider, catalog)?;

        // Wrap code and evaluate.
        let wrapped = format!("(async () => {{ {} }})()", code_owned);
        let promise: rquickjs::Promise = ctx.eval(wrapped).catch(&ctx).map_err(|e| {
            anyhow::anyhow!("JS eval error: {:?}", e)
        })?;

        // Await the promise.
        let js_result: JsValue = match promise.into_future().await {
            Ok(val) => val,
            Err(_) => {
                // Extract the actual JS exception from the context for a useful error message.
                let exception_val = ctx.catch();
                let exception_msg = if let Some(s) = exception_val.clone().into_string() {
                    s.to_string().unwrap_or_else(|_| "Unknown JS exception".to_string())
                } else if let Some(obj) = exception_val.clone().into_object() {
                    // Try .message property first (standard Error objects)
                    obj.get::<_, String>("message")
                        .or_else(|_| obj.get::<_, String>("stack"))
                        .unwrap_or_else(|_| {
                            // Fall back to JSON.stringify
                            let ctx_clone = obj.ctx().clone();
                            ctx_clone.json_stringify(obj).ok().flatten()
                                .and_then(|s| s.to_string().ok())
                                .unwrap_or_else(|| "Unknown JS exception".to_string())
                        })
                } else {
                    format_js_value(&exception_val)
                };
                return Err(anyhow::anyhow!("JS promise rejected: {}", exception_msg));
            }
        };

        // Serialize JS result to serde_json::Value.
        let json = js_value_to_json(&js_result);
        Ok::<Value, anyhow::Error>(json)
    })
    .await;

    result
}

/// Convert a rquickjs JsValue to a serde_json Value.
fn js_value_to_json(value: &JsValue) -> Value {
    if value.is_undefined() || value.is_null() {
        return Value::Null;
    }
    if let Some(b) = value.as_bool() {
        return Value::Bool(b);
    }
    if let Some(n) = value.as_int() {
        return Value::Number(n.into());
    }
    if let Some(n) = value.as_float() {
        if n.is_finite() {
            return serde_json::json!(n);
        }
        return Value::Null;
    }
    if let Some(s) = value.clone().into_string() {
        if let Ok(owned) = s.to_string() {
            return Value::String(owned);
        }
    }
    if let Some(arr) = value.clone().into_array() {
        let items: Vec<Value> = (0..arr.len())
            .filter_map(|i| arr.get::<JsValue>(i).ok())
            .map(|v| js_value_to_json(&v))
            .collect();
        return Value::Array(items);
    }
    if let Some(obj) = value.clone().into_object() {
        let ctx = obj.ctx().clone();
        if let Ok(Some(s)) = ctx.json_stringify(obj.clone()) {
            if let Ok(owned) = s.to_string() {
                if let Ok(parsed) = serde_json::from_str::<Value>(&owned) {
                    return parsed;
                }
            }
        }
        let mut map = serde_json::Map::new();
        let keys_iter = obj.keys::<String>();
        for key in keys_iter.flatten() {
            if let Ok(val) = obj.get::<_, JsValue>(&key) {
                map.insert(key, js_value_to_json(&val));
            }
        }
        return Value::Object(map);
    }
    Value::Null
}

/// Format a JS value for console output.
fn format_js_value(value: &JsValue) -> String {
    if value.is_undefined() {
        return "undefined".to_string();
    }
    if value.is_null() {
        return "null".to_string();
    }
    if let Some(b) = value.as_bool() {
        return b.to_string();
    }
    if let Some(n) = value.as_int() {
        return n.to_string();
    }
    if let Some(n) = value.as_float() {
        return n.to_string();
    }
    if let Some(s) = value.clone().into_string() {
        if let Ok(owned) = s.to_string() {
            return owned;
        }
    }
    let json = js_value_to_json(value);
    serde_json::to_string(&json).unwrap_or_else(|_| "[object]".to_string())
}

fn usize_from_env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

fn duration_from_env_ms(name: &str, default: Duration) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(default)
}
