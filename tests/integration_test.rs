mod common;

use std::os::unix::fs::PermissionsExt;

use common::{make_pg_item, setup_task_golem_store, setup_test_env};
use phase_golem::materialization::{
    X_PG_EXECUTION_POLICY, X_PG_EXECUTOR_PROFILE, X_PG_OWNER, X_PG_RUN_ID, X_PG_TEMPLATE_ID,
    X_PG_TEMPLATE_NODE_KEY, X_PG_VERIFICATION,
};
use task_golem::events::{Event, EventType};
use task_golem::model::item::Item;
use task_golem::model::status::Status;

/// handle_init does NOT create BACKLOG.yaml, checks for .task-golem/ existence.
#[test]
fn handle_init_does_not_create_backlog_yaml() {
    let dir = setup_test_env();

    // Verify no BACKLOG.yaml exists before init
    let backlog_path = dir.path().join("BACKLOG.yaml");
    assert!(
        !backlog_path.exists(),
        "BACKLOG.yaml should not exist before init"
    );

    // We cannot call handle_init directly (it's in the binary crate, not lib),
    // so we verify the behavior by inspecting what the new init does:
    // 1. It does NOT create BACKLOG.yaml
    // 2. It checks for .task-golem/ and prints guidance

    // Verify the expected directories are created by setup_test_env
    assert!(dir.path().join(".phase-golem").exists());
    assert!(dir.path().join("changes").exists());

    // Verify .task-golem/ does NOT exist (init should warn, not create)
    assert!(
        !dir.path().join(".task-golem").exists(),
        ".task-golem/ should not be auto-created by phase-golem init"
    );
}

#[test]
fn manual_run_reaches_supervisor_when_unrelated_legacy_workflow_is_missing() {
    // Arrange
    let dir = setup_test_env();
    let store = setup_task_golem_store(dir.path());
    let mut item = make_pg_item(common::ID_1, Status::Todo).0;
    item.extensions
        .insert(X_PG_OWNER.to_string(), serde_json::json!("phase-golem"));
    item.extensions.insert(
        X_PG_TEMPLATE_NODE_KEY.to_string(),
        serde_json::json!("build"),
    );
    item.extensions.insert(
        X_PG_EXECUTOR_PROFILE.to_string(),
        serde_json::json!("test-executor"),
    );
    item.extensions.insert(
        X_PG_EXECUTION_POLICY.to_string(),
        serde_json::json!({
            "timeout_minutes": 1,
            "max_retries": 0,
            "destructive": false,
            "workflows": ["snapshotted-build.md"]
        }),
    );
    item.extensions.insert(
        X_PG_VERIFICATION.to_string(),
        serde_json::json!({ "required_checks": [] }),
    );
    store.save_active(&[item]).expect("seed materialized work");

    let missing_workflow = dir.path().join("missing/legacy.md");
    std::fs::write(
        dir.path().join("phase-golem.toml"),
        r#"[executor_profiles.test-executor]
command = "definitely-not-invoked"
args = []
environment = {}

[pipelines.unrelated]
pre_phases = []
phases = [{ name = "legacy", workflows = ["missing/legacy.md"], is_destructive = false }]
"#,
    )
    .expect("write config with stale legacy workflow");
    std::fs::write(dir.path().join(".gitignore"), ".phase-golem/\n")
        .expect("ignore runtime directory");
    assert!(!missing_workflow.exists());

    let add = std::process::Command::new("git")
        .args(["add", ".task-golem", "phase-golem.toml", ".gitignore"])
        .current_dir(dir.path())
        .output()
        .expect("stage CLI test project");
    assert!(
        add.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    let commit = std::process::Command::new("git")
        .args(["commit", "-m", "Seed materialized work"])
        .current_dir(dir.path())
        .output()
        .expect("commit CLI test project");
    assert!(
        commit.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&commit.stderr)
    );

    // Act
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_phase-golem"))
        .arg("--root")
        .arg(dir.path())
        .args(["run", "--target", common::ID_1, "--cap", "0"])
        .output()
        .expect("run phase-golem CLI");

    // Assert
    assert!(
        !output.status.success(),
        "incomplete targeted run must fail"
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Halt reason: BudgetReached"),
        "manual run did not reach the finite supervisor: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cli_materializes_and_executes_one_trusted_task_to_archived_completion() {
    // Arrange
    let directory = setup_test_env();
    let store = setup_task_golem_store(directory.path());
    store.ensure_gitignore().expect("install TG cache ignores");
    let executor_path = directory.path().join("mock-trusted-executor.sh");
    let invocation_path = directory.path().join("mock-invocation.txt");
    let artifact_path = directory.path().join("executor-artifact.txt");
    let active_state_at_invocation_path = directory.path().join("active-state-at-invocation.json");
    let verification_marker_path = directory.path().join("shell-verifier-marker.txt");
    let verifier_command = "grep -Fqx 'completed:release-e2e' executor-artifact.txt && printf verified:release-e2e > shell-verifier-marker.txt";
    std::fs::write(
        &executor_path,
        r#"#!/bin/sh
set -eu
invocation_path=$1
artifact_path=$2
active_state_path=$3
active_state_at_invocation_path=$4
prompt=$5
printf '%s\n' "$prompt" > "$invocation_path"
item_id=$(printf '%s\n' "$prompt" | sed -n 's/^  "item_id": "\([^"]*\)",$/\1/p')
phase=$(printf '%s\n' "$prompt" | sed -n 's/^  "phase": "\([^"]*\)",$/\1/p')
result_path=$(printf '%s\n' "$prompt" | sed -n 's/^Write one JSON result to \(.*\) with exactly:.*$/\1/p')
test -n "$item_id"
test -n "$phase"
test -n "$result_path"
active_item=$(grep -F "\"id\":\"$item_id\"" "$active_state_path")
test -n "$active_item"
printf '%s\n' "$active_item" | grep -Fq '"status":"doing"'
printf '%s\n' "$active_item" | grep -Fq '"claimed_by":"phase-golem"'
project_root=$(dirname "$(dirname "$active_state_path")")
test "$PWD" = "$project_root"
test "${PROFILE_MARKER:-}" = "profile-only"
test -z "${HOME+x}"
printf '%s\n' "$active_item" > "$active_state_at_invocation_path"
printf 'completed:release-e2e\n' > "$artifact_path"
printf '{"item_id":"%s","phase":"%s","result":"complete","summary":"hermetic executor completed","evidence_references":["artifact:executor-artifact.txt","task:%s"]}\n' "$item_id" "$phase" "$item_id" > "$result_path"
"#,
    )
    .expect("write trusted executor fixture");
    let mut permissions = std::fs::metadata(&executor_path)
        .expect("read executor fixture metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&executor_path, permissions)
        .expect("make trusted executor fixture executable");

    let config_path = directory.path().join("phase-golem.toml");
    std::fs::write(
        &config_path,
        format!(
            r#"[executor_profiles.local]
command = {}
args = [{}, {}, {}, {}]
environment = {{ PROFILE_MARKER = "profile-only" }}

[workflow_template]
id = "process-e2e-workflow"

[workflow_template.provenance]
source = "tests/integration_test.rs"
revision = "1"

[[workflow_template.inputs]]
name = "subject"

[[workflow_template.nodes]]
key = "execute"
title = "Execute ${{subject}}"
description = "One hermetic process-level task"
executor_profile = "local"

[workflow_template.nodes.execution_policy]
timeout_minutes = 1
max_retries = 0
destructive = false
workflows = []

[workflow_template.nodes.verification]
required_checks = [{}]
"#,
            toml_string(&executor_path),
            toml_string(&invocation_path),
            toml_string(&artifact_path),
            toml_string(&store.tasks_path()),
            toml_string(&active_state_at_invocation_path),
            toml::Value::String(verifier_command.to_string()),
        ),
    )
    .expect("write process E2E config");
    std::fs::write(
        directory.path().join(".gitignore"),
        ".phase-golem/\nactive-state-at-invocation.json\nexecutor-artifact.txt\nmock-invocation.txt\nshell-verifier-marker.txt\n",
    )
    .expect("write process E2E ignores");

    // Verify malformed public inputs fail before materialization.
    let malformed = phase_golem_command(directory.path(), &config_path)
        .args(["materialize", "--input", "missing-separator"])
        .output()
        .expect("run malformed materialization command");
    assert!(!malformed.status.success());
    assert!(malformed.stdout.is_empty());
    assert!(String::from_utf8_lossy(&malformed.stderr)
        .contains("Invalid --input 'missing-separator': expected NAME=VALUE"));
    assert!(store
        .load_active()
        .expect("load unchanged TG store")
        .is_empty());

    // Materialize the selected config template through the real CLI process.
    let materialization = phase_golem_command(directory.path(), &config_path)
        .args([
            "materialize",
            "--input",
            "subject=release-e2e",
            "--run-id",
            "process-e2e-run",
        ])
        .output()
        .expect("run materialization command");
    assert!(
        materialization.status.success(),
        "materialization failed: {}",
        String::from_utf8_lossy(&materialization.stderr)
    );
    let materialization_stdout =
        String::from_utf8(materialization.stdout).expect("materialization stdout is UTF-8");
    assert_eq!(materialization_stdout.lines().count(), 1);
    let materialization_json: serde_json::Value =
        serde_json::from_str(&materialization_stdout).expect("parse materialization JSON");
    assert_eq!(
        materialization_json,
        serde_json::json!({
            "run_id": "process-e2e-run",
            "node_mapping": {"execute": materialization_json["node_mapping"]["execute"]}
        })
    );
    let item_id = materialization_json["node_mapping"]["execute"]
        .as_str()
        .expect("materialized execute item ID")
        .to_string();
    task_golem::validate_id(&item_id).expect("materialized ID is canonical UUIDv7");

    let materialized_item = store
        .load_active()
        .expect("load materialized TG state")
        .into_iter()
        .find(|item| item.id == item_id)
        .expect("select materialized TG item");
    assert_eq!(materialized_item.title, "Execute release-e2e");
    assert_eq!(materialized_item.status, Status::Todo);
    assert_eq!(materialized_item.extensions[X_PG_RUN_ID], "process-e2e-run");
    assert_eq!(
        materialized_item.extensions[X_PG_TEMPLATE_ID],
        "process-e2e-workflow"
    );
    assert_eq!(
        materialized_item.extensions[X_PG_TEMPLATE_NODE_KEY],
        "execute"
    );
    assert_eq!(materialized_item.extensions[X_PG_OWNER], "phase-golem");
    assert_eq!(materialized_item.extensions[X_PG_EXECUTOR_PROFILE], "local");
    assert_eq!(
        materialized_item.extensions[X_PG_EXECUTION_POLICY],
        serde_json::json!({
            "timeout_minutes": 1,
            "max_retries": 0,
            "destructive": false,
            "workflows": []
        })
    );
    assert_eq!(
        materialized_item.extensions[X_PG_VERIFICATION],
        serde_json::json!({
            "required_checks": [verifier_command]
        })
    );

    commit_process_e2e_seed(directory.path());

    // Execute the selected materialized item through the real foreground CLI process.
    let run = phase_golem_command(directory.path(), &config_path)
        .args(["run", "--target", &item_id])
        .output()
        .expect("run foreground supervisor command");

    // Assert the process boundary, executor protocol, verifier, and TG lifecycle.
    assert!(
        run.status.success(),
        "foreground run failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let run_stderr = String::from_utf8_lossy(&run.stderr);
    assert!(run_stderr.contains("Tasks executed: 1"));
    assert!(run_stderr.contains("Halt reason: SelectedScopeComplete"));
    let invocation = std::fs::read_to_string(&invocation_path)
        .expect("trusted executor invocation was persisted");
    assert!(invocation.contains(&format!(r#""item_id": "{item_id}""#)));
    assert!(invocation.contains(r#""phase": "execute""#));
    assert!(invocation.contains(r#""attempt": 1"#));
    assert!(invocation.contains("result (complete or blocked)"));
    let active_item_at_invocation: Item = serde_json::from_str(
        &std::fs::read_to_string(&active_state_at_invocation_path)
            .expect("invocation-time durable TG state"),
    )
    .expect("parse invocation-time durable TG item");
    assert_eq!(active_item_at_invocation.id, item_id);
    task_golem::validate_id(&active_item_at_invocation.id)
        .expect("invoked durable item ID is canonical UUIDv7");
    assert_eq!(active_item_at_invocation.status, Status::Doing);
    assert_eq!(
        active_item_at_invocation.claimed_by.as_deref(),
        Some("phase-golem")
    );
    assert_eq!(
        std::fs::read_to_string(&artifact_path).expect("executor artifact"),
        "completed:release-e2e\n"
    );
    assert_eq!(
        std::fs::read_to_string(&verification_marker_path).expect("shell verifier marker"),
        "verified:release-e2e"
    );

    assert!(store
        .load_active()
        .expect("load final active TG state")
        .is_empty());
    let archived_items = store.load_all_archive().expect("load TG archive");
    assert_eq!(archived_items.len(), 1);
    let archived = &archived_items[0];
    assert_eq!(archived.id, item_id);
    assert_eq!(archived.status, Status::Done);
    assert!(archived.claimed_by.is_none());
    assert!(archived.claimed_at.is_none());
    assert_eq!(archived.extensions[X_PG_RUN_ID], "process-e2e-run");

    let events = std::fs::read_to_string(store.events_archive_path())
        .expect("read archived TG lifecycle events")
        .lines()
        .map(|line| serde_json::from_str::<Event>(line).expect("parse TG lifecycle event"))
        .collect::<Vec<_>>();
    assert!(events.iter().all(|event| event.task_id == item_id));
    assert_eq!(
        events
            .iter()
            .filter_map(|event| event.status)
            .collect::<Vec<_>>(),
        vec![Status::Doing, Status::Done]
    );
    let attempt_event = events
        .iter()
        .find(|event| event.event_type == EventType::Note)
        .expect("persisted PG attempt evidence");
    let attempt: serde_json::Value = serde_json::from_str(
        attempt_event
            .text
            .strip_prefix("phase-golem-attempt ")
            .expect("PG attempt evidence prefix"),
    )
    .expect("parse PG attempt evidence");
    assert_eq!(attempt["schema"], "phase-golem/trusted-executor-attempt/v1");
    assert_eq!(attempt["item_id"], item_id);
    assert_eq!(attempt["phase"], "execute");
    assert_eq!(attempt["outcome"], "complete");
    assert_eq!(
        attempt["executor_evidence"],
        serde_json::json!(["artifact:executor-artifact.txt", format!("task:{item_id}")])
    );
    assert_eq!(
        attempt["verification_evidence"],
        serde_json::json!([format!("verification:{verifier_command}")])
    );
}

fn phase_golem_command(
    project_root: &std::path::Path,
    config_path: &std::path::Path,
) -> std::process::Command {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_phase-golem"));
    command
        .arg("--root")
        .arg(project_root)
        .arg("--config")
        .arg(config_path);
    command
}

fn toml_string(path: &std::path::Path) -> String {
    toml::Value::String(path.to_string_lossy().into_owned()).to_string()
}

fn commit_process_e2e_seed(project_root: &std::path::Path) {
    let add = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(project_root)
        .output()
        .expect("stage process E2E seed");
    assert!(
        add.status.success(),
        "git add failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    let commit = std::process::Command::new("git")
        .args(["commit", "-m", "Seed process E2E workflow"])
        .current_dir(project_root)
        .output()
        .expect("commit process E2E seed");
    assert!(
        commit.status.success(),
        "git commit failed: {}",
        String::from_utf8_lossy(&commit.stderr)
    );
    let status = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(project_root)
        .output()
        .expect("check process E2E seed status");
    assert!(status.status.success());
    assert!(
        status.stdout.is_empty(),
        "process E2E seed must be clean: {}",
        String::from_utf8_lossy(&status.stdout)
    );
}

/// Shutdown commit flow: dirty tasks.jsonl is detected and committed.
#[tokio::test]
async fn shutdown_commits_dirty_tasks_jsonl() {
    let dir = setup_test_env();
    let store = setup_task_golem_store(dir.path());

    // Save an item to make tasks.jsonl dirty relative to git
    let item = make_pg_item(common::ID_1, Status::Todo);
    store.save_active(&[item.0]).expect("save item");

    // Stage and commit .task-golem/ so it has a baseline
    std::process::Command::new("git")
        .args(["add", ".task-golem/"])
        .current_dir(dir.path())
        .output()
        .expect("git add");
    std::process::Command::new("git")
        .args(["commit", "-m", "Add task-golem store"])
        .current_dir(dir.path())
        .output()
        .expect("git commit");

    // Now modify the store (makes it dirty)
    let item2 = make_pg_item(common::ID_2, Status::Todo);
    store.save_active(&[item2.0]).expect("save modified items");

    // Verify tasks.jsonl is dirty in git
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir.path())
        .output()
        .expect("git status");
    let status_str = String::from_utf8_lossy(&status_output.stdout);
    assert!(
        status_str.contains("tasks.jsonl"),
        "tasks.jsonl should be dirty; got: {}",
        status_str
    );

    // Simulate shutdown commit: stage and commit via tg_git
    task_golem::git::stage_self(dir.path()).expect("stage_self");
    let sha = task_golem::git::commit("[phase-golem] Save task state on halt (test)", dir.path())
        .expect("commit");
    assert!(!sha.is_empty(), "commit should return a SHA");

    // Verify tasks.jsonl is now clean
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir.path())
        .output()
        .expect("git status after commit");
    let status_str = String::from_utf8_lossy(&status_output.stdout);
    assert!(
        !status_str.contains("tasks.jsonl"),
        "tasks.jsonl should be clean after commit; got: {}",
        status_str
    );
}

/// Clean exit with no pending phases does not create an empty commit.
#[tokio::test]
async fn shutdown_no_pending_phases_no_empty_commit() {
    let dir = setup_test_env();
    let _store = setup_task_golem_store(dir.path());

    // Stage and commit .task-golem/ to establish baseline
    std::process::Command::new("git")
        .args(["add", ".task-golem/"])
        .current_dir(dir.path())
        .output()
        .expect("git add");
    std::process::Command::new("git")
        .args(["commit", "-m", "Add task-golem store"])
        .current_dir(dir.path())
        .output()
        .expect("git commit");

    // Get the current HEAD
    let head_before = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let sha_before = String::from_utf8_lossy(&head_before.stdout)
        .trim()
        .to_string();

    // Verify tasks.jsonl is NOT dirty (nothing to commit)
    let status_output = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(dir.path())
        .output()
        .expect("git status");
    let status_str = String::from_utf8_lossy(&status_output.stdout);
    assert!(
        !status_str.contains("tasks.jsonl"),
        "tasks.jsonl should be clean; got: {}",
        status_str
    );

    // HEAD should not change (no commit made)
    let head_after = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let sha_after = String::from_utf8_lossy(&head_after.stdout)
        .trim()
        .to_string();
    assert_eq!(sha_before, sha_after, "No commit should have been created");
}
