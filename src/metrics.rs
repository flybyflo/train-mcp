use std::time::Duration;

use axum::http::StatusCode;
use once_cell::sync::Lazy;
use prometheus::{
    Encoder, HistogramOpts, HistogramVec, IntCounterVec, IntGauge, Opts, Registry, TextEncoder,
};

pub struct Metrics {
    pub http_requests_total: IntCounterVec,
    pub http_request_duration_seconds: HistogramVec,
    pub http_rejected_total: IntCounterVec,
    pub mcp_inflight_requests: IntGauge,
    pub tool_calls_total: IntCounterVec,
    pub tool_call_duration_seconds: HistogramVec,
    pub tool_result_total: IntCounterVec,
    pub protocol_requests_total: IntCounterVec,
    pub protocol_invalid_payload_total: IntCounterVec,
    pub oebb_upstream_requests_total: IntCounterVec,
    pub oebb_upstream_request_duration_seconds: HistogramVec,
    pub oebb_upstream_retries_total: IntCounterVec,
    pub oebb_upstream_timeouts_total: IntCounterVec,
    pub oebb_cache_events_total: IntCounterVec,
}

pub static REGISTRY: Lazy<Registry> = Lazy::new(Registry::new);

pub static METRICS: Lazy<Metrics> = Lazy::new(|| {
    let http_requests_total = IntCounterVec::new(
        Opts::new("train_mcp_http_requests_total", "Total HTTP requests for train-mcp."),
        &["route", "status_class"],
    )
    .expect("create http_requests_total");
    let http_request_duration_seconds = HistogramVec::new(
        HistogramOpts::new(
            "train_mcp_http_request_duration_seconds",
            "Request latency in seconds for train-mcp routes.",
        ),
        &["route", "status_class"],
    )
    .expect("create http_request_duration_seconds");
    let http_rejected_total = IntCounterVec::new(
        Opts::new(
            "train_mcp_http_rejected_total",
            "Rejected MCP HTTP requests by reason.",
        ),
        &["reason"],
    )
    .expect("create http_rejected_total");
    let mcp_inflight_requests =
        IntGauge::new("train_mcp_inflight_requests", "Current inflight MCP requests.")
            .expect("create mcp_inflight_requests");
    let tool_calls_total = IntCounterVec::new(
        Opts::new("train_mcp_tool_calls_total", "Total MCP tool calls."),
        &["tool", "outcome"],
    )
    .expect("create tool_calls_total");
    let tool_call_duration_seconds = HistogramVec::new(
        HistogramOpts::new(
            "train_mcp_tool_call_duration_seconds",
            "MCP tool call latency in seconds.",
        ),
        &["tool", "outcome"],
    )
    .expect("create tool_call_duration_seconds");
    let tool_result_total = IntCounterVec::new(
        Opts::new(
            "train_mcp_tool_result_total",
            "Business outcomes of MCP tool execution results.",
        ),
        &["tool", "outcome", "reason"],
    )
    .expect("create tool_result_total");
    let protocol_requests_total = IntCounterVec::new(
        Opts::new(
            "train_mcp_protocol_requests_total",
            "MCP protocol requests grouped by method and outcome.",
        ),
        &["method", "outcome"],
    )
    .expect("create protocol_requests_total");
    let protocol_invalid_payload_total = IntCounterVec::new(
        Opts::new(
            "train_mcp_protocol_invalid_payload_total",
            "Invalid MCP payloads by reason.",
        ),
        &["reason"],
    )
    .expect("create protocol_invalid_payload_total");
    let oebb_upstream_requests_total = IntCounterVec::new(
        Opts::new(
            "train_mcp_oebb_upstream_requests_total",
            "Total ÖBB upstream HTTP attempts by endpoint and status.",
        ),
        &["endpoint", "status"],
    )
    .expect("create oebb_upstream_requests_total");
    let oebb_upstream_request_duration_seconds = HistogramVec::new(
        HistogramOpts::new(
            "train_mcp_oebb_upstream_request_duration_seconds",
            "Latency for ÖBB upstream HTTP attempts.",
        ),
        &["endpoint", "outcome"],
    )
    .expect("create oebb_upstream_request_duration_seconds");
    let oebb_upstream_retries_total = IntCounterVec::new(
        Opts::new(
            "train_mcp_oebb_upstream_retries_total",
            "Number of ÖBB upstream retries by endpoint and reason.",
        ),
        &["endpoint", "reason"],
    )
    .expect("create oebb_upstream_retries_total");
    let oebb_upstream_timeouts_total = IntCounterVec::new(
        Opts::new(
            "train_mcp_oebb_upstream_timeouts_total",
            "Number of ÖBB upstream timeout errors by endpoint.",
        ),
        &["endpoint"],
    )
    .expect("create oebb_upstream_timeouts_total");
    let oebb_cache_events_total = IntCounterVec::new(
        Opts::new(
            "train_mcp_oebb_cache_events_total",
            "ÖBB cache events for hit/miss and in-flight wait behavior.",
        ),
        &["event"],
    )
    .expect("create oebb_cache_events_total");

    REGISTRY
        .register(Box::new(http_requests_total.clone()))
        .expect("register http_requests_total");
    REGISTRY
        .register(Box::new(http_request_duration_seconds.clone()))
        .expect("register http_request_duration_seconds");
    REGISTRY
        .register(Box::new(http_rejected_total.clone()))
        .expect("register http_rejected_total");
    REGISTRY
        .register(Box::new(mcp_inflight_requests.clone()))
        .expect("register mcp_inflight_requests");
    REGISTRY
        .register(Box::new(tool_calls_total.clone()))
        .expect("register tool_calls_total");
    REGISTRY
        .register(Box::new(tool_call_duration_seconds.clone()))
        .expect("register tool_call_duration_seconds");
    REGISTRY
        .register(Box::new(tool_result_total.clone()))
        .expect("register tool_result_total");
    REGISTRY
        .register(Box::new(protocol_requests_total.clone()))
        .expect("register protocol_requests_total");
    REGISTRY
        .register(Box::new(protocol_invalid_payload_total.clone()))
        .expect("register protocol_invalid_payload_total");
    REGISTRY
        .register(Box::new(oebb_upstream_requests_total.clone()))
        .expect("register oebb_upstream_requests_total");
    REGISTRY
        .register(Box::new(oebb_upstream_request_duration_seconds.clone()))
        .expect("register oebb_upstream_request_duration_seconds");
    REGISTRY
        .register(Box::new(oebb_upstream_retries_total.clone()))
        .expect("register oebb_upstream_retries_total");
    REGISTRY
        .register(Box::new(oebb_upstream_timeouts_total.clone()))
        .expect("register oebb_upstream_timeouts_total");
    REGISTRY
        .register(Box::new(oebb_cache_events_total.clone()))
        .expect("register oebb_cache_events_total");

    Metrics {
        http_requests_total,
        http_request_duration_seconds,
        http_rejected_total,
        mcp_inflight_requests,
        tool_calls_total,
        tool_call_duration_seconds,
        tool_result_total,
        protocol_requests_total,
        protocol_invalid_payload_total,
        oebb_upstream_requests_total,
        oebb_upstream_request_duration_seconds,
        oebb_upstream_retries_total,
        oebb_upstream_timeouts_total,
        oebb_cache_events_total,
    }
});

pub fn observe_http_request(route: &str, status: StatusCode, duration: Duration) {
    let status_class = match status.as_u16() {
        100..=199 => "1xx",
        200..=299 => "2xx",
        300..=399 => "3xx",
        400..=499 => "4xx",
        _ => "5xx",
    };
    METRICS
        .http_requests_total
        .with_label_values(&[route, status_class])
        .inc();
    METRICS
        .http_request_duration_seconds
        .with_label_values(&[route, status_class])
        .observe(duration.as_secs_f64());
}

pub fn reject_request(reason: &str) {
    METRICS
        .http_rejected_total
        .with_label_values(&[reason])
        .inc();
}

pub fn mcp_inflight_inc() {
    METRICS.mcp_inflight_requests.inc();
}

pub fn mcp_inflight_dec() {
    METRICS.mcp_inflight_requests.dec();
}

pub fn observe_tool_call(tool: &str, outcome: &str, duration: Duration) {
    METRICS
        .tool_calls_total
        .with_label_values(&[tool, outcome])
        .inc();
    METRICS
        .tool_call_duration_seconds
        .with_label_values(&[tool, outcome])
        .observe(duration.as_secs_f64());
}

pub fn observe_tool_result(tool: &str, outcome: &str, reason: &str) {
    METRICS
        .tool_result_total
        .with_label_values(&[tool, outcome, reason])
        .inc();
}

pub fn observe_protocol_request(method: &str, outcome: &str) {
    METRICS
        .protocol_requests_total
        .with_label_values(&[method, outcome])
        .inc();
}

pub fn observe_protocol_invalid_payload(reason: &str) {
    METRICS
        .protocol_invalid_payload_total
        .with_label_values(&[reason])
        .inc();
}

pub fn observe_oebb_upstream_attempt(endpoint: &str, status: &str, outcome: &str, duration: Duration) {
    METRICS
        .oebb_upstream_requests_total
        .with_label_values(&[endpoint, status])
        .inc();
    METRICS
        .oebb_upstream_request_duration_seconds
        .with_label_values(&[endpoint, outcome])
        .observe(duration.as_secs_f64());
}

pub fn observe_oebb_retry(endpoint: &str, reason: &str) {
    METRICS
        .oebb_upstream_retries_total
        .with_label_values(&[endpoint, reason])
        .inc();
}

pub fn observe_oebb_timeout(endpoint: &str) {
    METRICS
        .oebb_upstream_timeouts_total
        .with_label_values(&[endpoint])
        .inc();
}

pub fn observe_oebb_cache_event(event: &str) {
    METRICS
        .oebb_cache_events_total
        .with_label_values(&[event])
        .inc();
}

pub fn gather_text() -> Result<String, String> {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|e| format!("failed to encode metrics: {e}"))?;
    String::from_utf8(buffer).map_err(|e| format!("invalid UTF-8 in metrics output: {e}"))
}
