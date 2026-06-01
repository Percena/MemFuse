use std::fs;
use std::io;
use std::path::Path;
use std::process;
use tracing::warn;

/// Guard that holds the PID lock and cleans it up on Drop.
pub struct PidLockGuard {
    lock_path: std::path::PathBuf,
}

impl PidLockGuard {
    /// Acquire a PID-based data directory lock.
    ///
    /// If the lock file exists and the recorded PID is still alive, returns an error.
    /// If the lock file exists but the PID is stale (process no longer running), the stale lock
    /// is cleaned up and a new lock is acquired.
    pub fn acquire(workspace_root: &Path) -> Result<Self, PidLockError> {
        let lock_path = workspace_root.join(".memfuse.pid");
        let current_pid = process::id();

        // Ensure _system directory exists for first-time setup
        let system_dir = workspace_root.join("_system");
        if !system_dir.exists() {
            fs::create_dir_all(&system_dir).map_err(|e| PidLockError::Io {
                action: "create _system dir",
                path: system_dir.clone(),
                source: e,
            })?;
        }

        if lock_path.exists() {
            let content = fs::read_to_string(&lock_path).map_err(|e| PidLockError::Io {
                action: "read PID lock",
                path: lock_path.clone(),
                source: e,
            })?;

            let existing_pid: u32 = match content.trim().parse::<u32>() {
                Ok(pid) if pid != 0 => pid,
                _ => {
                    // Lock file contains invalid or zero PID — likely corrupted.
                    // Treat as stale to allow recovery, but warn.
                    warn!(
                        path = %lock_path.display(),
                        "PID lock file contains invalid content, treating as stale"
                    );
                    fs::remove_file(&lock_path).map_err(|e| PidLockError::Io {
                        action: "remove corrupted PID lock",
                        path: lock_path.clone(),
                        source: e,
                    })?;
                    return Self::create_lock_file(&lock_path, current_pid);
                }
            };

            if is_pid_alive(existing_pid) {
                return Err(PidLockError::AlreadyLocked {
                    pid: existing_pid,
                    lock_path: lock_path.to_string_lossy().to_string(),
                });
            }

            // Stale lock — clean it up
            fs::remove_file(&lock_path).map_err(|e| PidLockError::Io {
                action: "remove stale PID lock",
                path: lock_path.clone(),
                source: e,
            })?;
        }

        Self::create_lock_file(&lock_path, current_pid)
    }

    fn create_lock_file(lock_path: &Path, current_pid: u32) -> Result<Self, PidLockError> {
        let lock_path_owned = lock_path.to_path_buf();

        // Write our PID with O_EXCL semantics via create_new
        let mut file = fs::File::create_new(lock_path).map_err(|e| {
            if e.kind() == io::ErrorKind::AlreadyExists {
                // Race condition: another process grabbed the lock between our check and write
                let content = fs::read_to_string(lock_path).unwrap_or_default();
                let pid: u32 = content.trim().parse().ok().unwrap_or(0);
                PidLockError::AlreadyLocked {
                    pid,
                    lock_path: lock_path.to_string_lossy().to_string(),
                }
            } else {
                PidLockError::Io {
                    action: "create PID lock",
                    path: lock_path_owned.clone(),
                    source: e,
                }
            }
        })?;

        use std::io::Write;
        file.write_all(current_pid.to_string().as_bytes())
            .map_err(|e| PidLockError::Io {
                action: "write PID to lock",
                path: lock_path_owned.clone(),
                source: e,
            })?;

        Ok(Self {
            lock_path: lock_path_owned,
        })
    }
}

impl Drop for PidLockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.lock_path);
    }
}

#[derive(Debug)]
pub enum PidLockError {
    AlreadyLocked {
        pid: u32,
        lock_path: String,
    },
    Io {
        action: &'static str,
        path: std::path::PathBuf,
        source: io::Error,
    },
}

impl std::fmt::Display for PidLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyLocked { pid, lock_path } => {
                write!(
                    f,
                    "Data dir locked by MemFuse process PID={} at {}",
                    pid, lock_path
                )
            }
            Self::Io {
                action,
                path,
                source,
            } => {
                write!(
                    f,
                    "IO error during '{}' on {}: {}",
                    action,
                    path.display(),
                    source
                )
            }
        }
    }
}

impl std::error::Error for PidLockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::AlreadyLocked { .. } => None,
        }
    }
}

/// Check if a PID is still alive.
///
/// On Unix, uses `kill(pid, 0)` which only checks process existence without sending a signal.
/// Returns `true` if the process exists (including when we lack permission to signal it —
/// EPERM), and `false` only when the process definitively does not exist (ESRCH).
#[allow(unsafe_code)]
fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: kill(pid, 0) is a well-defined POSIX call that only checks process existence
        let ret = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if ret == 0 {
            // Process exists and we have permission to signal it
            true
        } else {
            let errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if errno == libc::ESRCH {
                // Process does not exist — stale lock
                false
            } else {
                // EPERM or other error — process exists but we lack permission
                // Treat as alive to avoid incorrectly removing someone else's lock
                true
            }
        }
    }

    #[cfg(not(unix))]
    {
        // Fallback: always treat stale PIDs as not alive on non-Unix platforms
        false
    }
}
