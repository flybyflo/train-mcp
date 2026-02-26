use std::sync::Arc;
use std::time::Instant;

use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use serde_json::json;

use crate::catalog::create_catalog_payload;
use crate::executor::codemode::TOOL_ERROR_MARKER;
use crate::executor::quickjs::{ExecutorLimits, Mode, QuickJsExecutor};
use crate::metrics;
use crate::transit::{OebbTransitProvider, TransitProvider};

/// Input for both the `search` and `execute` MCP tools.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CodeInput {
    /// The JavaScript code to execute in the sandbox.
    pub code: String,
}

/// The MCP server handler.
#[derive(Clone)]
pub struct TrainMcp {
    executor: Arc<QuickJsExecutor>,
    tool_router: ToolRouter<Self>,
}

impl TrainMcp {
    pub fn new(oebb_base_url: String) -> Self {
        let provider = Arc::new(OebbTransitProvider::new(oebb_base_url));
        Self::new_with_provider_and_limits(provider, ExecutorLimits::default())
    }

    pub fn new_with_executor_limits(oebb_base_url: String, limits: ExecutorLimits) -> Self {
        let provider = Arc::new(OebbTransitProvider::new(oebb_base_url));
        Self::new_with_provider_and_limits(provider, limits)
    }

    pub fn new_with_provider(provider: Arc<dyn TransitProvider>) -> Self {
        Self::new_with_provider_and_limits(provider, ExecutorLimits::default())
    }

    pub fn new_with_provider_and_limits(
        provider: Arc<dyn TransitProvider>,
        limits: ExecutorLimits,
    ) -> Self {
        let catalog = create_catalog_payload();
        let executor = Arc::new(QuickJsExecutor::with_limits(provider, catalog, limits));
        Self {
            executor,
            tool_router: Self::tool_router(),
        }
    }

    fn build_call_tool_result(result: crate::executor::quickjs::ExecuteResult) -> CallToolResult {
        let (normalized_result, structured_error) = extract_structured_tool_error(result.result);
        let executor_error = result.error;

        let (error_code, error_message) = match (executor_error, structured_error) {
            (Some(message), _) => (Some("execution_error".to_string()), Some(message)),
            (None, Some((code, message))) => (Some(code), Some(message)),
            (None, None) => (None, None),
        };
        let is_ok = error_code.is_none();

        let payload = json!({
            "ok": is_ok,
            "result": normalized_result,
            "logs": result.logs,
            "error": error_code,
            "errorMessage": error_message,
        });

        if is_ok {
            CallToolResult::structured(payload)
        } else {
            CallToolResult::structured_error(payload)
        }
    }
}

fn extract_structured_tool_error(
    value: serde_json::Value,
) -> (serde_json::Value, Option<(String, String)>) {
    let mut obj = match value {
        serde_json::Value::Object(obj) => obj,
        other => return (other, None),
    };

    let is_tool_error = obj
        .get(TOOL_ERROR_MARKER)
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !is_tool_error {
        return (serde_json::Value::Object(obj), None);
    }

    obj.remove(TOOL_ERROR_MARKER);
    let code = obj
        .get("error")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("tool_error")
        .to_string();
    let message = obj
        .get("message")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("Tool execution failed.")
        .to_string();

    (serde_json::Value::Object(obj), Some((code, message)))
}

#[tool_router]
impl TrainMcp {
    /// Catalog-only mode — use to discover available tools, their parameters, and examples.
    /// Only `codemode.getCatalog({})` and `codemode.listTools({})` are available.
    /// Do NOT call transit tools here — use the `execute` tool instead.
    /// Example: `const tools = await codemode.listTools({}); return tools;`
    #[tool(name = "search")]
    async fn search(&self, params: Parameters<CodeInput>) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        let result = self.executor.execute(Mode::Search, &params.0.code).await;
        let outcome = if result.error.is_some() { "error" } else { "ok" };
        metrics::observe_tool_call("search", outcome, started.elapsed());
        Ok(Self::build_call_tool_result(result))
    }

    /// Transit execution mode. All tools are called as `const result = await codemode.<toolName>({...}); return result;`
    /// Available: `oebbPlanJourney`, `oebbPlanTour`, `oebbResolveItineraryStops`, `oebbLocations`, `oebbDepartures`, `oebbJourneys`, `oebbTrip`.
    /// Catalog helpers (`getCatalog`, `listTools`) are NOT available here — use the `search` tool for those.
    /// IMPORTANT: Always `return` the result. Do NOT use `codemode.callTool(...)` — call functions directly on `codemode`.
    /// Example 1 (single journey): `const j = await codemode.oebbPlanJourney({ from: "Wien Hbf", to: "Salzburg Hbf", departure: "2026-02-27T08:00:00+01:00" }); return j;`
    /// Example 2 (multi-city tour): `const t = await codemode.oebbPlanTour({ departure: "2026-02-27T08:00:00+01:00", legs: [{ from: "A", to: "B", minStopMinutesAfter: 90 }, { from: "B", to: "C" }] }); return t;`
    #[tool(name = "execute")]
    async fn execute(&self, params: Parameters<CodeInput>) -> Result<CallToolResult, McpError> {
        let started = Instant::now();
        let result = self.executor.execute(Mode::Execute, &params.0.code).await;
        let outcome = if result.error.is_some() { "error" } else { "ok" };
        metrics::observe_tool_call("execute", outcome, started.elapsed());
        Ok(Self::build_call_tool_result(result))
    }
}

#[tool_handler]
impl ServerHandler for TrainMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            protocol_version: ProtocolVersion::V_2024_11_05,
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            server_info: Implementation {
                name: "train-mcp".to_string(),
                version: "0.1.0".to_string(),
                title: None,
                description: None,
                icons: None,
                website_url: None,
            },
            instructions: Some(
                "Transit MCP server with CodeMode-style sandbox. Use the `search` tool to discover available functions, and the `execute` tool to call them. In the `execute` tool, the `codemode` object exposes each tool directly as a method (e.g. `codemode.oebbPlanTour({...})`). Do NOT use `codemode.callTool(\"name\", {...})` — that function does not exist and will fail.".to_string(),
            ),
        }
    }
}
