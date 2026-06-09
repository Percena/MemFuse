#![allow(unsafe_code)]
//! Test utilities shared across mfs crates.
//!
//! The primary utility is `env_isolated()`, an RAII guard that removes
//! non-deterministic environment variables (API keys, model names, provider
//! overrides) for the duration of a test and restores them on drop.
//! This ensures tests that rely on deterministic fallback paths produce
//! consistent results even when the shell environment contains real API keys.

/// Environment variable keys that steer the runtime away from deterministic
/// fallback.  When these keys are present, `chat_provider_from_env()`,
/// `SemanticPipelineConfig::from_env()`, and similar functions select
/// non-deterministic providers (OpenAI, Jina), which breaks tests that
/// assert on deterministic output.
pub const NON_DETERMINISTIC_ENV_KEYS: &[&str] = &[
    "MEMFUSE_OPENAI_API_KEY",
    "OPENAI_API_KEY",
    "MEMFUSE_JINA_API_KEY",
    "JINA_API_KEY",
    "MEMFUSE_OPENAI_API_BASE",
    "OPENAI_BASE_URL",
    "MEMFUSE_SUMMARY_MODEL",
    "OPENAI_COMPATIBLE_MODEL",
    "MEMFUSE_CHAT_MODEL",
    "MEMFUSE_CHAT_PROVIDER",
    "MEMFUSE_READ_LLM_ENABLED",
    "MEMFUSE_READ_MAX_RETRIES",
    "MEMFUSE_READ_RETRY_BASE_DELAY_MS",
    "MEMFUSE_READ_RETRY_MAX_DELAY_MS",
    "MEMFUSE_READ_TIMEOUT_MS",
    "MEMFUSE_READ_CONNECT_TIMEOUT_MS",
    "MEMFUSE_READ_CB_FAILURE_THRESHOLD",
    "MEMFUSE_READ_CB_RESET_TIMEOUT_MS",
    "MEMFUSE_READ_EMBED_TIMEOUT_MS",
    "MEMFUSE_EMBEDDING_MODEL",
    "OPENAI_EMBEDDING_MODEL",
    "MEMFUSE_SUMMARY_PROVIDER",
    "MEMFUSE_EMBEDDING_PROVIDER",
    "MEMFUSE_RERANK_PROVIDER",
    // Auth env vars — must be isolated so auth-mode tests don't pollute
    // concurrent tests that expect Dev mode.
    "MEMFUSE_AUTH_MODE",
    "MEMFUSE_API_KEY",
    "MEMFUSE_RATE_LIMIT_ENABLED",
    "MEMFUSE_RATE_LIMIT_REQUESTS",
    "MEMFUSE_RATE_LIMIT_WINDOW_SECS",
    "MEMFUSE_MAX_BODY_SIZE_MB",
];

/// A global mutex that ensures env-isolation tests run serially even when
/// `cargo test` uses multiple threads.  The lock is held for the entire
/// test via the `EnvGuard`'s `_lock` field.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII guard that removes non-deterministic env keys on creation and
/// restores their original values on drop.
///
/// Call `env_isolated()` at the start of any test that needs deterministic
/// provider behaviour.  The guard automatically restores the original env
/// state when the test completes (even on panic), and a global mutex
/// ensures that concurrent env-isolation tests within the same binary run
/// serially to avoid cross-test pollution.
///
/// # Example
///
/// ```rust,no_run
/// // In a test function (typically with #[tokio::test]):
/// let _guard = mfs_test_util::env_isolated();
/// // ... test code that relies on deterministic providers ...
/// ```
pub struct EnvGuard {
    saved: Vec<(String, Option<OsString>)>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl EnvGuard {
    /// Set an environment variable while the global test env lock is held.
    pub fn set_var<K, V>(&self, key: K, value: V)
    where
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        // SAFETY: EnvGuard holds ENV_LOCK for its full lifetime, serializing
        // test environment mutations within this process.
        unsafe {
            std::env::set_var(key, value);
        }
    }

    /// Remove an environment variable while the global test env lock is held.
    pub fn remove_var<K>(&self, key: K)
    where
        K: AsRef<OsStr>,
    {
        // SAFETY: EnvGuard holds ENV_LOCK for its full lifetime, serializing
        // test environment mutations within this process.
        unsafe {
            std::env::remove_var(key);
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, val) in &self.saved {
            // SAFETY: ENV_LOCK is still held (we own the guard), so this is
            // safe even under multi-threaded test execution.
            if let Some(v) = val {
                self.set_var(key, v);
            } else {
                self.remove_var(key);
            }
        }
    }
}

/// Remove all non-deterministic env keys and return an `EnvGuard` that
/// restores them when dropped.
///
/// See [`EnvGuard`] for usage details.
pub fn env_isolated() -> EnvGuard {
    let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved = NON_DETERMINISTIC_ENV_KEYS
        .iter()
        .map(|key| {
            // SAFETY: ENV_LOCK is held, guaranteeing serial access.
            let val = std::env::var_os(key);
            unsafe {
                std::env::remove_var(key);
            }
            (key.to_string(), val)
        })
        .collect::<Vec<_>>();
    EnvGuard { saved, _lock: lock }
}

/// Set or remove the supplied environment variables under the global test env
/// lock and restore their original values when the guard is dropped.
pub fn env_with_vars(vars: &[(&str, Option<&str>)]) -> EnvGuard {
    let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let saved = vars
        .iter()
        .map(|(key, _)| ((*key).to_owned(), std::env::var_os(key)))
        .collect::<Vec<_>>();
    let guard = EnvGuard { saved, _lock: lock };

    for (key, value) in vars {
        match value {
            Some(value) => guard.set_var(key, value),
            None => guard.remove_var(key),
        }
    }

    guard
}
use std::ffi::{OsStr, OsString};
