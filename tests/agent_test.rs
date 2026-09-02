mod common;

use std::fs;
use std::path::Path;
use std::time::Duration;

use tempfile::TempDir;

use phase_golem::agent::{
    read_result_file, read_trusted_executor_result_file, run_agent_for_claimed_pg_item,
    run_shell_command, run_subprocess_agent, AgentRunner, MockAgentRunner,
    MAX_TRUSTED_RESULT_PAYLOAD_BYTES,
};
use phase_golem::pg_item::X_PG_OWNER;
use phase_golem::types::{PhaseResult, ResultCode};
use task_golem::model::status::Status;

/// Create a valid PhaseResult JSON string.
fn valid_result_json() -> String {
    serde_json::to_string_pretty(&PhaseResult {
        item_id: common::ID_1.to_string(),
        phase: "prd".to_string(),
        result: ResultCode::PhaseComplete,
        summary: "Created PRD with all sections filled".to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: vec![],
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
        description: None,
    })
    .unwrap()
}

fn make_result(result_code: ResultCode, summary: &str) -> PhaseResult {
    PhaseResult {
        item_id: common::ID_1.to_string(),
        phase: "prd".to_string(),
        result: result_code,
        summary: summary.to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: vec![],
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
        description: None,
    }
}

#[tokio::test]
async fn triage_agent_is_not_invoked_after_execution_claim_revocation() {
    for revoked_item in [
        {
            let mut item = common::make_doing_pg_item(common::ID_1, "build");
            item.0.claimed_by = None;
            item
        },
        {
            let mut item = common::make_doing_pg_item(common::ID_1, "build");
            item.0.status = Status::Todo;
            item
        },
        {
            let mut item = common::make_doing_pg_item(common::ID_1, "build");
            item.0.extensions.remove(X_PG_OWNER);
            item
        },
    ] {
        let runner = MockAgentRunner::new(vec![]);

        let result = run_agent_for_claimed_pg_item(
            &revoked_item,
            &runner,
            "triage prompt",
            Path::new("result.json"),
            Duration::from_secs(1),
        )
        .await;

        assert!(result.is_none(), "revoked work must not invoke an agent");
    }
}

// --- read_result_file tests ---

#[tokio::test]
async fn read_result_file_valid_json() {
    let dir = TempDir::new().unwrap();
    let result_path = dir.path().join("result.json");
    fs::write(&result_path, valid_result_json()).unwrap();

    let result = read_result_file(&result_path).await;
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
    let pr = result.unwrap();
    assert_eq!(pr.item_id, common::ID_1);
    assert_eq!(pr.phase, "prd");
    assert_eq!(pr.result, ResultCode::PhaseComplete);
}

#[tokio::test]
async fn read_result_file_missing_file() {
    let dir = TempDir::new().unwrap();
    let result_path = dir.path().join("nonexistent.json");

    let result = read_result_file(&result_path).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("not found"),
        "Expected 'not found' in: {}",
        err
    );
}

#[tokio::test]
async fn read_result_file_invalid_json() {
    let dir = TempDir::new().unwrap();
    let result_path = dir.path().join("bad.json");
    fs::write(&result_path, "not valid json {{{").unwrap();

    let result = read_result_file(&result_path).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("parse"), "Expected 'parse' in error: {}", err);
}

#[tokio::test]
async fn read_result_file_missing_required_fields() {
    let dir = TempDir::new().unwrap();
    let result_path = dir.path().join("partial.json");
    fs::write(
        &result_path,
        format!(r#"{{"item_id": "{}", "phase": "prd"}}"#, common::ID_1),
    )
    .unwrap();

    let result = read_result_file(&result_path).await;
    assert!(result.is_err(), "Should fail with missing required fields");
}

#[tokio::test]
async fn trusted_result_read_rejects_oversized_files_without_reading_the_full_payload() {
    // Arrange
    let dir = TempDir::new().expect("create temp directory");
    let result_path = dir.path().join("trusted-result.json");
    fs::write(
        &result_path,
        vec![b'x'; MAX_TRUSTED_RESULT_PAYLOAD_BYTES + 4_096],
    )
    .expect("write oversized trusted result");

    // Act
    let result = read_trusted_executor_result_file(&result_path).await;

    // Assert
    assert!(result
        .expect_err("oversized trusted result must be rejected")
        .contains("exceeds the 16384-byte protocol limit"));
}

// --- run_subprocess_agent tests (using mock shell scripts) ---

#[tokio::test]
async fn subprocess_success_writes_valid_result() {
    let dir = TempDir::new().unwrap();
    let result_path = dir.path().join("result.json");
    let script = common::fixtures_dir().join("mock_agent_success.sh");

    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg(&script).arg(&result_path);

    let result = run_subprocess_agent(cmd, &result_path, Duration::from_secs(30)).await;
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result);

    let pr = result.unwrap();
    assert_eq!(pr.item_id, common::ID_1);
    assert_eq!(pr.result, ResultCode::PhaseComplete);

    // Result file should be cleaned up
    assert!(
        !result_path.exists(),
        "Result file should be deleted after read"
    );
}

#[tokio::test]
async fn subprocess_failure_no_result_file() {
    let dir = TempDir::new().unwrap();
    let result_path = dir.path().join("result.json");
    let script = common::fixtures_dir().join("mock_agent_fail.sh");

    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg(&script);

    let result = run_subprocess_agent(cmd, &result_path, Duration::from_secs(30)).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("exit code"),
        "Expected 'exit code' in: {}",
        err
    );
}

#[tokio::test]
async fn subprocess_timeout_kills_process() {
    let dir = TempDir::new().unwrap();
    let result_path = dir.path().join("result.json");
    let script = common::fixtures_dir().join("mock_agent_timeout.sh");

    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg(&script);

    let start = std::time::Instant::now();
    let result = run_subprocess_agent(cmd, &result_path, Duration::from_secs(2)).await;
    let elapsed = start.elapsed();

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("timed out"),
        "Expected 'timed out' in: {}",
        err
    );
    // Should complete in roughly 2s (timeout) + 5s (SIGTERM grace) + margin
    assert!(
        elapsed.as_secs() < 15,
        "Should complete within timeout + kill grace period, took {}s",
        elapsed.as_secs()
    );
}

#[tokio::test]
async fn subprocess_bad_json_returns_error() {
    let dir = TempDir::new().unwrap();
    let result_path = dir.path().join("result.json");
    let script = common::fixtures_dir().join("mock_agent_bad_json.sh");

    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg(&script).arg(&result_path);

    let result = run_subprocess_agent(cmd, &result_path, Duration::from_secs(30)).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("parse"), "Expected 'parse' in: {}", err);
}

#[tokio::test]
async fn subprocess_stale_result_file_cleaned_before_spawn() {
    let dir = TempDir::new().unwrap();
    let result_path = dir.path().join("result.json");

    // Pre-create a stale result file with different content
    fs::write(&result_path, r#"{"stale": true}"#).unwrap();
    assert!(result_path.exists(), "Stale file should exist");

    let script = common::fixtures_dir().join("mock_agent_success.sh");
    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg(&script).arg(&result_path);

    let result = run_subprocess_agent(cmd, &result_path, Duration::from_secs(30)).await;
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result);

    // Should have the new result, not the stale one
    let pr = result.unwrap();
    assert_eq!(pr.item_id, common::ID_1);
}

#[tokio::test]
async fn subprocess_nonzero_exit_with_valid_json_respects_result() {
    let dir = TempDir::new().unwrap();
    let result_path = dir.path().join("result.json");

    // Create a script that writes valid JSON but exits non-zero
    let script_path = dir.path().join("nonzero_with_json.sh");
    let script_content = format!(
        "#!/bin/bash\ncat > \"$1\" << 'HEREDOC'\n{}\nHEREDOC\nexit 1\n",
        valid_result_json()
    );
    fs::write(&script_path, script_content).unwrap();

    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg(&script_path).arg(&result_path);

    let result = run_subprocess_agent(cmd, &result_path, Duration::from_secs(30)).await;
    assert!(
        result.is_ok(),
        "Non-zero exit with valid JSON should succeed: {:?}",
        result
    );
    let pr = result.unwrap();
    assert_eq!(pr.result, ResultCode::PhaseComplete);
}

#[tokio::test]
async fn subprocess_zero_exit_without_result_file_fails() {
    let dir = TempDir::new().unwrap();
    let result_path = dir.path().join("result.json");

    // Script that exits 0 but doesn't write a result file
    let script_path = dir.path().join("zero_no_result.sh");
    fs::write(&script_path, "#!/bin/bash\nexit 0\n").unwrap();

    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg(&script_path);

    let result = run_subprocess_agent(cmd, &result_path, Duration::from_secs(30)).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("zero exit"),
        "Expected 'zero exit' in: {}",
        err
    );
}

// --- MockAgentRunner tests ---

#[tokio::test]
async fn mock_runner_returns_predefined_results_in_order() {
    let results = vec![
        Ok(make_result(ResultCode::PhaseComplete, "Phase 1 done")),
        Ok(make_result(ResultCode::SubphaseComplete, "Subphase done")),
        Err("Simulated failure".to_string()),
    ];

    let mock = MockAgentRunner::new(results);
    let dummy_path = Path::new("/tmp/dummy.json");
    let timeout = Duration::from_secs(30);

    // First call
    let r1 = mock.run_agent("prompt1", dummy_path, timeout).await;
    assert!(r1.is_ok());
    assert_eq!(r1.unwrap().result, ResultCode::PhaseComplete);

    // Second call
    let r2 = mock.run_agent("prompt2", dummy_path, timeout).await;
    assert!(r2.is_ok());
    assert_eq!(r2.unwrap().result, ResultCode::SubphaseComplete);

    // Third call
    let r3 = mock.run_agent("prompt3", dummy_path, timeout).await;
    assert!(r3.is_err());
    assert_eq!(r3.unwrap_err(), "Simulated failure");
}

#[tokio::test]
async fn mock_runner_exhausted_returns_error() {
    let mock = MockAgentRunner::new(vec![Ok(make_result(ResultCode::PhaseComplete, "Done"))]);
    let dummy_path = Path::new("/tmp/dummy.json");
    let timeout = Duration::from_secs(30);

    // Use the one result
    let _ = mock.run_agent("p1", dummy_path, timeout).await;

    // Now exhausted
    let result = mock.run_agent("p2", dummy_path, timeout).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("no more results"));
}

#[tokio::test]
async fn mock_runner_empty_sequence() {
    let mock = MockAgentRunner::new(vec![]);
    let dummy_path = Path::new("/tmp/dummy.json");
    let timeout = Duration::from_secs(30);

    let result = mock.run_agent("prompt", dummy_path, timeout).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("no more results"));
}

// --- Signal handler tests ---

#[test]
fn install_signal_handlers_succeeds() {
    let result = phase_golem::agent::install_signal_handlers();
    assert!(
        result.is_ok(),
        "Signal handler installation should succeed: {:?}",
        result
    );
}

// --- Process group kill tests ---

#[tokio::test]
async fn process_group_kill_cleans_up_subprocess() {
    let dir = TempDir::new().unwrap();
    let result_path = dir.path().join("result.json");

    // Script that spawns a child process (tests process group cleanup)
    let script_path = dir.path().join("parent_child.sh");
    fs::write(&script_path, "#!/bin/bash\nsleep 3600 &\nsleep 3600\n").unwrap();

    let mut cmd = tokio::process::Command::new("bash");
    cmd.arg(&script_path);

    let start = std::time::Instant::now();
    let result = run_subprocess_agent(cmd, &result_path, Duration::from_secs(2)).await;
    let elapsed = start.elapsed();

    assert!(result.is_err());
    assert!(
        result.unwrap_err().contains("timed out"),
        "Should have timed out"
    );
    // Verify it completed in a reasonable time (process group was killed)
    assert!(
        elapsed.as_secs() < 15,
        "Should not hang — process group should be killed, took {}s",
        elapsed.as_secs()
    );
}

#[tokio::test]
async fn shell_verification_timeout_kills_the_process_group() {
    // Arrange
    let dir = TempDir::new().expect("create temp directory");
    let child_pid_path = dir.path().join("child.pid");

    // Act
    let started = std::time::Instant::now();
    let result = run_shell_command(
        &format!(
            "sleep 3600 & printf '%s' $! > '{}' && wait",
            child_pid_path.display()
        ),
        dir.path(),
        Duration::from_secs(1),
    )
    .await;

    // Assert
    assert!(result
        .expect_err("shell verification must time out")
        .contains("timed out"));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "process-group cleanup must remain finite"
    );
}

#[tokio::test]
async fn shell_verification_uses_project_root_and_a_reduced_environment() {
    // Arrange
    let dir = TempDir::new().expect("create temp directory");

    // Act
    let output = run_shell_command(
        "pwd && test -n \"$PATH\" && test -z \"${HOME+x}\"",
        dir.path(),
        Duration::from_secs(2),
    )
    .await;

    // Assert
    let output = output.expect("shell verification command succeeds");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        dir.path().to_string_lossy()
    );
}
