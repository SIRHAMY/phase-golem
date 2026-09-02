#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use phase_golem::config::{default_feature_pipeline, PhaseGolemConfig};
use phase_golem::pg_item::{self, PgItem};
use phase_golem::types::PhasePool;
use task_golem::model::status::Status;
use task_golem::store::Store;
use tempfile::TempDir;

pub const ID_1: &str = "018f2b1c-4d5e-7abc-8123-456789abcdef";
pub const ID_2: &str = "018f2b1c-4d5e-7abc-9234-56789abcdef0";
pub const ID_3: &str = "018f2b1c-4d5e-7abc-a345-6789abcdef01";
pub const ID_4: &str = "018f2b1c-4d5e-7abc-b456-789abcdef012";

pub fn make_pg_item(id: &str, status: Status) -> PgItem {
    pg_item::new_from_parts(
        id.to_string(),
        format!("Test item {id}"),
        status,
        vec![],
        vec![],
    )
}

pub fn make_doing_pg_item(id: &str, phase: &str) -> PgItem {
    let mut item = make_pg_item(id, Status::Doing);
    item.0.claimed_by = Some("phase-golem".to_string());
    item.0.claimed_at = Some(item.0.updated_at);
    pg_item::set_phase(&mut item.0, Some(phase));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));
    pg_item::set_pipeline_type(&mut item.0, Some("feature"));
    item
}

pub fn make_blocked_pg_item(id: &str, from_status: Status) -> PgItem {
    let mut item = make_pg_item(id, Status::Blocked);
    item.0.blocked_reason = Some("test block reason".to_string());
    item.0.blocked_from_status = Some(from_status);
    item
}

pub fn setup_task_golem_store(dir: &Path) -> Store {
    let tg_dir = dir.join(".task-golem");
    fs::create_dir_all(&tg_dir).expect("create task-golem directory");
    let store = Store::new(tg_dir.clone());
    store.save_active(&[]).expect("initialize active store");
    fs::write(tg_dir.join("archive.jsonl"), "{\"schema_version\":1}\n")
        .expect("initialize archive");
    store
}

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

pub fn fixture_path(name: &str) -> PathBuf {
    fixtures_dir().join(name)
}

pub fn setup_test_env() -> TempDir {
    let dir = TempDir::new().expect("create temp dir");
    for args in [
        vec!["init"],
        vec!["config", "user.email", "test@test.com"],
        vec!["config", "user.name", "Test"],
    ] {
        Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .expect("configure git test repository");
    }
    fs::write(dir.path().join("README.md"), "# Test\n").expect("write readme");
    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(dir.path())
        .output()
        .expect("stage readme");
    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(dir.path())
        .output()
        .expect("commit readme");
    for path in ["_ideas", "_worklog", "changes", ".phase-golem"] {
        fs::create_dir_all(dir.path().join(path)).expect("create test directory");
    }
    dir
}

pub fn default_config() -> PhaseGolemConfig {
    let mut config = PhaseGolemConfig::default();
    config
        .pipelines
        .insert("feature".to_string(), default_feature_pipeline());
    config
}
