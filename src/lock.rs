use std::fs;
use std::path::{Path, PathBuf};

use crate::log_warn;

/// A lock guard that releases the lock file on drop.
#[must_use = "lock is released when LockGuard is dropped"]
pub struct LockGuard {
    lock: fslock::LockFile,
    pid_path: PathBuf,
}

impl std::fmt::Debug for LockGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LockGuard")
            .field("pid_path", &self.pid_path)
            .finish()
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Err(e) = self.lock.unlock() {
            log_warn!("Warning: Failed to release lock: {}", e);
        }
        if let Err(e) = fs::remove_file(&self.pid_path) {
            log_warn!(
                "Warning: Failed to remove PID file {}: {}",
                self.pid_path.display(),
                e
            );
        }
    }
}

/// Attempts to acquire the phase-golem lock.
///
/// Creates the `.phase-golem/` directory if it doesn't exist.
/// Acquires the file lock first (atomic mutual exclusion), then writes a PID
/// file for diagnostics. On contention, checks the PID file to provide
/// actionable error messages about the holding process.
///
/// Returns a `LockGuard` that automatically releases on drop.
pub fn try_acquire(runtime_dir: &Path) -> Result<LockGuard, String> {
    fs::create_dir_all(runtime_dir)
        .map_err(|e| format!("Failed to create {}: {}", runtime_dir.display(), e))?;

    let lock_path = runtime_dir.join("phase-golem.lock");
    let pid_path = runtime_dir.join("phase-golem.pid");

    let mut lock = fslock::LockFile::open(&lock_path)
        .map_err(|e| format!("Failed to open lock file {}: {}", lock_path.display(), e))?;

    let acquired = lock
        .try_lock()
        .map_err(|e| format!("Failed to acquire lock: {}", e))?;

    if !acquired {
        // Lock is held — check PID file for a helpful error message
        let holder_info = fs::read_to_string(&pid_path)
            .ok()
            .and_then(|s| s.trim().parse::<i32>().ok());

        return match holder_info {
            Some(pid) if is_pid_alive(pid) => Err(format!(
                "Another phase-golem instance is running (PID {})",
                pid
            )),
            Some(pid) => {
                // Lock is held but PID is dead — OS-level flock should have
                // been released on process death, so this is unexpected.
                // Report it so the user can investigate.
                Err(format!(
                    "Lock file is held but recorded PID {} is not alive. \
                     Remove {} and {} to recover",
                    pid,
                    lock_path.display(),
                    pid_path.display()
                ))
            }
            None => Err(format!(
                "Another phase-golem instance holds the lock. \
                 If this is stale, remove {}",
                lock_path.display()
            )),
        };
    }

    // We hold the lock — safe to write PID
    fs::write(&pid_path, std::process::id().to_string())
        .map_err(|e| format!("Failed to write PID file: {}", e))?;

    Ok(LockGuard { lock, pid_path })
}

fn is_pid_alive(pid: i32) -> bool {
    // signal 0 checks if process exists without sending a signal
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_pid_alive_current_process() {
        let pid = std::process::id() as i32;
        assert!(is_pid_alive(pid));
    }

    #[test]
    fn test_is_pid_alive_nonexistent() {
        // PID 99999999 is almost certainly not alive
        assert!(!is_pid_alive(99_999_999));
    }
}
