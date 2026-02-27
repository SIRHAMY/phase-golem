#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;
use task_golem::store::Store;

use phase_golem::config::{default_feature_pipeline, PhaseGolemConfig};
use phase_golem::pg_item::{self, PgItem};
use phase_golem::types::{ItemStatus, PhasePool};

// =============================================================================
// PgItem test helpers
// =============================================================================

/// Creates a `PgItem` with minimal defaults via the adapter constructor.
///
/// The item has no phase, no phase pool, no pipeline type, and empty tags.
/// The title is auto-generated as `"Test item {id}"`.
pub fn make_pg_item(id: &str, status: ItemStatus) -> PgItem {
    pg_item::new_from_parts(
        id.to_string(),
        format!("Test item {}", id),
        status,
        vec![],
        vec![],
    )
}

/// Creates an in-progress `PgItem` with the given phase.
///
/// Sets `x-pg-phase`, `x-pg-phase-pool` to `Main`, and `x-pg-pipeline-type` to `"feature"`.
pub fn make_in_progress_pg_item(id: &str, phase: &str) -> PgItem {
    let mut pg = make_pg_item(id, ItemStatus::InProgress);
    pg_item::set_phase(&mut pg.0, Some(phase));
    pg_item::set_phase_pool(&mut pg.0, Some(&PhasePool::Main));
    pg_item::set_pipeline_type(&mut pg.0, Some("feature"));
    pg
}

/// Creates a blocked `PgItem` with the given `from_status`.
///
/// Sets `x-pg-blocked-from-status` and `blocked_reason` to `"test block reason"`.
pub fn make_blocked_pg_item(id: &str, from_status: ItemStatus) -> PgItem {
    let mut pg = make_pg_item(id, ItemStatus::Blocked);
    pg_item::set_blocked_from_status(&mut pg.0, Some(&from_status));
    pg.0.blocked_reason = Some("test block reason".to_string());
    pg
}

/// Wraps a list of `PgItem`s into a `Vec<PgItem>`.
pub fn make_pg_items(items: Vec<PgItem>) -> Vec<PgItem> {
    items
}

/// Creates a `.task-golem/` directory and initializes an empty Store.
///
/// Creates the `.task-golem/` directory, writes an empty tasks.jsonl and archive.jsonl
/// via `Store::save_active(&[])` + creating a schema header for archive.
/// Returns a `Store` pointing at the `.task-golem/` directory.
pub fn setup_task_golem_store(dir: &Path) -> Store {
    let tg_dir = dir.join(".task-golem");
    fs::create_dir_all(&tg_dir).expect("Failed to create .task-golem dir");

    let store = Store::new(tg_dir.clone());
    store
        .save_active(&[])
        .expect("Failed to initialize tasks.jsonl");

    // Create archive.jsonl with schema header
    fs::write(tg_dir.join("archive.jsonl"), "{\"schema_version\":1}\n")
        .expect("Failed to initialize archive.jsonl");

    store
}

/// Acquires the task-golem file lock from a background thread and returns a guard.
///
/// Dropping the guard releases the lock by signaling the thread to stop.
/// Used to simulate lock contention for `LockTimeout` retry tests.
///
/// Uses `fslock` (already a phase-golem dependency) to hold an exclusive lock
/// on the same file that task-golem's `with_lock()` uses.
pub struct LockGuard {
    _keep_alive: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        self._keep_alive.store(false, std::sync::atomic::Ordering::SeqCst);
        if let Some(handle) = self._thread.take() {
            let _ = handle.join();
        }
    }
}

/// Holds the task-golem store file lock until the returned `LockGuard` is dropped.
///
/// Acquires an exclusive `fd_lock::RwLock` on the `tasks.lock` file inside `store_dir`,
/// which is the same lock that task-golem's `with_lock()` contends for.
pub fn hold_store_lock(store_dir: &Path) -> LockGuard {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let keep_alive = Arc::new(AtomicBool::new(true));
    let keep_alive_clone = keep_alive.clone();
    let lock_path = store_dir.join("tasks.lock");

    // Signal that the lock has been acquired
    let (tx, rx) = std::sync::mpsc::channel();

    let thread = std::thread::spawn(move || {
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .expect("open lock file");

        let mut lock = fd_lock::RwLock::new(file);
        let _guard = lock.try_write().expect("acquire lock");
        let _ = tx.send(()); // Signal lock acquired

        while keep_alive_clone.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        // _guard dropped here, releasing the lock
    });

    // Wait for the lock to actually be acquired before returning
    rx.recv().expect("lock thread should signal acquisition");

    LockGuard {
        _keep_alive: keep_alive,
        _thread: Some(thread),
    }
}

/// Returns the path to the `tests/fixtures` directory.
///
/// Uses `CARGO_MANIFEST_DIR` to locate the project root, then appends
/// `tests/fixtures`.
pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Returns the path to a specific fixture file.
///
/// Joins the given `name` onto `fixtures_dir()`.
///
/// # Parameters
/// - `name`: The fixture filename (e.g., `"backlog_full.yaml"`)
pub fn fixture_path(name: &str) -> PathBuf {
    fixtures_dir().join(name)
}

/// Creates a temporary directory initialized as a git repository with a
/// standard project structure.
///
/// The environment includes:
/// - A git repo with user config (`test@test.com` / `Test`)
/// - An initial commit with `README.md`
/// - Directories: `_ideas`, `_worklog`, `changes`, `.phase-golem`
///
/// Returns the `TempDir` handle. The directory is cleaned up when dropped.
pub fn setup_test_env() -> TempDir {
    let dir = TempDir::new().expect("Failed to create temp dir");

    Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .expect("Failed to init git repo");

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir.path())
        .output()
        .expect("Failed to set git email");

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir.path())
        .output()
        .expect("Failed to set git name");

    let readme = dir.path().join("README.md");
    fs::write(&readme, "# Test\n").expect("Failed to write README");

    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(dir.path())
        .output()
        .expect("Failed to stage README");

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(dir.path())
        .output()
        .expect("Failed to commit");

    let dirs = ["_ideas", "_worklog", "changes", ".phase-golem"];
    for d in &dirs {
        fs::create_dir_all(dir.path().join(d)).expect("Failed to create dir");
    }

    dir
}

/// Creates an `PhaseGolemConfig` with a `"feature"` pipeline using the
/// default feature pipeline configuration.
///
/// Uses `PhaseGolemConfig::default()` as the base, then inserts the
/// `default_feature_pipeline()` under the `"feature"` key.
pub fn default_config() -> PhaseGolemConfig {
    let mut config = PhaseGolemConfig::default();
    config
        .pipelines
        .insert("feature".to_string(), default_feature_pipeline());
    config
}
