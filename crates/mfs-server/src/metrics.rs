use std::sync::LazyLock;

use prometheus::{HistogramOpts, HistogramVec, Opts, Registry, TextEncoder};

/// Global Prometheus registry for MemFuse metrics.
pub static REGISTRY: LazyLock<Registry> = LazyLock::new(|| {
    let registry = Registry::new();

    // HTTP request metrics
    registry
        .register(Box::new(HTTP_REQUESTS_TOTAL.clone()))
        .unwrap();
    registry
        .register(Box::new(HTTP_REQUEST_DURATION.clone()))
        .unwrap();
    registry
        .register(Box::new(HTTP_INFLIGHT_REQUESTS.clone()))
        .unwrap();

    // LLM API metrics
    registry
        .register(Box::new(LLM_CALLS_TOTAL.clone()))
        .unwrap();
    registry
        .register(Box::new(LLM_CALL_DURATION.clone()))
        .unwrap();
    registry
        .register(Box::new(LLM_TOKENS_USED.clone()))
        .unwrap();

    // System metrics
    registry
        .register(Box::new(TASK_QUEUE_DEPTH.clone()))
        .unwrap();
    registry
        .register(Box::new(CIRCUIT_BREAKER_STATE.clone()))
        .unwrap();

    registry
});

// === HTTP request metrics ===

pub static HTTP_REQUESTS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    CounterVec::new(
        Opts::new("memfuse_http_requests_total", "Total HTTP requests served"),
        &["method", "path", "status"],
    )
    .unwrap()
});

pub static HTTP_REQUEST_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    HistogramVec::new(
        HistogramOpts::new(
            "memfuse_http_request_duration_seconds",
            "HTTP request duration in seconds",
        )
        .buckets(vec![
            0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
        ]),
        &["method", "path"],
    )
    .unwrap()
});

pub static HTTP_INFLIGHT_REQUESTS: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "memfuse_http_inflight_requests",
            "Currently processing HTTP requests",
        ),
        &["method"],
    )
    .unwrap()
});

// === LLM API metrics ===

pub static LLM_CALLS_TOTAL: LazyLock<CounterVec> = LazyLock::new(|| {
    CounterVec::new(
        Opts::new("memfuse_llm_calls_total", "Total LLM API calls"),
        &["provider", "operation", "result"],
    )
    .unwrap()
});

pub static LLM_CALL_DURATION: LazyLock<HistogramVec> = LazyLock::new(|| {
    HistogramVec::new(
        HistogramOpts::new(
            "memfuse_llm_call_duration_seconds",
            "LLM API call duration in seconds",
        )
        .buckets(vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0]),
        &["provider", "operation"],
    )
    .unwrap()
});

pub static LLM_TOKENS_USED: LazyLock<CounterVec> = LazyLock::new(|| {
    CounterVec::new(
        Opts::new("memfuse_llm_tokens_used", "Total LLM tokens consumed"),
        &["provider", "operation", "type"],
    )
    .unwrap()
});

// === System metrics ===

pub static TASK_QUEUE_DEPTH: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    IntGaugeVec::new(
        Opts::new("memfuse_task_queue_depth", "Number of tasks in each state"),
        &["state"],
    )
    .unwrap()
});

pub static CIRCUIT_BREAKER_STATE: LazyLock<IntGaugeVec> = LazyLock::new(|| {
    IntGaugeVec::new(
        Opts::new(
            "memfuse_circuit_breaker_state",
            "Circuit breaker state (0=closed, 1=open, 2=half_open)",
        ),
        &["provider"],
    )
    .unwrap()
});

/// Render all registered metrics in Prometheus exposition format.
pub fn render_metrics() -> String {
    TextEncoder::new()
        .encode_to_string(&REGISTRY.gather())
        .unwrap_or_default()
}

/// Record an HTTP request with its method, path, status code, and duration.
pub fn record_http_request(method: &str, path: &str, status: u16, duration_secs: f64) {
    HTTP_REQUESTS_TOTAL
        .with_label_values(&[method, path, &status.to_string()])
        .inc();
    HTTP_REQUEST_DURATION
        .with_label_values(&[method, path])
        .observe(duration_secs);
}

/// Increment inflight gauge for an HTTP request method.
pub fn inflight_request_enter(method: &str) {
    HTTP_INFLIGHT_REQUESTS.with_label_values(&[method]).inc();
}

/// Decrement inflight gauge for an HTTP request method.
pub fn inflight_request_exit(method: &str) {
    HTTP_INFLIGHT_REQUESTS.with_label_values(&[method]).dec();
}

/// Record a LLM API call result.
pub fn record_llm_call(provider: &str, operation: &str, result: &str) {
    LLM_CALLS_TOTAL
        .with_label_values(&[provider, operation, result])
        .inc();
}

/// Record LLM API call duration.
pub fn record_llm_call_duration(provider: &str, operation: &str, duration_secs: f64) {
    LLM_CALL_DURATION
        .with_label_values(&[provider, operation])
        .observe(duration_secs);
}

/// Update the task queue depth gauge for a given state.
pub fn update_task_queue_depth(state: &str, count: i64) {
    TASK_QUEUE_DEPTH.with_label_values(&[state]).set(count);
}

use prometheus::CounterVec;
use prometheus::IntGaugeVec;
