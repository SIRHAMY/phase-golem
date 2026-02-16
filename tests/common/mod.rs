#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

use orchestrate::config::{default_feature_pipeline, OrchestrateConfig};
use orchestrate::types::{BacklogFile, BacklogItem, ItemStatus};

/// Creates a `BacklogItem` with minimal defaults.
///
/// All optional fields are set to `None`, collections to empty, and timestamps
/// to `"2026-02-10T00:00:00+00:00"`. The title is auto-generated as
/// `"Test item {id}"`.
///
/// # Parameters
/// - `id`: The item identifier (e.g., `"WRK-001"`)
/// - `status`: The initial `ItemStatus` for the item
pub fn make_item(id: &str, status: ItemStatus) -> BacklogItem {
    BacklogItem {
        id: id.to_string(),
        title: format!("Test item {}", id),
        status,
        created: "2026-02-10T00:00:00+00:00".to_string(),
        updated: "2026-02-10T00:00:00+00:00".to_string(),
        ..Default::default()
    }
}

/// Creates an in-progress `BacklogItem` with the given phase.
///
/// Calls `make_item` with `ItemStatus::InProgress`, then sets `item.phase`.
/// Uses minimal defaults: `phase_pool` is `None`. Callers that need a specific
/// `PhasePool` should set it explicitly after calling this function.
///
/// # Parameters
/// - `id`: The item identifier
/// - `phase`: The phase name (e.g., `"build"`, `"prd"`)
pub fn make_in_progress_item(id: &str, phase: &str) -> BacklogItem {
    let mut item = make_item(id, ItemStatus::InProgress);
    item.phase = Some(phase.to_string());
    item
}

/// Creates a `BacklogFile` containing the given items.
///
/// Sets `schema_version: 3` and `next_item_id: 0`.
///
/// # Parameters
/// - `items`: The list of `BacklogItem`s to include
pub fn make_backlog(items: Vec<BacklogItem>) -> BacklogFile {
    BacklogFile {
        schema_version: 3,
        items,
        next_item_id: 0,
    }
}

/// Creates an empty `BacklogFile` with no items.
///
/// Equivalent to `make_backlog(vec![])`.
pub fn empty_backlog() -> BacklogFile {
    make_backlog(vec![])
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
/// - Directories: `_ideas`, `_worklog`, `changes`, `.orchestrator`
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

    let dirs = ["_ideas", "_worklog", "changes", ".orchestrator"];
    for d in &dirs {
        fs::create_dir_all(dir.path().join(d)).expect("Failed to create dir");
    }

    dir
}

/// Creates an `OrchestrateConfig` with a `"feature"` pipeline using the
/// default feature pipeline configuration.
///
/// Uses `OrchestrateConfig::default()` as the base, then inserts the
/// `default_feature_pipeline()` under the `"feature"` key.
pub fn default_config() -> OrchestrateConfig {
    let mut config = OrchestrateConfig::default();
    config
        .pipelines
        .insert("feature".to_string(), default_feature_pipeline());
    config
}
