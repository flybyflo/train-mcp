use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{Request, State},
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
use serde_json::json;
use tokio::sync::{Mutex, Semaphore};
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::EnvFilter;

use train_mcp::catalog::create_catalog_payload;
use train_mcp::metrics;
use train_mcp::server::TrainMcp;

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

async fn healthz() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "service": "train-mcp" }))
}

async fn catalog_route() -> Json<serde_json::Value> {
    Json(create_catalog_payload())
}

async fn metrics_route() -> Response {
    match metrics::gather_text() {
        Ok(payload) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
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

    // The service factory creates a new TrainMcp for each session.
    let oebb_url = oebb_base_url.clone();
    let mcp_service = StreamableHttpService::new(
        move || Ok(TrainMcp::new(oebb_url.clone())),
        session_manager,
        config,
    );

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/catalog", get(catalog_route))
        .route("/metrics", get(metrics_route))
        .route(
            "/mcp",
            post(mcp_handler).route_layer(middleware::from_fn_with_state(
                request_guard.clone(),
                enforce_request_limits,
            )),
        )
        .with_state(mcp_service)
        .layer(axum::extract::DefaultBodyLimit::max(max_body_bytes))
        .layer(cors_layer);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("train-mcp listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn mcp_handler(
    axum::extract::State(mut service): axum::extract::State<StreamableHttpService<TrainMcp>>,
    request: axum::extract::Request,
) -> impl axum::response::IntoResponse {
    use tower_service::Service;
    let response = service.call(request).await;
    match response {
        Ok(resp) => resp,
        Err(infallible) => match infallible {},
    }
}

async fn enforce_request_limits(
    State(guard): State<RequestGuard>,
    request: Request,
    next: Next,
) -> Response {
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
                metrics::observe_http_request("/mcp", response.status(), started.elapsed());
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
                metrics::observe_http_request("/mcp", response.status(), started.elapsed());
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
                    "message": "Rate limit exceeded for MCP endpoint.",
                })),
            )
                .into_response();
            metrics::observe_http_request("/mcp", response.status(), started.elapsed());
            return response;
        }
        state.count += 1;
    }

    metrics::mcp_inflight_inc();
    let response = next.run(request).await;
    metrics::mcp_inflight_dec();
    metrics::observe_http_request("/mcp", response.status(), started.elapsed());
    drop(permit);
    response
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
