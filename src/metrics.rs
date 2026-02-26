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

    Metrics {
        http_requests_total,
        http_request_duration_seconds,
        http_rejected_total,
        mcp_inflight_requests,
        tool_calls_total,
        tool_call_duration_seconds,
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

pub fn gather_text() -> Result<String, String> {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|e| format!("failed to encode metrics: {e}"))?;
    String::from_utf8(buffer).map_err(|e| format!("invalid UTF-8 in metrics output: {e}"))
}
