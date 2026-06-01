use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tokio::fs;
use tokio::time::sleep;

use crate::SessionError;

pub struct PathLockGuard {
    lock_path: PathBuf,
}

impl Drop for PathLockGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

pub async fn acquire_path_lock(
    lock_path: &Path,
    timeout: Duration,
) -> Result<PathLockGuard, SessionError> {
    if let Some(parent) = lock_path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|source| SessionError::io("create lock directory", parent, source))?;
    }

    let deadline = Instant::now() + timeout;
    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
        {
            Ok(_) => {
                return Ok(PathLockGuard {
                    lock_path: lock_path.to_path_buf(),
                });
            }
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                if Instant::now() >= deadline {
                    return Err(SessionError::LockTimeout(lock_path.to_path_buf()));
                }
                sleep(Duration::from_millis(25)).await;
            }
            Err(source) => {
                return Err(SessionError::io("acquire path lock", lock_path, source));
            }
        }
    }
}
