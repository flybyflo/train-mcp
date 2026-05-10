use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    body::{to_bytes, Body},
    extract::{Extension, Path, Query, Request, State},
    http::{header, HeaderValue, Method, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::tower::{
    StreamableHttpServerConfig, StreamableHttpService,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{Mutex, Semaphore};
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::EnvFilter;

use train_mcp::auth::{AuthContext, AuthService};
use train_mcp::catalog::{catalog_tools, create_catalog_payload};
use train_mcp::executor::codemode::dispatch_transit_op;
use train_mcp::executor::quickjs::ExecutorLimits;
use train_mcp::metrics;
use train_mcp::persistence::{Persistence, QueryLogInput};
use train_mcp::server::TrainMcp;
use train_mcp::transit::{OebbTransitProvider, TransitProvider};

#[derive(Clone)]
struct AppState {
    transit_provider: Arc<dyn TransitProvider>,
    request_guard: RequestGuard,
    mcp_service: StreamableHttpService<TrainMcp>,
    auth_service: Arc<AuthService>,
    persistence: Option<Arc<Persistence>>,
}

#[derive(Clone)]
struct RequestGuard {
    inflight: Arc<Semaphore>,
    queue_timeout: Duration,
    rate_limit_per_second: u64,
    rate_state: Arc<Mutex<RateState>>,
}

#[derive(Debug)]
struct RateState {
    window_started: Instant,
    count: u64,
}

#[derive(Debug, Clone)]
struct McpToolCallRecord {
    operation: String,
    payload: Value,
}

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    limit: Option<u32>,
    offset: Option<u32>,
}

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "service": "train-mcp" }))
}

async fn catalog_route() -> Json<serde_json::Value> {
    Json(create_catalog_payload())
}

async fn api_list_tools() -> Json<Value> {
    Json(json!({ "tools": catalog_tools() }))
}

async fn api_call_transit_op(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(op_name): Path<String>,
    Json(input): Json<Value>,
) -> impl IntoResponse {
    if !is_known_transit_op(&op_name) {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({
                "ok": false,
                "error": "unknown_operation",
                "message": format!("Unknown operation: {op_name}"),
            })),
        )
            .into_response();
    }

    let started = Instant::now();
    let input_for_log = input.clone();
    let result = dispatch_transit_op(&op_name, state.transit_provider.clone(), input).await;
    persist_if_authenticated(
        &state,
        &auth,
        QueryLogInput {
            source: "api".to_string(),
            operation: op_name,
            request_payload: input_for_log,
            response_payload: Some(result.clone()),
            succeeded: !is_tool_error_payload(&result),
            duration_ms: started.elapsed().as_millis() as i64,
        },
    )
    .await;
    (StatusCode::OK, Json(result)).into_response()
}

fn is_known_transit_op(name: &str) -> bool {
    matches!(
        name,
        "oebbPlanJourney"
            | "oebbPlanTour"
            | "oebbResolveItineraryStops"
            | "oebbLocations"
            | "oebbDepartures"
            | "oebbJourneys"
            | "oebbTrip"
    )
}

async fn api_session(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> impl IntoResponse {
    let Some(user) = auth.user() else {
        return unauthorized_response();
    };

    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "authEnabled": state.auth_service.is_enabled(),
            "persistenceEnabled": state.persistence.is_some(),
            "session": {
                "userId": user.user_id,
                "sessionKey": user.session_key,
                "sub": user.sub,
                "iss": user.iss,
                "email": user.email,
                "name": user.name,
                "jti": user.jti,
            }
        })),
    )
        .into_response()
}

async fn api_recent_queries(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(query): Query<HistoryQuery>,
) -> impl IntoResponse {
    let Some(user) = auth.user() else {
        return unauthorized_response();
    };
    let Some(persistence) = &state.persistence else {
        return persistence_unavailable_response();
    };

    match persistence
        .recent_queries(&user.user_id, query.limit, query.offset)
        .await
    {
        Ok(items) => (StatusCode::OK, Json(json!({ "ok": true, "items": items }))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "error": "history_query_failed",
                "message": error.to_string(),
            })),
        )
            .into_response(),
    }
}

async fn api_recent_journeys(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(query): Query<HistoryQuery>,
) -> impl IntoResponse {
    let Some(user) = auth.user() else {
        return unauthorized_response();
    };
    let Some(persistence) = &state.persistence else {
        return persistence_unavailable_response();
    };

    match persistence
        .recent_journeys(&user.user_id, query.limit, query.offset)
        .await
    {
        Ok(items) => (StatusCode::OK, Json(json!({ "ok": true, "items": items }))).into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({
                "ok": false,
                "error": "history_query_failed",
                "message": error.to_string(),
            })),
        )
            .into_response(),
    }
}

async fn metrics_route() -> Response {
    match metrics::gather_text() {
        Ok(payload) => (
            StatusCode::OK,
            [(
                header::CONTENT_TYPE,
                "text/plain; version=0.0.4; charset=utf-8",
            )],
            payload,
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "error": "metrics_encode_failed", "message": error })),
        )
            .into_response(),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("train_mcp=info".parse()?))
        .init();

    let oebb_base_url = std::env::var("OEBB_BASE_URL")
        .unwrap_or_else(|_| "https://v6.oebb.transport.rest/api".to_string());
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8000);
    let max_concurrency = usize_from_env("MCP_MAX_CONCURRENCY", 128).max(1);
    let rate_limit_per_second = u64_from_env("MCP_RATE_LIMIT_PER_SECOND", 30).max(1);
    let queue_timeout = duration_from_env_ms("MCP_QUEUE_TIMEOUT_MS", 250);
    let max_body_bytes = usize_from_env("MCP_MAX_BODY_BYTES", 128 * 1024).max(1024);
    let cors_layer = build_cors_layer();
    let request_guard = RequestGuard {
        inflight: Arc::new(Semaphore::new(max_concurrency)),
        queue_timeout,
        rate_limit_per_second,
        rate_state: Arc::new(Mutex::new(RateState {
            window_started: Instant::now(),
            count: 0,
        })),
    };

    // Build the StreamableHttpService as a Tower service for axum.
    let config = StreamableHttpServerConfig {
        // Stateless POST-only mode avoids session-header 401 responses that can
        // be misinterpreted by some clients as OAuth-required.
        stateful_mode: false,
        ..StreamableHttpServerConfig::default()
    };
    let session_manager = Arc::new(LocalSessionManager::default());

    let executor_limits = ExecutorLimits::default();
    let auth_service = Arc::new(AuthService::from_env());
    let persistence = Persistence::from_env().await?.map(Arc::new);
    let transit_provider: Arc<dyn TransitProvider> = Arc::new(OebbTransitProvider::new(
        oebb_base_url.clone(),
        persistence.clone(),
    ));

    // The service factory creates a new TrainMcp for each session (shared ÖBB provider).
    let provider_for_mcp = transit_provider.clone();
    let limits_for_mcp = executor_limits.clone();
    let mcp_service = StreamableHttpService::new(
        move || {
            Ok(TrainMcp::new_with_provider_and_limits(
                provider_for_mcp.clone(),
                limits_for_mcp.clone(),
            ))
        },
        session_manager,
        config,
    );

    let app_state = AppState {
        transit_provider,
        request_guard: request_guard.clone(),
        mcp_service,
        auth_service,
        persistence,
    };

    let api_tools = Router::new()
        .route("/", get(api_list_tools))
        .route("/{op_name}", post(api_call_transit_op))
        .route_layer(middleware::from_fn_with_state(
            app_state.clone(),
            resolve_auth_context,
        ))
        .route_layer(middleware::from_fn_with_state(
            app_state.clone(),
            enforce_request_limits,
        ));

    let api_history = Router::new()
        .route("/session", get(api_session))
        .route("/history/queries", get(api_recent_queries))
        .route("/history/journeys", get(api_recent_journeys))
        .route_layer(middleware::from_fn_with_state(
            app_state.clone(),
            resolve_auth_context,
        ))
        .route_layer(middleware::from_fn_with_state(
            app_state.clone(),
            enforce_request_limits,
        ));

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/catalog", get(catalog_route))
        .route("/metrics", get(metrics_route))
        .nest("/api", api_history)
        .nest("/api/tools", api_tools)
        .route(
            "/mcp",
            post(mcp_handler)
                .route_layer(middleware::from_fn_with_state(
                    app_state.clone(),
                    resolve_auth_context,
                ))
                .route_layer(middleware::from_fn_with_state(
                    app_state.clone(),
                    enforce_request_limits,
                )),
        )
        .with_state(app_state)
        .layer(axum::extract::DefaultBodyLimit::max(max_body_bytes))
        .layer(cors_layer);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("train-mcp listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn mcp_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    Extension(auth): Extension<AuthContext>,
    request: axum::extract::Request,
) -> impl axum::response::IntoResponse {
    let mut service = state.mcp_service.clone();
    const MCP_PARSE_MAX_BYTES: usize = 256 * 1024;
    use tower_service::Service;
    let started = Instant::now();

    let (parts, body) = request.into_parts();
    let body_bytes = match to_bytes(body, MCP_PARSE_MAX_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            metrics::observe_protocol_invalid_payload("body_too_large_or_unreadable");
            metrics::observe_protocol_request("invalid_payload", "error");
            return (
                StatusCode::PAYLOAD_TOO_LARGE,
                Json(json!({
                    "ok": false,
                    "error": "payload_too_large",
                    "message": "MCP request body is too large or unreadable.",
                })),
            )
                .into_response();
        }
    };

    let (protocol_methods, invalid_payload) = extract_protocol_methods(&body_bytes);
    let tool_calls = extract_mcp_tool_calls(&body_bytes);
    if invalid_payload {
        metrics::observe_protocol_invalid_payload("invalid_json_rpc");
    }

    let request = axum::http::Request::from_parts(parts, Body::from(body_bytes));
    let response = service.call(request).await;
    match response {
        Ok(resp) => {
            let outcome = if resp.status().is_success() {
                "ok"
            } else {
                "error"
            };
            for method in protocol_methods {
                metrics::observe_protocol_request(method, outcome);
            }

            if !tool_calls.is_empty() {
                for call in tool_calls {
                    persist_if_authenticated(
                        &state,
                        &auth,
                        QueryLogInput {
                            source: "mcp".to_string(),
                            operation: call.operation,
                            request_payload: call.payload,
                            response_payload: Some(json!({ "httpStatus": resp.status().as_u16() })),
                            succeeded: resp.status().is_success(),
                            duration_ms: started.elapsed().as_millis() as i64,
                        },
                    )
                    .await;
                }
            }

            resp.map(Body::new)
        }
        Err(infallible) => match infallible {},
    }
}

fn extract_protocol_methods(body_bytes: &[u8]) -> (Vec<&'static str>, bool) {
    let payload = match serde_json::from_slice::<serde_json::Value>(body_bytes) {
        Ok(value) => value,
        Err(_) => return (vec!["invalid_payload"], true),
    };

    match payload {
        serde_json::Value::Object(obj) => {
            let method = classify_protocol_method(obj.get("method"));
            (vec![method], false)
        }
        serde_json::Value::Array(items) => {
            if items.is_empty() {
                return (vec!["invalid_payload"], true);
            }
            let methods = items
                .iter()
                .filter_map(|v| v.as_object())
                .map(|obj| classify_protocol_method(obj.get("method")))
                .collect::<Vec<_>>();
            if methods.is_empty() {
                (vec!["invalid_payload"], true)
            } else {
                (methods, false)
            }
        }
        _ => (vec!["invalid_payload"], true),
    }
}

fn classify_protocol_method(method_value: Option<&serde_json::Value>) -> &'static str {
    let method = method_value
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();
    match method {
        "initialize" => "initialize",
        "tools/list" => "tools_list",
        "tools/call" => "tools_call",
        _ => "other",
    }
}

fn extract_mcp_tool_calls(body_bytes: &[u8]) -> Vec<McpToolCallRecord> {
    let payload = match serde_json::from_slice::<serde_json::Value>(body_bytes) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };

    let mut records = Vec::new();
    match payload {
        serde_json::Value::Object(obj) => collect_mcp_tool_call_record(&obj, &mut records),
        serde_json::Value::Array(items) => {
            for item in items {
                if let Some(obj) = item.as_object() {
                    collect_mcp_tool_call_record(obj, &mut records);
                }
            }
        }
        _ => {}
    }
    records
}

fn collect_mcp_tool_call_record(
    obj: &serde_json::Map<String, serde_json::Value>,
    records: &mut Vec<McpToolCallRecord>,
) {
    let method = obj
        .get("method")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim();
    if method != "tools/call" {
        return;
    }

    let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));
    let mut operation = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("tools/call")
        .to_string();

    if operation == "execute" {
        let inferred = params
            .get("arguments")
            .and_then(|v| v.get("code"))
            .and_then(|v| v.as_str())
            .and_then(infer_execute_operation);
        if let Some(inferred) = inferred {
            operation = inferred;
        }
    }

    records.push(McpToolCallRecord {
        operation,
        payload: params,
    });
}

fn infer_execute_operation(code: &str) -> Option<String> {
    const OPS: [&str; 7] = [
        "oebbPlanJourney",
        "oebbPlanTour",
        "oebbResolveItineraryStops",
        "oebbLocations",
        "oebbDepartures",
        "oebbJourneys",
        "oebbTrip",
    ];

    OPS.into_iter()
        .find(|op| {
            let dot = format!("codemode.{op}");
            let bracket_single = format!("codemode['{op}']");
            let bracket_double = format!("codemode[\"{op}\"]");
            code.contains(&dot) || code.contains(&bracket_single) || code.contains(&bracket_double)
        })
        .map(ToString::to_string)
}

async fn resolve_auth_context(
    State(app): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    match app.auth_service.resolve(request.headers()) {
        Ok(context) => {
            request.extensions_mut().insert(context);
            next.run(request).await
        }
        Err(error) => (error.status_code(), Json(error.payload())).into_response(),
    }
}

async fn enforce_request_limits(
    State(app): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let guard = &app.request_guard;
    let path = request.uri().path().to_string();
    let started = Instant::now();
    let permit =
        match tokio::time::timeout(guard.queue_timeout, guard.inflight.clone().acquire_owned())
            .await
        {
            Ok(Ok(permit)) => permit,
            Ok(Err(_)) => {
                metrics::reject_request("server_unavailable");
                let response = (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(json!({
                        "ok": false,
                        "error": "server_unavailable",
                        "message": "Server is shutting down.",
                    })),
                )
                    .into_response();
                metrics::observe_http_request(&path, response.status(), started.elapsed());
                return response;
            }
            Err(_) => {
                metrics::reject_request("server_busy");
                let response = (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({
                        "ok": false,
                        "error": "server_busy",
                        "message": "Too many concurrent requests.",
                    })),
                )
                    .into_response();
                metrics::observe_http_request(&path, response.status(), started.elapsed());
                return response;
            }
        };

    {
        let mut state = guard.rate_state.lock().await;
        if state.window_started.elapsed() >= Duration::from_secs(1) {
            state.window_started = Instant::now();
            state.count = 0;
        }
        if state.count >= guard.rate_limit_per_second {
            metrics::reject_request("rate_limited");
            let response = (
                StatusCode::TOO_MANY_REQUESTS,
                Json(json!({
                    "ok": false,
                    "error": "rate_limited",
                    "message": "Rate limit exceeded for this endpoint.",
                })),
            )
                .into_response();
            metrics::observe_http_request(&path, response.status(), started.elapsed());
            return response;
        }
        state.count += 1;
    }

    metrics::mcp_inflight_inc();
    let response = next.run(request).await;
    metrics::mcp_inflight_dec();
    metrics::observe_http_request(&path, response.status(), started.elapsed());
    drop(permit);
    response
}

async fn persist_if_authenticated(state: &AppState, auth: &AuthContext, input: QueryLogInput) {
    let Some(user) = auth.user() else {
        return;
    };
    let Some(persistence) = &state.persistence else {
        return;
    };
    if let Err(error) = persistence.log_call(user, input).await {
        tracing::warn!("failed to persist query log: {error}");
    }
}

fn unauthorized_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "ok": false,
            "error": "unauthorized",
            "message": "This endpoint requires a valid Bearer JWT.",
        })),
    )
        .into_response()
}

fn persistence_unavailable_response() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "ok": false,
            "error": "persistence_unavailable",
            "message": "Persistence is disabled because DATABASE_URL is not configured.",
        })),
    )
        .into_response()
}

fn is_tool_error_payload(value: &Value) -> bool {
    value
        .get("error")
        .and_then(|v| v.as_str())
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
}

fn build_cors_layer() -> CorsLayer {
    let allowed_origins = std::env::var("MCP_ALLOWED_ORIGINS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let cors = CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::DELETE])
        .allow_headers([
            header::CONTENT_TYPE,
            header::ACCEPT,
            header::AUTHORIZATION,
            header::HeaderName::from_static("x-api-key"),
        ]);

    if allowed_origins.is_empty() {
        return cors;
    }

    if allowed_origins.iter().any(|origin| origin == "*") {
        return cors.allow_origin(Any);
    }

    let origins: Vec<HeaderValue> = allowed_origins
        .iter()
        .filter_map(|origin| HeaderValue::from_str(origin).ok())
        .collect();
    if origins.is_empty() {
        return cors;
    }
    cors.allow_origin(origins)
}

fn usize_from_env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

fn u64_from_env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn duration_from_env_ms(name: &str, default_ms: u64) -> Duration {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_millis(default_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_mcp_tool_calls_detects_execute_operation() {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "execute",
                "arguments": {
                    "code": "return await codemode.oebbPlanJourney({ from: 'Wien Hbf', to: 'Linz Hbf' });"
                }
            }
        });
        let bytes = serde_json::to_vec(&payload).expect("serialize");
        let calls = extract_mcp_tool_calls(&bytes);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].operation, "oebbPlanJourney");
    }

    #[test]
    fn extract_mcp_tool_calls_handles_batch_payload() {
        let payload = json!([
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": "search", "arguments": { "code": "return 1;" } }
            },
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": { "name": "execute", "arguments": { "code": "return await codemode.oebbJourneys({ from: 'A', to: 'B' });" } }
            }
        ]);
        let bytes = serde_json::to_vec(&payload).expect("serialize");
        let calls = extract_mcp_tool_calls(&bytes);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].operation, "search");
        assert_eq!(calls[1].operation, "oebbJourneys");
    }
}
