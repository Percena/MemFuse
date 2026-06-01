use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Circuit breaker states: CLOSED -> OPEN -> HALF_OPEN -> CLOSED
const CB_CLOSED: u8 = 0;
const CB_OPEN: u8 = 1;
const CB_HALF_OPEN: u8 = 2;

/// A thread-safe circuit breaker for LLM API calls.
///
/// Tracks consecutive failures and transitions between states:
/// - CLOSED: Normal operation, all requests pass through
/// - OPEN: Too many failures, all requests short-circuit to fallback
/// - HALF_OPEN: Testing if the downstream service has recovered;
///   a single failure in this state immediately re-opens the circuit.
///
/// Thread-safe via atomic operations, suitable for use from `std::thread::scope` workers.
pub struct CircuitBreaker {
    state: AtomicU8,
    failure_count: AtomicU64,
    failure_threshold: u64,
    reset_timeout: Duration,
    last_failure_at: AtomicU64, // stores Instant as nanos since program start
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u64, reset_timeout: Duration) -> Self {
        Self {
            state: AtomicU8::new(CB_CLOSED),
            failure_count: AtomicU64::new(0),
            failure_threshold,
            reset_timeout,
            last_failure_at: AtomicU64::new(0),
        }
    }

    /// Check if a request is allowed to proceed.
    ///
    /// Returns `true` in CLOSED state, `false` in OPEN state (unless reset_timeout has elapsed),
    /// and `true` in HALF_OPEN state (allowing a probe request).
    ///
    /// Uses Acquire/Release ordering to ensure failure_count and last_failure_at
    /// are visible after state transitions.
    pub fn allow_request(&self) -> bool {
        let state = self.state.load(Ordering::Acquire);
        match state {
            CB_CLOSED => true,
            CB_OPEN => {
                // Check if reset_timeout has elapsed
                let last_failure_nanos = self.last_failure_at.load(Ordering::Acquire);
                let elapsed = Instant::now().saturating_duration_since(program_start());
                let since_last_failure =
                    elapsed.saturating_sub(Duration::from_nanos(last_failure_nanos));
                if since_last_failure >= self.reset_timeout {
                    // Transition to HALF_OPEN — use compare_exchange to prevent thundering herd
                    // If multiple threads race, only one will successfully transition
                    match self.state.compare_exchange(
                        CB_OPEN,
                        CB_HALF_OPEN,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    ) {
                        Ok(_) => true,             // We won the race, allow our probe request
                        Err(CB_HALF_OPEN) => true, // Another thread already transitioned, allow through
                        Err(CB_OPEN) => false,     // Another thread reset back to OPEN, deny
                        Err(_) => true,            // Unexpected state, be permissive
                    }
                } else {
                    false
                }
            }
            CB_HALF_OPEN => true,
            _ => true,
        }
    }

    /// Record a successful response. Resets failure count and transitions to CLOSED.
    pub fn record_success(&self) {
        self.failure_count.store(0, Ordering::Release);
        self.state.store(CB_CLOSED, Ordering::Release);
    }

    /// Record a failure. Increments failure count and may transition to OPEN.
    ///
    /// If `permanent` is true, immediately transitions to OPEN regardless of threshold.
    /// In HALF_OPEN state, any failure immediately re-opens the circuit.
    pub fn record_failure(&self, permanent: bool) {
        let current_state = self.state.load(Ordering::Acquire);

        // In HALF_OPEN state, a single failure immediately re-opens the circuit
        if current_state == CB_HALF_OPEN {
            self.state.store(CB_OPEN, Ordering::Release);
            let elapsed = Instant::now().saturating_duration_since(program_start());
            self.last_failure_at
                .store(elapsed.as_nanos() as u64, Ordering::Release);
            return;
        }

        let new_count = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;
        let elapsed = Instant::now().saturating_duration_since(program_start());
        self.last_failure_at
            .store(elapsed.as_nanos() as u64, Ordering::Release);

        if permanent || new_count >= self.failure_threshold {
            self.state.store(CB_OPEN, Ordering::Release);
        }
    }

    /// Get current state as a human-readable string.
    pub fn state_str(&self) -> &'static str {
        match self.state.load(Ordering::Relaxed) {
            CB_CLOSED => "closed",
            CB_OPEN => "open",
            CB_HALF_OPEN => "half_open",
            _ => "unknown",
        }
    }
}

impl std::fmt::Debug for CircuitBreaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CircuitBreaker")
            .field("state", &self.state_str())
            .field("failure_count", &self.failure_count.load(Ordering::Relaxed))
            .field("failure_threshold", &self.failure_threshold)
            .field("reset_timeout", &self.reset_timeout)
            .finish()
    }
}

fn program_start() -> Instant {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now)
}

/// Classify an HTTP error as permanent (don't retry) or transient (retry with backoff).
///
/// Based on HTTP status code:
/// - 401/403: permanent, not retryable (auth/permission issues)
/// - 400: permanent, not retryable (bad request)
/// - 408: transient, retryable (request timeout from server)
/// - 429: transient, retryable (rate limit)
/// - 500/502/503/504: transient, retryable (server errors)
/// - Other 4xx: permanent; other 5xx: transient
pub enum ApiErrorClass {
    Permanent,
    Transient,
}

pub fn classify_api_error(status: u16) -> ApiErrorClass {
    match status {
        400 | 401 | 403 => ApiErrorClass::Permanent,
        408 => ApiErrorClass::Transient, // Request timeout from server
        429 => ApiErrorClass::Transient,
        500 | 502 | 503 | 504 => ApiErrorClass::Transient,
        _ => {
            if status >= 400 && status < 500 {
                ApiErrorClass::Permanent
            } else {
                ApiErrorClass::Transient
            }
        }
    }
}

/// Execute an async operation with exponential backoff retry.
///
/// `max_attempts` controls the total number of attempts (first try + retries).
/// For example, `max_attempts=4` means 1 initial attempt + up to 3 retries.
///
/// Only retries on transient or network errors. Permanent errors immediately return None.
/// Circuit breaker is checked before each attempt; if open, returns None immediately.
///
/// Returns the successful result as `Some(T)`, or `None` after all retries exhausted
/// or circuit breaker short-circuited.
pub async fn retry_with_backoff<F, Fut, T>(
    max_attempts: u32,
    base_delay: Duration,
    max_delay: Duration,
    circuit_breaker: &CircuitBreaker,
    mut operation: F,
) -> Option<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, RetryableError>>,
{
    // Check circuit breaker first
    if !circuit_breaker.allow_request() {
        return None;
    }

    let mut last_delay = base_delay;

    for attempt in 0..max_attempts {
        match operation().await {
            Ok(result) => {
                circuit_breaker.record_success();
                return Some(result);
            }
            Err(RetryableError::Permanent) => {
                circuit_breaker.record_failure(true);
                return None;
            }
            Err(err) => {
                circuit_breaker.record_failure(false);

                if attempt + 1 < max_attempts {
                    // Exponential backoff with jitter
                    let jitter_factor = rand_backoff_jitter();
                    let jitter = Duration::from_millis(
                        (jitter_factor * last_delay.as_millis() as f64) as u64,
                    );
                    let sleep_duration = (last_delay + jitter).min(max_delay);
                    tokio::time::sleep(sleep_duration).await;

                    last_delay = (last_delay * 2).min(max_delay);
                }
                // Continue to next attempt or exhaust retries
                let _ = err; // suppress unused warning
            }
        }
    }

    None
}

/// Error type for retryable operations.
pub enum RetryableError {
    /// Permanent error — do not retry (e.g., 401, 403, 400)
    Permanent,
    /// Transient error — retry with backoff (e.g., 429, 503)
    Transient { status: u16 },
    /// Network-level error — retry with backoff (e.g., connection refused, timeout)
    Network,
}

/// Simple jitter factor (0.0 to 1.0) using thread-local state.
/// Not cryptographically random — just enough to prevent thundering herd.
fn rand_backoff_jitter() -> f64 {
    use std::cell::Cell;
    thread_local! {
        static SEED: Cell<u64> = Cell::new(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
        );
    }
    SEED.with(|seed| {
        let mut s = seed.get();
        // xorshift64
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        seed.set(s);
        // Map to [0.0, 1.0)
        (s as f64) / (u64::MAX as f64)
    })
}

/// Configuration for resilience parameters, loaded from environment variables.
#[derive(Debug, Clone)]
pub struct ResilienceConfig {
    /// Total attempts per operation (initial + retries). Default: 4 (= 1 try + 3 retries)
    pub max_attempts: u32,
    /// Base delay for exponential backoff. Default: 500ms
    pub base_delay: Duration,
    /// Maximum delay cap for backoff. Default: 8s
    pub max_delay: Duration,
    /// Consecutive failures needed to trip the circuit breaker. Default: 5
    pub cb_failure_threshold: u64,
    /// Time to wait before transitioning from OPEN to HALF_OPEN. Default: 5min
    pub cb_reset_timeout: Duration,
}

impl ResilienceConfig {
    pub fn from_env() -> Self {
        Self {
            max_attempts: env_parse("MEMFUSE_MAX_RETRIES", 3) as u32 + 1, // +1 for initial attempt
            base_delay: Duration::from_millis(env_parse("MEMFUSE_RETRY_BASE_DELAY_MS", 500)),
            max_delay: Duration::from_millis(env_parse("MEMFUSE_RETRY_MAX_DELAY_MS", 8000)),
            cb_failure_threshold: env_parse("MEMFUSE_CB_FAILURE_THRESHOLD", 5),
            cb_reset_timeout: Duration::from_millis(env_parse(
                "MEMFUSE_CB_RESET_TIMEOUT_MS",
                300_000,
            )),
        }
    }

    pub fn from_env_for_read() -> Self {
        // Read-path circuit-breaker thresholds fall back to the write-path
        // defaults when the read-specific env key is absent.
        let fallback_cb_threshold = env_parse("MEMFUSE_CB_FAILURE_THRESHOLD", 5);
        let fallback_cb_reset = env_parse("MEMFUSE_CB_RESET_TIMEOUT_MS", 300_000);
        Self {
            max_attempts: env_parse("MEMFUSE_READ_MAX_RETRIES", 0) as u32 + 1,
            base_delay: Duration::from_millis(env_parse("MEMFUSE_READ_RETRY_BASE_DELAY_MS", 100)),
            max_delay: Duration::from_millis(env_parse("MEMFUSE_READ_RETRY_MAX_DELAY_MS", 500)),
            cb_failure_threshold: env_parse(
                "MEMFUSE_READ_CB_FAILURE_THRESHOLD",
                fallback_cb_threshold,
            ),
            cb_reset_timeout: Duration::from_millis(env_parse(
                "MEMFUSE_READ_CB_RESET_TIMEOUT_MS",
                fallback_cb_reset,
            )),
        }
    }
}

pub fn env_parse(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circuit_breaker_transitions_closed_to_open() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(300));
        assert!(cb.allow_request());
        assert_eq!(cb.state_str(), "closed");

        cb.record_failure(false);
        cb.record_failure(false);
        cb.record_failure(false); // threshold reached
        assert_eq!(cb.state_str(), "open");
        assert!(!cb.allow_request());
    }

    #[test]
    fn circuit_breaker_permanent_failure_immediately_opens() {
        let cb = CircuitBreaker::new(100, Duration::from_secs(300));
        assert!(cb.allow_request());
        cb.record_failure(true);
        assert_eq!(cb.state_str(), "open");
        assert!(!cb.allow_request());
    }

    #[test]
    fn circuit_breaker_success_resets_to_closed() {
        let cb = CircuitBreaker::new(2, Duration::from_secs(300));
        cb.record_failure(false);
        cb.record_failure(false);
        assert_eq!(cb.state_str(), "open");
        // Simulate reset timeout elapsed by transitioning to half_open
        cb.state.store(CB_HALF_OPEN, Ordering::Relaxed);
        assert!(cb.allow_request());
        cb.record_success();
        assert_eq!(cb.state_str(), "closed");
    }

    #[test]
    fn circuit_breaker_half_open_failure_immediately_reopens() {
        let cb = CircuitBreaker::new(5, Duration::from_secs(300));
        // Force into HALF_OPEN state
        cb.state.store(CB_HALF_OPEN, Ordering::Release);
        assert_eq!(cb.state_str(), "half_open");
        assert!(cb.allow_request());

        // A single failure in HALF_OPEN should immediately re-open
        cb.record_failure(false);
        assert_eq!(cb.state_str(), "open");
        assert!(!cb.allow_request());
    }

    #[test]
    fn classify_429_as_transient() {
        assert!(matches!(classify_api_error(429), ApiErrorClass::Transient));
    }

    #[test]
    fn classify_401_as_permanent() {
        assert!(matches!(classify_api_error(401), ApiErrorClass::Permanent));
    }

    #[test]
    fn classify_503_as_transient() {
        assert!(matches!(classify_api_error(503), ApiErrorClass::Transient));
    }

    #[test]
    fn classify_408_as_transient() {
        assert!(matches!(classify_api_error(408), ApiErrorClass::Transient));
    }

    #[tokio::test]
    async fn retry_with_backoff_succeeds_on_first_try() {
        let cb = CircuitBreaker::new(5, Duration::from_secs(300));
        let result = retry_with_backoff(
            4, // max_attempts=4 means 1 initial + 3 retries
            Duration::from_millis(100),
            Duration::from_secs(8),
            &cb,
            || async { Ok(42) },
        )
        .await;
        assert_eq!(result, Some(42));
        assert_eq!(cb.state_str(), "closed");
    }

    #[tokio::test]
    async fn retry_with_backoff_retries_transient_and_succeeds() {
        let cb = CircuitBreaker::new(5, Duration::from_secs(300));
        let attempts = std::sync::atomic::AtomicU32::new(0);
        let result = retry_with_backoff(
            4,
            Duration::from_millis(10),
            Duration::from_millis(50),
            &cb,
            || async {
                let a = attempts.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if a < 3 {
                    Err(RetryableError::Transient { status: 503 })
                } else {
                    Ok("success")
                }
            },
        )
        .await;
        assert_eq!(result, Some("success"));
        assert_eq!(cb.state_str(), "closed");
    }

    #[tokio::test]
    async fn retry_with_backoff_permanent_error_returns_none() {
        let cb = CircuitBreaker::new(5, Duration::from_secs(300));
        let result: Option<&str> = retry_with_backoff(
            4,
            Duration::from_millis(10),
            Duration::from_millis(50),
            &cb,
            || async { Err(RetryableError::Permanent) },
        )
        .await;
        assert_eq!(result, None);
        assert_eq!(cb.state_str(), "open");
    }

    #[tokio::test]
    async fn circuit_breaker_open_short_circuits() {
        let cb = CircuitBreaker::new(1, Duration::from_secs(300));
        cb.record_failure(false); // threshold=1, immediately open
        let result = retry_with_backoff(
            4,
            Duration::from_millis(10),
            Duration::from_millis(50),
            &cb,
            || async { Ok("would succeed but cb is open") },
        )
        .await;
        assert_eq!(result, None); // short-circuited without calling operation
    }

    #[test]
    fn resilience_config_defaults() {
        let config = ResilienceConfig::from_env();
        // max_attempts = MEMFUSE_MAX_RETRIES + 1 (default 3+1=4)
        assert_eq!(config.max_attempts, 4);
        assert_eq!(config.base_delay, Duration::from_millis(500));
        assert_eq!(config.max_delay, Duration::from_millis(8000));
        assert_eq!(config.cb_failure_threshold, 5);
        assert_eq!(config.cb_reset_timeout, Duration::from_millis(300_000));
    }
}
