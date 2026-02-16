mod common;

use std::fs;
use std::process::Command;

use tokio_util::sync::CancellationToken;

use orchestrate::agent::MockAgentRunner;
use orchestrate::config::{GuardrailsConfig, PhaseConfig, PipelineConfig, StalenessAction};
use orchestrate::coordinator::spawn_coordinator;
use orchestrate::executor::{
    check_staleness, execute_phase, passes_guardrails, resolve_transition, StalenessResult,
};
use orchestrate::types::{
    BacklogItem, DimensionLevel, ItemStatus, ItemUpdate, PhaseExecutionResult, PhasePool,
    PhaseResult, ResultCode, SizeLevel,
};

// --- Test helpers ---

fn make_feature_item(id: &str, status: ItemStatus) -> BacklogItem {
    let mut item = common::make_item(id, status);
    item.pipeline_type = Some("feature".to_string());
    item
}

fn make_in_progress_item(id: &str, phase: &str) -> BacklogItem {
    let mut item = common::make_in_progress_item(id, phase);
    item.phase_pool = Some(PhasePool::Main);
    item.pipeline_type = Some("feature".to_string());
    item
}

fn make_scoping_item(id: &str, phase: &str) -> BacklogItem {
    let mut item = make_feature_item(id, ItemStatus::Scoping);
    item.phase = Some(phase.to_string());
    item.phase_pool = Some(PhasePool::Pre);
    item
}

fn make_phase_result(item_id: &str, phase: &str, result: ResultCode) -> PhaseResult {
    PhaseResult {
        item_id: item_id.to_string(),
        phase: phase.to_string(),
        result,
        summary: "Test summary".to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: Vec::new(),
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    }
}

fn default_guardrails() -> GuardrailsConfig {
    GuardrailsConfig {
        max_size: SizeLevel::Medium,
        max_complexity: DimensionLevel::Medium,
        max_risk: DimensionLevel::Low,
    }
}

fn make_simple_pipeline() -> PipelineConfig {
    PipelineConfig {
        pre_phases: vec![PhaseConfig {
            workflows: vec![
                ".claude/skills/changes/workflows/orchestration/research-scope.md".to_string(),
            ],
            ..PhaseConfig::new("research", false)
        }],
        phases: vec![
            PhaseConfig {
                workflows: vec![".claude/skills/changes/workflows/0-prd/create-prd.md".to_string()],
                ..PhaseConfig::new("prd", false)
            },
            PhaseConfig {
                workflows: vec![
                    ".claude/skills/changes/workflows/orchestration/build-spec-phase.md"
                        .to_string(),
                ],
                ..PhaseConfig::new("build", true)
            },
            PhaseConfig {
                workflows: vec![
                    ".claude/skills/changes/workflows/5-review/change-review.md".to_string()
                ],
                ..PhaseConfig::new("review", false)
            },
        ],
    }
}

// --- resolve_transition tests ---

#[test]
fn resolve_transition_last_pre_phase_passes_guardrails_promotes_to_ready() {
    let item = make_scoping_item("WRK-001", "research");
    let result = make_phase_result("WRK-001", "research", ResultCode::PhaseComplete);
    let pipeline = make_simple_pipeline();
    let guardrails = default_guardrails();

    let updates = resolve_transition(&item, &result, &pipeline, &guardrails);

    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0], ItemUpdate::ClearPhase);
    assert_eq!(updates[1], ItemUpdate::TransitionStatus(ItemStatus::Ready));
}

#[test]
fn resolve_transition_last_pre_phase_fails_guardrails_blocks() {
    let mut item = make_scoping_item("WRK-001", "research");
    item.size = Some(SizeLevel::Large); // Exceeds max_size: Medium
    let result = make_phase_result("WRK-001", "research", ResultCode::PhaseComplete);
    let pipeline = make_simple_pipeline();
    let guardrails = default_guardrails();

    let updates = resolve_transition(&item, &result, &pipeline, &guardrails);

    assert_eq!(updates.len(), 1);
    match &updates[0] {
        ItemUpdate::SetBlocked(reason) => {
            assert!(reason.contains("guardrail"));
        }
        other => panic!("Expected SetBlocked, got {:?}", other),
    }
}

#[test]
fn resolve_transition_last_pre_phase_requires_human_review_blocks() {
    let mut item = make_scoping_item("WRK-001", "research");
    item.requires_human_review = true;
    let result = make_phase_result("WRK-001", "research", ResultCode::PhaseComplete);
    let pipeline = make_simple_pipeline();
    let guardrails = default_guardrails();

    let updates = resolve_transition(&item, &result, &pipeline, &guardrails);

    assert_eq!(updates.len(), 1);
    match &updates[0] {
        ItemUpdate::SetBlocked(reason) => {
            assert!(reason.contains("human review"));
        }
        other => panic!("Expected SetBlocked, got {:?}", other),
    }
}

#[test]
fn resolve_transition_last_main_phase_transitions_to_done() {
    let item = make_in_progress_item("WRK-001", "review");
    let result = make_phase_result("WRK-001", "review", ResultCode::PhaseComplete);
    let pipeline = make_simple_pipeline();
    let guardrails = default_guardrails();

    let updates = resolve_transition(&item, &result, &pipeline, &guardrails);

    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0], ItemUpdate::TransitionStatus(ItemStatus::Done));
}

#[test]
fn resolve_transition_mid_pipeline_advances_to_next_phase() {
    let item = make_in_progress_item("WRK-001", "prd");
    let result = make_phase_result("WRK-001", "prd", ResultCode::PhaseComplete);
    let pipeline = make_simple_pipeline();
    let guardrails = default_guardrails();

    let updates = resolve_transition(&item, &result, &pipeline, &guardrails);

    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0], ItemUpdate::SetPhase("build".to_string()));
}

#[test]
fn resolve_transition_mid_pipeline_with_commit_sets_last_phase_commit() {
    let item = make_in_progress_item("WRK-001", "prd");
    let mut result = make_phase_result("WRK-001", "prd", ResultCode::PhaseComplete);
    result.based_on_commit = Some("abc123".to_string());
    let pipeline = make_simple_pipeline();
    let guardrails = default_guardrails();

    let updates = resolve_transition(&item, &result, &pipeline, &guardrails);

    assert_eq!(updates.len(), 2);
    assert_eq!(updates[0], ItemUpdate::SetPhase("build".to_string()));
    assert_eq!(
        updates[1],
        ItemUpdate::SetLastPhaseCommit("abc123".to_string())
    );
}

#[test]
fn resolve_transition_mid_main_pipeline_advances_build_to_review() {
    let item = make_in_progress_item("WRK-001", "build");
    let result = make_phase_result("WRK-001", "build", ResultCode::PhaseComplete);
    let pipeline = make_simple_pipeline();
    let guardrails = default_guardrails();

    let updates = resolve_transition(&item, &result, &pipeline, &guardrails);

    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0], ItemUpdate::SetPhase("review".to_string()));
}

#[test]
fn resolve_transition_failed_result_blocks_with_reason() {
    let item = make_in_progress_item("WRK-001", "prd");
    let result = make_phase_result("WRK-001", "prd", ResultCode::Failed);
    let pipeline = make_simple_pipeline();
    let guardrails = default_guardrails();

    let updates = resolve_transition(&item, &result, &pipeline, &guardrails);

    assert_eq!(updates.len(), 1);
    match &updates[0] {
        ItemUpdate::SetBlocked(reason) => {
            assert!(reason.contains("failed"));
            assert!(reason.contains("prd"));
        }
        other => panic!("Expected SetBlocked, got {:?}", other),
    }
}

#[test]
fn resolve_transition_blocked_result_uses_context() {
    let item = make_in_progress_item("WRK-001", "prd");
    let mut result = make_phase_result("WRK-001", "prd", ResultCode::Blocked);
    result.context = Some("Need clarification on requirements".to_string());
    let pipeline = make_simple_pipeline();
    let guardrails = default_guardrails();

    let updates = resolve_transition(&item, &result, &pipeline, &guardrails);

    assert_eq!(updates.len(), 1);
    match &updates[0] {
        ItemUpdate::SetBlocked(reason) => {
            assert_eq!(reason, "Need clarification on requirements");
        }
        other => panic!("Expected SetBlocked, got {:?}", other),
    }
}

#[test]
fn resolve_transition_blocked_without_context_uses_summary() {
    let item = make_in_progress_item("WRK-001", "prd");
    let result = make_phase_result("WRK-001", "prd", ResultCode::Blocked);
    let pipeline = make_simple_pipeline();
    let guardrails = default_guardrails();

    let updates = resolve_transition(&item, &result, &pipeline, &guardrails);

    assert_eq!(updates.len(), 1);
    match &updates[0] {
        ItemUpdate::SetBlocked(reason) => {
            assert_eq!(reason, "Test summary");
        }
        other => panic!("Expected SetBlocked, got {:?}", other),
    }
}

#[test]
fn resolve_transition_subphase_complete_returns_empty() {
    let item = make_in_progress_item("WRK-001", "build");
    let result = make_phase_result("WRK-001", "build", ResultCode::SubphaseComplete);
    let pipeline = make_simple_pipeline();
    let guardrails = default_guardrails();

    let updates = resolve_transition(&item, &result, &pipeline, &guardrails);

    assert!(updates.is_empty());
}

#[test]
fn resolve_transition_no_phase_pool_treats_as_main() {
    let mut item = make_in_progress_item("WRK-001", "prd");
    item.phase_pool = None; // Missing phase_pool
    let result = make_phase_result("WRK-001", "prd", ResultCode::PhaseComplete);
    let pipeline = make_simple_pipeline();
    let guardrails = default_guardrails();

    let updates = resolve_transition(&item, &result, &pipeline, &guardrails);

    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0], ItemUpdate::SetPhase("build".to_string()));
}

// --- passes_guardrails tests ---

#[test]
fn passes_guardrails_all_within_limits() {
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    item.size = Some(SizeLevel::Small);
    item.complexity = Some(DimensionLevel::Low);
    item.risk = Some(DimensionLevel::Low);
    let guardrails = default_guardrails();

    assert!(passes_guardrails(&item, &guardrails));
}

#[test]
fn passes_guardrails_missing_dimensions_pass() {
    let item = make_feature_item("WRK-001", ItemStatus::InProgress);
    let guardrails = default_guardrails();

    assert!(passes_guardrails(&item, &guardrails));
}

#[test]
fn passes_guardrails_size_exceeds() {
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    item.size = Some(SizeLevel::Large);
    let guardrails = default_guardrails(); // max_size: Medium

    assert!(!passes_guardrails(&item, &guardrails));
}

#[test]
fn passes_guardrails_risk_exceeds() {
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    item.risk = Some(DimensionLevel::Medium);
    let guardrails = default_guardrails(); // max_risk: Low

    assert!(!passes_guardrails(&item, &guardrails));
}

#[test]
fn passes_guardrails_complexity_exceeds() {
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    item.complexity = Some(DimensionLevel::High);
    let guardrails = default_guardrails(); // max_complexity: Medium

    assert!(!passes_guardrails(&item, &guardrails));
}

#[test]
fn passes_guardrails_at_exact_limit_passes() {
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    item.size = Some(SizeLevel::Medium);
    item.complexity = Some(DimensionLevel::Medium);
    item.risk = Some(DimensionLevel::Low);
    let guardrails = default_guardrails();

    assert!(passes_guardrails(&item, &guardrails));
}

// --- check_staleness tests ---

#[tokio::test]
async fn check_staleness_no_prior_commit_proceeds() {
    let dir = common::setup_test_env();
    let backlog = common::make_backlog(vec![]);
    let (handle, _coord_task) = spawn_coordinator(
        backlog,
        dir.path().join("BACKLOG.yaml"),
        dir.path().join("BACKLOG_INBOX.yaml"),
        dir.path().to_path_buf(),
        "WRK".to_string(),
    );

    let item = make_in_progress_item("WRK-001", "build");
    let phase_config = PhaseConfig {
        staleness: StalenessAction::Block,
        ..PhaseConfig::new("build", true)
    };

    let result = check_staleness(&item, &phase_config, &handle).await;

    assert_eq!(result, StalenessResult::Proceed);
}

#[tokio::test]
async fn check_staleness_ancestor_commit_proceeds() {
    let dir = common::setup_test_env();

    // Get current HEAD SHA
    let head_sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    // Create another commit so head_sha is an ancestor
    fs::write(dir.path().join("file2.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "file2.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Second commit"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let backlog = common::make_backlog(vec![]);
    let (handle, _coord_task) = spawn_coordinator(
        backlog,
        dir.path().join("BACKLOG.yaml"),
        dir.path().join("BACKLOG_INBOX.yaml"),
        dir.path().to_path_buf(),
        "WRK".to_string(),
    );

    let mut item = make_in_progress_item("WRK-001", "build");
    item.last_phase_commit = Some(head_sha);

    let phase_config = PhaseConfig {
        staleness: StalenessAction::Block,
        ..PhaseConfig::new("build", true)
    };

    let result = check_staleness(&item, &phase_config, &handle).await;

    assert_eq!(result, StalenessResult::Proceed);
}

#[tokio::test]
async fn check_staleness_not_ancestor_with_warn_config_warns() {
    let dir = common::setup_test_env();

    // Get HEAD SHA
    let head_sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    // Create a new branch and diverge, making the old SHA not an ancestor
    Command::new("git")
        .args(["checkout", "--orphan", "new-branch"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    fs::write(dir.path().join("new.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "new.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Orphan commit"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let backlog = common::make_backlog(vec![]);
    let (handle, _coord_task) = spawn_coordinator(
        backlog,
        dir.path().join("BACKLOG.yaml"),
        dir.path().join("BACKLOG_INBOX.yaml"),
        dir.path().to_path_buf(),
        "WRK".to_string(),
    );

    let mut item = make_in_progress_item("WRK-001", "build");
    item.last_phase_commit = Some(head_sha);

    let phase_config = PhaseConfig {
        staleness: StalenessAction::Warn,
        ..PhaseConfig::new("build", true)
    };

    let result = check_staleness(&item, &phase_config, &handle).await;

    assert_eq!(result, StalenessResult::Warn);
}

#[tokio::test]
async fn check_staleness_not_ancestor_with_block_config_blocks() {
    let dir = common::setup_test_env();

    let head_sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    Command::new("git")
        .args(["checkout", "--orphan", "new-branch"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    fs::write(dir.path().join("new.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "new.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Orphan commit"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let backlog = common::make_backlog(vec![]);
    let (handle, _coord_task) = spawn_coordinator(
        backlog,
        dir.path().join("BACKLOG.yaml"),
        dir.path().join("BACKLOG_INBOX.yaml"),
        dir.path().to_path_buf(),
        "WRK".to_string(),
    );

    let mut item = make_in_progress_item("WRK-001", "build");
    item.last_phase_commit = Some(head_sha);

    let phase_config = PhaseConfig {
        staleness: StalenessAction::Block,
        ..PhaseConfig::new("build", true)
    };

    let result = check_staleness(&item, &phase_config, &handle).await;

    match result {
        StalenessResult::Block(reason) => {
            assert!(reason.contains("Stale"));
            assert!(reason.contains("no longer in history"));
        }
        other => panic!("Expected Block, got {:?}", other),
    }
}

#[tokio::test]
async fn check_staleness_not_ancestor_with_ignore_config_proceeds() {
    let dir = common::setup_test_env();

    let head_sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    Command::new("git")
        .args(["checkout", "--orphan", "new-branch"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    fs::write(dir.path().join("new.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "new.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Orphan commit"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let backlog = common::make_backlog(vec![]);
    let (handle, _coord_task) = spawn_coordinator(
        backlog,
        dir.path().join("BACKLOG.yaml"),
        dir.path().join("BACKLOG_INBOX.yaml"),
        dir.path().to_path_buf(),
        "WRK".to_string(),
    );

    let mut item = make_in_progress_item("WRK-001", "build");
    item.last_phase_commit = Some(head_sha);

    let phase_config = PhaseConfig::new("build", true);

    let result = check_staleness(&item, &phase_config, &handle).await;

    assert_eq!(result, StalenessResult::Proceed);
}

#[tokio::test]
async fn check_staleness_unknown_commit_blocks_regardless_of_config() {
    let dir = common::setup_test_env();

    let backlog = common::make_backlog(vec![]);
    let (handle, _coord_task) = spawn_coordinator(
        backlog,
        dir.path().join("BACKLOG.yaml"),
        dir.path().join("BACKLOG_INBOX.yaml"),
        dir.path().to_path_buf(),
        "WRK".to_string(),
    );

    let mut item = make_in_progress_item("WRK-001", "build");
    item.last_phase_commit = Some("deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string());

    let phase_config = PhaseConfig::new("build", true); // Even with ignore, unknown commits block

    let result = check_staleness(&item, &phase_config, &handle).await;

    match result {
        StalenessResult::Block(reason) => {
            assert!(reason.contains("failed"));
        }
        other => panic!("Expected Block, got {:?}", other),
    }
}

// --- execute_phase tests ---

#[tokio::test]
async fn execute_phase_success_returns_success() {
    let dir = common::setup_test_env();
    let item = make_in_progress_item("WRK-001", "prd");
    let backlog = common::make_backlog(vec![item.clone()]);

    orchestrate::backlog::save(&dir.path().join("BACKLOG.yaml"), &backlog).unwrap();

    let (handle, _coord_task) = spawn_coordinator(
        backlog,
        dir.path().join("BACKLOG.yaml"),
        dir.path().join("BACKLOG_INBOX.yaml"),
        dir.path().to_path_buf(),
        "WRK".to_string(),
    );

    let phase_result = PhaseResult {
        item_id: "WRK-001".to_string(),
        phase: "prd".to_string(),
        result: ResultCode::PhaseComplete,
        summary: "PRD created".to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: Vec::new(),
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    };

    let mock = MockAgentRunner::new(vec![Ok(phase_result)]);
    let config = common::default_config();
    let cancel = CancellationToken::new();
    let phase_config = config.pipelines["feature"].phases[0].clone();

    let result = execute_phase(
        &item,
        &phase_config,
        &config,
        &handle,
        &mock,
        &cancel,
        dir.path(),
        None,
    )
    .await;

    match result {
        PhaseExecutionResult::Success(r) => {
            assert_eq!(r.summary, "PRD created");
        }
        other => panic!("Expected Success, got {:?}", other),
    }
}

#[tokio::test]
async fn execute_phase_failure_with_retry_returns_failed_after_exhaustion() {
    let dir = common::setup_test_env();
    let item = make_in_progress_item("WRK-001", "prd");
    let backlog = common::make_backlog(vec![item.clone()]);

    orchestrate::backlog::save(&dir.path().join("BACKLOG.yaml"), &backlog).unwrap();

    let (handle, _coord_task) = spawn_coordinator(
        backlog,
        dir.path().join("BACKLOG.yaml"),
        dir.path().join("BACKLOG_INBOX.yaml"),
        dir.path().to_path_buf(),
        "WRK".to_string(),
    );

    // Config with max_retries: 1 (so 2 total attempts)
    let mut config = common::default_config();
    config.execution.max_retries = 1;

    let fail_result1 = PhaseResult {
        item_id: "WRK-001".to_string(),
        phase: "prd".to_string(),
        result: ResultCode::Failed,
        summary: "First failure".to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: Vec::new(),
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    };
    let fail_result2 = PhaseResult {
        item_id: "WRK-001".to_string(),
        phase: "prd".to_string(),
        result: ResultCode::Failed,
        summary: "Second failure".to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: Vec::new(),
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    };

    let mock = MockAgentRunner::new(vec![Ok(fail_result1), Ok(fail_result2)]);
    let cancel = CancellationToken::new();
    let phase_config = config.pipelines["feature"].phases[0].clone();

    let result = execute_phase(
        &item,
        &phase_config,
        &config,
        &handle,
        &mock,
        &cancel,
        dir.path(),
        None,
    )
    .await;

    match result {
        PhaseExecutionResult::Failed(reason) => {
            assert!(reason.contains("failed after"));
            assert!(reason.contains("Second failure"));
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

#[tokio::test]
async fn execute_phase_subphase_complete_returns_immediately() {
    let dir = common::setup_test_env();
    let item = make_in_progress_item("WRK-001", "build");
    let backlog = common::make_backlog(vec![item.clone()]);

    orchestrate::backlog::save(&dir.path().join("BACKLOG.yaml"), &backlog).unwrap();

    let (handle, _coord_task) = spawn_coordinator(
        backlog,
        dir.path().join("BACKLOG.yaml"),
        dir.path().join("BACKLOG_INBOX.yaml"),
        dir.path().to_path_buf(),
        "WRK".to_string(),
    );

    let subphase_result = PhaseResult {
        item_id: "WRK-001".to_string(),
        phase: "build".to_string(),
        result: ResultCode::SubphaseComplete,
        summary: "Phase 1 of SPEC complete".to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: Vec::new(),
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    };

    let mock = MockAgentRunner::new(vec![Ok(subphase_result)]);
    let config = common::default_config();
    let cancel = CancellationToken::new();
    let phase_config = config.pipelines["feature"].phases[4].clone(); // build phase

    let result = execute_phase(
        &item,
        &phase_config,
        &config,
        &handle,
        &mock,
        &cancel,
        dir.path(),
        None,
    )
    .await;

    match result {
        PhaseExecutionResult::SubphaseComplete(r) => {
            assert_eq!(r.summary, "Phase 1 of SPEC complete");
        }
        other => panic!("Expected SubphaseComplete, got {:?}", other),
    }
}

#[tokio::test]
async fn execute_phase_cancellation_returns_cancelled() {
    let dir = common::setup_test_env();
    let item = make_in_progress_item("WRK-001", "prd");
    let backlog = common::make_backlog(vec![item.clone()]);

    orchestrate::backlog::save(&dir.path().join("BACKLOG.yaml"), &backlog).unwrap();

    let (handle, _coord_task) = spawn_coordinator(
        backlog,
        dir.path().join("BACKLOG.yaml"),
        dir.path().join("BACKLOG_INBOX.yaml"),
        dir.path().to_path_buf(),
        "WRK".to_string(),
    );

    // Mock that never returns (we cancel before it completes)
    let mock = MockAgentRunner::new(vec![]);
    let config = common::default_config();
    let cancel = CancellationToken::new();
    cancel.cancel(); // Cancel immediately

    let phase_config = config.pipelines["feature"].phases[0].clone();

    let result = execute_phase(
        &item,
        &phase_config,
        &config,
        &handle,
        &mock,
        &cancel,
        dir.path(),
        None,
    )
    .await;

    assert_eq!(result, PhaseExecutionResult::Cancelled);
}

#[tokio::test]
async fn execute_phase_blocked_result_returns_blocked() {
    let dir = common::setup_test_env();
    let item = make_in_progress_item("WRK-001", "prd");
    let backlog = common::make_backlog(vec![item.clone()]);

    orchestrate::backlog::save(&dir.path().join("BACKLOG.yaml"), &backlog).unwrap();

    let (handle, _coord_task) = spawn_coordinator(
        backlog,
        dir.path().join("BACKLOG.yaml"),
        dir.path().join("BACKLOG_INBOX.yaml"),
        dir.path().to_path_buf(),
        "WRK".to_string(),
    );

    let blocked_result = PhaseResult {
        item_id: "WRK-001".to_string(),
        phase: "prd".to_string(),
        result: ResultCode::Blocked,
        summary: "Blocked summary".to_string(),
        context: Some("Need human decision".to_string()),
        updated_assessments: None,
        follow_ups: Vec::new(),
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    };

    let mock = MockAgentRunner::new(vec![Ok(blocked_result)]);
    let config = common::default_config();
    let cancel = CancellationToken::new();
    let phase_config = config.pipelines["feature"].phases[0].clone();

    let result = execute_phase(
        &item,
        &phase_config,
        &config,
        &handle,
        &mock,
        &cancel,
        dir.path(),
        None,
    )
    .await;

    match result {
        PhaseExecutionResult::Blocked(reason) => {
            assert_eq!(reason, "Need human decision");
        }
        other => panic!("Expected Blocked, got {:?}", other),
    }
}

#[tokio::test]
async fn execute_phase_agent_error_retries_and_fails() {
    let dir = common::setup_test_env();
    let item = make_in_progress_item("WRK-001", "prd");
    let backlog = common::make_backlog(vec![item.clone()]);

    orchestrate::backlog::save(&dir.path().join("BACKLOG.yaml"), &backlog).unwrap();

    let (handle, _coord_task) = spawn_coordinator(
        backlog,
        dir.path().join("BACKLOG.yaml"),
        dir.path().join("BACKLOG_INBOX.yaml"),
        dir.path().to_path_buf(),
        "WRK".to_string(),
    );

    let mut config = common::default_config();
    config.execution.max_retries = 0; // Only 1 attempt

    let mock = MockAgentRunner::new(vec![Err("Agent crashed".to_string())]);
    let cancel = CancellationToken::new();
    let phase_config = config.pipelines["feature"].phases[0].clone();

    let result = execute_phase(
        &item,
        &phase_config,
        &config,
        &handle,
        &mock,
        &cancel,
        dir.path(),
        None,
    )
    .await;

    match result {
        PhaseExecutionResult::Failed(reason) => {
            assert!(reason.contains("Agent crashed"));
        }
        other => panic!("Expected Failed, got {:?}", other),
    }
}

#[tokio::test]
async fn execute_phase_staleness_blocks_destructive_phase() {
    let dir = common::setup_test_env();

    // Get HEAD SHA, then diverge
    let head_sha = String::from_utf8(
        Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .trim()
    .to_string();

    Command::new("git")
        .args(["checkout", "--orphan", "diverged"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    fs::write(dir.path().join("new.txt"), "diverged").unwrap();
    Command::new("git")
        .args(["add", "new.txt"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Diverge"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let mut item = make_in_progress_item("WRK-001", "build");
    item.last_phase_commit = Some(head_sha);

    let backlog = common::make_backlog(vec![item.clone()]);
    orchestrate::backlog::save(&dir.path().join("BACKLOG.yaml"), &backlog).unwrap();

    let (handle, _coord_task) = spawn_coordinator(
        backlog,
        dir.path().join("BACKLOG.yaml"),
        dir.path().join("BACKLOG_INBOX.yaml"),
        dir.path().to_path_buf(),
        "WRK".to_string(),
    );

    let mock = MockAgentRunner::new(vec![]);
    let mut config = common::default_config();
    // Override the build phase to have staleness: block
    config.pipelines.get_mut("feature").unwrap().phases[4].staleness = StalenessAction::Block;

    let cancel = CancellationToken::new();
    let phase_config = config.pipelines["feature"].phases[4].clone();

    let result = execute_phase(
        &item,
        &phase_config,
        &config,
        &handle,
        &mock,
        &cancel,
        dir.path(),
        None,
    )
    .await;

    match result {
        PhaseExecutionResult::Blocked(reason) => {
            assert!(reason.contains("Stale"));
        }
        other => panic!("Expected Blocked due to staleness, got {:?}", other),
    }
}
