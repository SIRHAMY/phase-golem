mod common;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use phase_golem::agent::MockAgentRunner;
use phase_golem::backlog;
use phase_golem::config::{
    default_feature_pipeline, ExecutionConfig, PhaseConfig, PhaseGolemConfig, PipelineConfig,
};
use phase_golem::coordinator;
use phase_golem::filter;
use phase_golem::scheduler::{
    self, advance_to_next_active_target, select_actions, select_targeted_actions,
    unmet_dep_summary, HaltReason, RunParams, RunningTasks,
};
use phase_golem::types::{
    BacklogFile, BacklogItem, DimensionLevel, FollowUp, ItemStatus, PhasePool, PhaseResult,
    ResultCode, SchedulerAction, SizeLevel, UpdatedAssessments,
};

// --- Test helpers ---

fn backlog_path(root: &Path) -> std::path::PathBuf {
    root.join("BACKLOG.yaml")
}

fn make_item(id: &str, title: &str, status: ItemStatus) -> BacklogItem {
    BacklogItem {
        id: id.to_string(),
        title: title.to_string(),
        status,
        created: "2026-01-01T00:00:00+00:00".to_string(),
        updated: "2026-01-01T00:00:00+00:00".to_string(),
        ..Default::default()
    }
}

fn make_in_progress_item(id: &str, title: &str, phase: &str) -> BacklogItem {
    let mut item = make_item(id, title, ItemStatus::InProgress);
    item.phase = Some(phase.to_string());
    item.phase_pool = Some(PhasePool::Main);
    item
}

fn make_scoping_item(id: &str, title: &str, phase: &str) -> BacklogItem {
    let mut item = make_item(id, title, ItemStatus::Scoping);
    item.phase = Some(phase.to_string());
    item.phase_pool = Some(PhasePool::Pre);
    item
}

fn make_ready_item(id: &str, title: &str, impact: Option<DimensionLevel>) -> BacklogItem {
    let mut item = make_item(id, title, ItemStatus::Ready);
    item.impact = impact;
    item
}

fn default_config() -> PhaseGolemConfig {
    let mut config = PhaseGolemConfig::default();
    if config.pipelines.is_empty() {
        config
            .pipelines
            .insert("feature".to_string(), default_feature_pipeline());
    }
    config
}

fn default_execution_config() -> ExecutionConfig {
    ExecutionConfig {
        phase_timeout_minutes: 30,
        max_retries: 1,
        default_phase_cap: 100,
        max_wip: 2,
        max_concurrent: 3,
    }
}

fn default_pipelines() -> HashMap<String, PipelineConfig> {
    let mut map = HashMap::new();
    map.insert("feature".to_string(), default_feature_pipeline());
    map
}

fn simple_pipeline() -> HashMap<String, PipelineConfig> {
    let mut map = HashMap::new();
    map.insert(
        "feature".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![
                PhaseConfig::new("build", true),
                PhaseConfig::new("review", false),
            ],
        },
    );
    map
}

fn phase_complete_result(item_id: &str, phase: &str) -> PhaseResult {
    PhaseResult {
        item_id: item_id.to_string(),
        phase: phase.to_string(),
        result: ResultCode::PhaseComplete,
        summary: "Phase completed successfully".to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: Vec::new(),
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    }
}

fn failed_result(item_id: &str, phase: &str) -> PhaseResult {
    PhaseResult {
        item_id: item_id.to_string(),
        phase: phase.to_string(),
        result: ResultCode::Failed,
        summary: "Phase failed".to_string(),
        context: Some("Something went wrong".to_string()),
        updated_assessments: None,
        follow_ups: Vec::new(),
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    }
}

fn blocked_result(item_id: &str, phase: &str) -> PhaseResult {
    PhaseResult {
        item_id: item_id.to_string(),
        phase: phase.to_string(),
        result: ResultCode::Blocked,
        summary: "Need human input".to_string(),
        context: Some("Need a decision on approach".to_string()),
        updated_assessments: None,
        follow_ups: Vec::new(),
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    }
}

fn subphase_complete_result(item_id: &str, phase: &str) -> PhaseResult {
    PhaseResult {
        item_id: item_id.to_string(),
        phase: phase.to_string(),
        result: ResultCode::SubphaseComplete,
        summary: "Subphase done".to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: Vec::new(),
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    }
}

fn triage_result_with_assessments(item_id: &str) -> PhaseResult {
    PhaseResult {
        item_id: item_id.to_string(),
        phase: "triage".to_string(),
        result: ResultCode::PhaseComplete,
        summary: "Item triaged".to_string(),
        context: None,
        updated_assessments: Some(UpdatedAssessments {
            size: Some(SizeLevel::Small),
            complexity: Some(DimensionLevel::Low),
            risk: Some(DimensionLevel::Low),
            impact: Some(DimensionLevel::Medium),
        }),
        follow_ups: Vec::new(),
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    }
}

fn make_backlog(items: Vec<BacklogItem>) -> BacklogFile {
    BacklogFile {
        schema_version: 2,
        items,
        next_item_id: 0,
    }
}

fn run_params(root: &Path, target: Option<&str>, cap: u32) -> RunParams {
    RunParams {
        targets: target.map(|s| vec![s.to_string()]).unwrap_or_default(),
        filter: None,
        cap,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    }
}

// ============================================================
// select_actions() unit tests — pure function, no I/O
// ============================================================

#[test]
fn select_actions_empty_backlog_returns_empty() {
    let snapshot = make_backlog(vec![]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);
    assert!(actions.is_empty());
}

#[test]
fn select_actions_all_done_returns_empty() {
    let snapshot = make_backlog(vec![make_item("WRK-001", "Done task", ItemStatus::Done)]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);
    assert!(actions.is_empty());
}

#[test]
fn select_actions_all_blocked_returns_empty() {
    let mut item = make_item("WRK-001", "Blocked task", ItemStatus::Blocked);
    item.blocked_from_status = Some(ItemStatus::InProgress);
    item.blocked_reason = Some("needs input".to_string());

    let snapshot = make_backlog(vec![item]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);
    assert!(actions.is_empty());
}

#[test]
fn select_actions_promotes_ready_items_when_under_max_wip() {
    let snapshot = make_backlog(vec![
        make_ready_item("WRK-001", "Task A", Some(DimensionLevel::High)),
        make_ready_item("WRK-002", "Task B", Some(DimensionLevel::Low)),
    ]);
    let running = RunningTasks::new();
    let config = default_execution_config(); // max_wip=2
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    // Should promote both (max_wip=2, in_progress=0)
    let promotions: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::Promote(_)))
        .collect();
    assert_eq!(promotions.len(), 2);

    // Highest impact first
    assert!(matches!(&actions[0], SchedulerAction::Promote(id) if id == "WRK-001"));
}

#[test]
fn select_actions_respects_max_wip_limit() {
    let snapshot = make_backlog(vec![
        make_in_progress_item("WRK-001", "Running", "prd"),
        make_in_progress_item("WRK-002", "Running 2", "build"),
        make_ready_item(
            "WRK-003",
            "Ready but blocked by WIP",
            Some(DimensionLevel::High),
        ),
    ]);
    let running = RunningTasks::new();
    let config = default_execution_config(); // max_wip=2
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    // Should NOT promote WRK-003 — already at max_wip=2
    let promotions: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::Promote(_)))
        .collect();
    assert_eq!(promotions.len(), 0);
}

#[test]
fn select_actions_in_progress_advance_furthest_first() {
    // WRK-001 at "prd" (index 0), WRK-002 at "spec" (index 3)
    // Should pick WRK-002 first (furthest-first)
    let snapshot = make_backlog(vec![
        make_in_progress_item("WRK-001", "Early task", "prd"),
        make_in_progress_item("WRK-002", "Late task", "spec"),
    ]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    // Filter to RunPhase actions only
    let run_phases: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::RunPhase { .. }))
        .collect();

    assert!(run_phases.len() >= 2);
    // WRK-002 (spec, index 3) should come before WRK-001 (prd, index 0)
    let first_id = match &run_phases[0] {
        SchedulerAction::RunPhase { item_id, .. } => item_id.as_str(),
        _ => "",
    };
    assert_eq!(first_id, "WRK-002");
}

#[test]
fn select_actions_in_progress_before_scoping() {
    // InProgress items should be scheduled before Scoping items
    let snapshot = make_backlog(vec![
        make_scoping_item("WRK-001", "Scoping task", "research"),
        make_in_progress_item("WRK-002", "Active task", "prd"),
    ]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let run_phases: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::RunPhase { .. }))
        .collect();

    assert!(run_phases.len() >= 2);
    // InProgress WRK-002 should come first
    let first_id = match &run_phases[0] {
        SchedulerAction::RunPhase { item_id, .. } => item_id.as_str(),
        _ => "",
    };
    assert_eq!(first_id, "WRK-002");
}

#[test]
fn select_actions_triage_after_phases() {
    // Triage is lowest priority — should come after InProgress phases
    let snapshot = make_backlog(vec![
        make_item("WRK-001", "New item", ItemStatus::New),
        make_in_progress_item("WRK-002", "Active task", "prd"),
    ]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let run_phases: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::RunPhase { .. }))
        .collect();
    let triages: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::Triage(_)))
        .collect();

    assert_eq!(run_phases.len(), 1); // WRK-002 phase
    assert_eq!(triages.len(), 1); // WRK-001 triage

    // RunPhase should appear before Triage
    let first_phase_pos = actions
        .iter()
        .position(|a| matches!(a, SchedulerAction::RunPhase { .. }))
        .unwrap();
    let triage_pos = actions
        .iter()
        .position(|a| matches!(a, SchedulerAction::Triage(_)))
        .unwrap();
    assert!(first_phase_pos < triage_pos);
}

#[test]
fn select_actions_destructive_phase_is_exclusive() {
    // An item at "build" (destructive) should block all other phases
    let snapshot = make_backlog(vec![
        make_in_progress_item("WRK-001", "Build task", "build"),
        make_in_progress_item("WRK-002", "Other task", "prd"),
    ]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let run_phases: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::RunPhase { .. }))
        .collect();

    // "build" (destructive, furthest-first at index 4) should be picked
    // and it should be the ONLY phase action
    assert_eq!(run_phases.len(), 1);
    let phase_id = match &run_phases[0] {
        SchedulerAction::RunPhase { item_id, .. } => item_id.as_str(),
        _ => "",
    };
    assert_eq!(phase_id, "WRK-001");
}

#[test]
fn select_actions_destructive_running_blocks_all() {
    let snapshot = make_backlog(vec![
        make_in_progress_item("WRK-001", "Build running", "build"),
        make_in_progress_item("WRK-002", "Other task", "prd"),
    ]);
    let mut running = RunningTasks::new();
    running.insert_destructive("WRK-001", "build");
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    // Nothing should be scheduled while destructive is running
    let run_phases: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                SchedulerAction::RunPhase { .. } | SchedulerAction::Triage(_)
            )
        })
        .collect();
    assert_eq!(run_phases.len(), 0);
}

#[test]
fn select_actions_respects_max_concurrent() {
    // With max_concurrent=1, only one phase action
    let snapshot = make_backlog(vec![
        make_in_progress_item("WRK-001", "Task A", "prd"),
        make_in_progress_item("WRK-002", "Task B", "spec"),
    ]);
    let running = RunningTasks::new();
    let config = ExecutionConfig {
        max_concurrent: 1,
        max_wip: 5,
        ..default_execution_config()
    };
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let executor_actions: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                SchedulerAction::RunPhase { .. } | SchedulerAction::Triage(_)
            )
        })
        .collect();
    assert_eq!(executor_actions.len(), 1);
}

#[test]
fn select_actions_skips_already_running_items() {
    let snapshot = make_backlog(vec![
        make_in_progress_item("WRK-001", "Running task", "prd"),
        make_in_progress_item("WRK-002", "Idle task", "spec"),
    ]);
    let mut running = RunningTasks::new();
    running.insert_non_destructive("WRK-001", "prd");
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let run_phases: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::RunPhase { .. }))
        .collect();

    // WRK-001 is already running, so only WRK-002 should be scheduled
    assert_eq!(run_phases.len(), 1);
    let scheduled_id = match &run_phases[0] {
        SchedulerAction::RunPhase { item_id, .. } => item_id.as_str(),
        _ => "",
    };
    assert_eq!(scheduled_id, "WRK-002");
}

#[test]
fn select_actions_new_items_trigger_triage() {
    let snapshot = make_backlog(vec![
        make_item("WRK-001", "New task 1", ItemStatus::New),
        make_item("WRK-002", "New task 2", ItemStatus::New),
    ]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let triages: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::Triage(_)))
        .collect();

    assert_eq!(triages.len(), 2);
}

#[test]
fn select_actions_promotion_tiebreaks_by_impact() {
    let snapshot = make_backlog(vec![
        make_ready_item("WRK-001", "Low impact", Some(DimensionLevel::Low)),
        make_ready_item("WRK-002", "High impact", Some(DimensionLevel::High)),
        make_ready_item("WRK-003", "Medium impact", Some(DimensionLevel::Medium)),
    ]);
    let running = RunningTasks::new();
    let config = ExecutionConfig {
        max_wip: 3,
        ..default_execution_config()
    };
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let promotions: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            SchedulerAction::Promote(id) => Some(id.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(promotions.len(), 3);
    assert_eq!(promotions[0], "WRK-002"); // High
    assert_eq!(promotions[1], "WRK-003"); // Medium
    assert_eq!(promotions[2], "WRK-001"); // Low
}

#[test]
fn select_actions_no_destructive_when_non_destructive_running() {
    // build (destructive) should NOT be scheduled if non-destructive tasks are already running
    let snapshot = make_backlog(vec![make_in_progress_item(
        "WRK-001",
        "Build task",
        "build",
    )]);
    let mut running = RunningTasks::new();
    running.insert_non_destructive("WRK-099", "prd"); // something else running
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    // Destructive can't run while non-destructive is active
    let run_phases: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::RunPhase { .. }))
        .collect();
    assert_eq!(run_phases.len(), 0);
}

#[test]
fn select_actions_scoping_items_with_pre_phases() {
    let snapshot = make_backlog(vec![make_scoping_item(
        "WRK-001",
        "Scoping item",
        "research",
    )]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let run_phases: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::RunPhase { .. }))
        .collect();

    assert_eq!(run_phases.len(), 1);
    match &run_phases[0] {
        SchedulerAction::RunPhase {
            item_id,
            phase,
            phase_pool,
            ..
        } => {
            assert_eq!(item_id, "WRK-001");
            assert_eq!(phase, "research");
            assert_eq!(phase_pool, &PhasePool::Pre);
        }
        _ => panic!("Expected RunPhase"),
    }
}

// ============================================================
// Integration tests with coordinator + mock agent
// ============================================================

#[tokio::test]
async fn scheduler_happy_path_single_item_all_phases() {
    let dir = common::setup_test_env();
    let root = dir.path();
    let config = default_config();
    let pipelines = simple_pipeline();

    let item = make_in_progress_item("WRK-001", "Test feature", "build");
    let backlog = common::make_backlog(vec![item]);

    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
    ]);

    let mut config_with_simple = config.clone();
    config_with_simple.pipelines = pipelines;

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary = scheduler::run_scheduler(
        coordinator_handle,
        Arc::new(runner),
        config_with_simple,
        params,
        cancel,
    )
    .await
    .expect("Scheduler should succeed");

    assert_eq!(summary.items_completed, vec!["WRK-001"]);
    assert!(summary.items_blocked.is_empty());
    assert_eq!(summary.halt_reason, HaltReason::AllDoneOrBlocked);
}

#[tokio::test]
async fn scheduler_blocked_result_blocks_item() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![Ok(blocked_result("WRK-001", "build"))]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert!(summary.items_completed.is_empty());
    assert_eq!(summary.items_blocked, vec!["WRK-001"]);
    assert_eq!(summary.halt_reason, HaltReason::AllDoneOrBlocked);
}

#[tokio::test]
async fn scheduler_retry_then_success() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    // First attempt fails, second succeeds (within executor retry)
    // max_retries=1 means 2 attempts total
    let runner = MockAgentRunner::new(vec![
        Ok(failed_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();
    config.execution.max_retries = 1;

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.items_completed, vec!["WRK-001"]);
}

#[tokio::test]
async fn scheduler_retry_exhaustion_blocks_item() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    // Two consecutive failures exhausts retries (max_retries=1)
    let runner = MockAgentRunner::new(vec![
        Ok(failed_result("WRK-001", "build")),
        Ok(failed_result("WRK-001", "build")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();
    config.execution.max_retries = 1;

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert!(summary.items_completed.is_empty());
    assert_eq!(summary.items_blocked, vec!["WRK-001"]);
}

#[tokio::test]
async fn scheduler_cap_limits_phase_execution() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-001", "build")),
        // review would need another result, but cap=1 stops after build
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 1); // cap=1

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.halt_reason, HaltReason::CapReached);
    assert_eq!(summary.phases_executed, 1);
}

#[tokio::test]
async fn scheduler_no_actionable_items_exits() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let backlog = common::make_backlog(vec![make_item("WRK-001", "Done item", ItemStatus::Done)]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![]);

    let config = default_config();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.halt_reason, HaltReason::AllDoneOrBlocked);
    assert_eq!(summary.phases_executed, 0);
}

#[tokio::test]
async fn scheduler_target_mode_completes_specific_item() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, Some("WRK-001"), 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.items_completed, vec!["WRK-001"]);
    assert_eq!(summary.halt_reason, HaltReason::TargetCompleted);
}

#[tokio::test]
async fn scheduler_subphase_complete_re_executes_phase() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    // First invocation returns SubphaseComplete, second returns PhaseComplete
    let runner = MockAgentRunner::new(vec![
        Ok(subphase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.items_completed, vec!["WRK-001"]);
}

#[tokio::test]
async fn scheduler_follow_ups_are_ingested() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let mut result = phase_complete_result("WRK-001", "build");
    result.follow_ups = vec![FollowUp {
        title: "Follow-up task".to_string(),
        context: Some("A new task from follow-up".to_string()),
        suggested_size: None,
        suggested_risk: None,
    }];

    let runner = MockAgentRunner::new(vec![
        Ok(result),
        Ok(phase_complete_result("WRK-001", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert!(summary.follow_ups_created >= 1);
}

// ============================================================
// Triage integration tests
// ============================================================

#[tokio::test]
async fn triage_small_low_risk_promotes_to_ready() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_item("WRK-001", "Small fix", ItemStatus::New);
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let config = default_config();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let triage_result = triage_result_with_assessments("WRK-001");
    scheduler::apply_triage_result(&coordinator_handle, "WRK-001", &triage_result, &config)
        .await
        .expect("apply_triage_result should succeed");

    let snapshot = coordinator_handle.get_snapshot().await.unwrap();
    let item = snapshot.items.iter().find(|i| i.id == "WRK-001").unwrap();

    // Small + Low risk → should be Ready
    assert_eq!(item.status, ItemStatus::Ready);
    assert_eq!(item.size, Some(SizeLevel::Small));
    assert_eq!(item.risk, Some(DimensionLevel::Low));
}

#[tokio::test]
async fn triage_large_item_goes_to_scoping_with_pre_phase() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_item("WRK-001", "Big feature", ItemStatus::New);
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let config = default_config();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let mut triage_result = triage_result_with_assessments("WRK-001");
    triage_result.updated_assessments = Some(UpdatedAssessments {
        size: Some(SizeLevel::Large),
        complexity: Some(DimensionLevel::High),
        risk: Some(DimensionLevel::High),
        impact: Some(DimensionLevel::High),
    });

    scheduler::apply_triage_result(&coordinator_handle, "WRK-001", &triage_result, &config)
        .await
        .expect("apply_triage_result should succeed");

    let snapshot = coordinator_handle.get_snapshot().await.unwrap();
    let item = snapshot.items.iter().find(|i| i.id == "WRK-001").unwrap();

    // Large + High risk → should be Scoping with first pre_phase
    assert_eq!(item.status, ItemStatus::Scoping);
    assert_eq!(item.phase, Some("research".to_string()));
    assert_eq!(item.phase_pool, Some(PhasePool::Pre));
}

#[tokio::test]
async fn triage_blocked_result_blocks_item() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_item("WRK-001", "Unclear item", ItemStatus::New);
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let config = default_config();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let triage_result = blocked_result("WRK-001", "triage");
    scheduler::apply_triage_result(&coordinator_handle, "WRK-001", &triage_result, &config)
        .await
        .expect("apply_triage_result should succeed");

    let snapshot = coordinator_handle.get_snapshot().await.unwrap();
    let item = snapshot.items.iter().find(|i| i.id == "WRK-001").unwrap();

    assert_eq!(item.status, ItemStatus::Blocked);
}

#[tokio::test]
async fn triage_with_invalid_pipeline_type_blocks() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_item("WRK-001", "Item", ItemStatus::New);
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let config = default_config();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let mut triage_result = triage_result_with_assessments("WRK-001");
    triage_result.pipeline_type = Some("nonexistent_pipeline".to_string());

    scheduler::apply_triage_result(&coordinator_handle, "WRK-001", &triage_result, &config)
        .await
        .expect("apply_triage_result should succeed");

    let snapshot = coordinator_handle.get_snapshot().await.unwrap();
    let item = snapshot.items.iter().find(|i| i.id == "WRK-001").unwrap();

    assert_eq!(item.status, ItemStatus::Blocked);
    assert!(item
        .blocked_reason
        .as_ref()
        .unwrap()
        .contains("nonexistent_pipeline"));
}

// ============================================================
// Destructive starvation prevention tests
// ============================================================

#[test]
fn select_actions_destructive_pending_blocks_new_non_destructive() {
    // WRK-001 at "build" (destructive, furthest along — highest priority)
    // WRK-002 at "prd" (non-destructive, lower priority)
    // WRK-099 is running non-destructive
    //
    // Previously, `continue` would skip the destructive action and let WRK-002's
    // prd phase leak through, starving the destructive action indefinitely.
    // With `break`, zero RunPhase actions are scheduled — running tasks drain
    // naturally, and the destructive action runs on the next iteration.
    let snapshot = make_backlog(vec![
        make_in_progress_item("WRK-001", "Build task", "build"),
        make_in_progress_item("WRK-002", "PRD task", "prd"),
    ]);
    let mut running = RunningTasks::new();
    running.insert_non_destructive("WRK-099", "prd");
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let run_phases: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::RunPhase { .. }))
        .collect();
    assert_eq!(
        run_phases.len(),
        0,
        "No phases should be scheduled when destructive is pending but can't run"
    );
}

#[test]
fn select_actions_destructive_pending_blocks_triage() {
    // Same starvation scenario but with a New item that could be triaged.
    // The destructive build can't run (non-destructive in flight), so `break`
    // should prevent the triage from being scheduled too.
    let snapshot = make_backlog(vec![
        make_in_progress_item("WRK-001", "Build task", "build"),
        make_item("WRK-002", "New item", ItemStatus::New),
    ]);
    let mut running = RunningTasks::new();
    running.insert_non_destructive("WRK-099", "prd");
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let executor_actions: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                SchedulerAction::RunPhase { .. } | SchedulerAction::Triage(_)
            )
        })
        .collect();
    assert_eq!(
        executor_actions.len(),
        0,
        "No executor actions should be scheduled when destructive is pending but can't run"
    );
}

// ============================================================
// Circuit breaker test
// ============================================================

#[tokio::test]
async fn scheduler_circuit_breaker_trips_after_consecutive_exhaustions() {
    let dir = common::setup_test_env();
    let root = dir.path();

    // Two items that will both exhaust retries (0 retries = 1 attempt each)
    let mut item1 = make_in_progress_item("WRK-001", "Feature 1", "build");
    item1.created = "2026-01-01T00:00:00+00:00".to_string();
    let mut item2 = make_in_progress_item("WRK-002", "Feature 2", "build");
    item2.created = "2026-01-02T00:00:00+00:00".to_string();

    let backlog = common::make_backlog(vec![item1, item2]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    // Both items fail — 2 consecutive exhaustions trips the breaker
    let runner = MockAgentRunner::new(vec![
        Ok(failed_result("WRK-001", "build")),
        Ok(failed_result("WRK-002", "build")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();
    config.execution.max_retries = 0; // 1 attempt only
    config.execution.max_concurrent = 1; // One at a time to guarantee order

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.halt_reason, HaltReason::CircuitBreakerTripped);
}

// ============================================================
// Dependency filtering tests — select_actions()
// ============================================================

#[test]
fn test_ready_item_with_unmet_dep_not_promoted() {
    let mut item_a = make_ready_item("WRK-001", "Depends on WRK-002", Some(DimensionLevel::High));
    item_a.dependencies = vec!["WRK-002".to_string()];
    let item_b = make_item("WRK-002", "Dependency", ItemStatus::Ready);

    let snapshot = make_backlog(vec![item_a, item_b]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let promotions: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            SchedulerAction::Promote(id) => Some(id.clone()),
            _ => None,
        })
        .collect();

    // WRK-001 should NOT be promoted (dep WRK-002 is Ready, not Done)
    assert!(
        !promotions.contains(&"WRK-001".to_string()),
        "Item with unmet dep should not be promoted"
    );
}

#[test]
fn test_ready_item_with_met_dep_promoted() {
    let mut item_a = make_ready_item("WRK-001", "Depends on WRK-002", Some(DimensionLevel::High));
    item_a.dependencies = vec!["WRK-002".to_string()];
    let item_b = make_item("WRK-002", "Done dependency", ItemStatus::Done);

    let snapshot = make_backlog(vec![item_a, item_b]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let promotions: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            SchedulerAction::Promote(id) => Some(id.clone()),
            _ => None,
        })
        .collect();

    assert!(
        promotions.contains(&"WRK-001".to_string()),
        "Item with met dep should be promoted"
    );
}

#[test]
fn test_ready_item_with_absent_dep_promoted() {
    let mut item_a = make_ready_item(
        "WRK-001",
        "Depends on archived item",
        Some(DimensionLevel::High),
    );
    item_a.dependencies = vec!["WRK-ARCHIVED".to_string()];

    let snapshot = make_backlog(vec![item_a]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let promotions: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            SchedulerAction::Promote(id) => Some(id.clone()),
            _ => None,
        })
        .collect();

    assert!(
        promotions.contains(&"WRK-001".to_string()),
        "Item with absent dep should be promoted (absent = archived = met)"
    );
}

#[test]
fn test_ready_item_with_partial_deps_not_promoted() {
    let mut item_a = make_ready_item("WRK-001", "Depends on A and B", Some(DimensionLevel::High));
    item_a.dependencies = vec!["WRK-002".to_string(), "WRK-003".to_string()];
    let item_b = make_item("WRK-002", "Done dep", ItemStatus::Done);
    let item_c = make_item("WRK-003", "Still Ready dep", ItemStatus::Ready);

    let snapshot = make_backlog(vec![item_a, item_b, item_c]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let promotions: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            SchedulerAction::Promote(id) => Some(id.clone()),
            _ => None,
        })
        .collect();

    assert!(
        !promotions.contains(&"WRK-001".to_string()),
        "Item with partially met deps should not be promoted"
    );
}

#[test]
fn test_ready_item_with_blocked_dep_not_promoted() {
    let mut item_a = make_ready_item("WRK-001", "Depends on blocked", Some(DimensionLevel::High));
    item_a.dependencies = vec!["WRK-002".to_string()];
    let mut item_b = make_item("WRK-002", "Blocked dep", ItemStatus::Blocked);
    item_b.blocked_from_status = Some(ItemStatus::InProgress);
    item_b.blocked_reason = Some("needs input".to_string());

    let snapshot = make_backlog(vec![item_a, item_b]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let promotions: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            SchedulerAction::Promote(id) => Some(id.clone()),
            _ => None,
        })
        .collect();

    assert!(
        !promotions.contains(&"WRK-001".to_string()),
        "Item with Blocked dep should not be promoted"
    );
}

#[test]
fn test_ready_item_with_in_progress_dep_not_promoted() {
    let mut item_a = make_ready_item(
        "WRK-001",
        "Depends on in-progress",
        Some(DimensionLevel::High),
    );
    item_a.dependencies = vec!["WRK-002".to_string()];
    let item_b = make_in_progress_item("WRK-002", "In-progress dep", "build");

    let snapshot = make_backlog(vec![item_a, item_b]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let promotions: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            SchedulerAction::Promote(id) => Some(id.clone()),
            _ => None,
        })
        .collect();

    assert!(
        !promotions.contains(&"WRK-001".to_string()),
        "Item with InProgress dep should not be promoted"
    );
}

#[test]
fn test_in_progress_with_unmet_dep_no_phase_action() {
    let mut item_a = make_in_progress_item("WRK-001", "Has unmet dep", "build");
    item_a.dependencies = vec!["WRK-002".to_string()];
    let item_b = make_item("WRK-002", "Still Ready", ItemStatus::Ready);

    let snapshot = make_backlog(vec![item_a, item_b]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let run_phases: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::RunPhase { item_id, .. } if item_id == "WRK-001"))
        .collect();

    assert!(
        run_phases.is_empty(),
        "InProgress item with unmet dep should not get RunPhase action"
    );
}

#[test]
fn test_in_progress_with_met_dep_gets_phase_action() {
    let mut item_a = make_in_progress_item("WRK-001", "Has met dep", "build");
    item_a.dependencies = vec!["WRK-002".to_string()];
    let item_b = make_item("WRK-002", "Done dep", ItemStatus::Done);

    let snapshot = make_backlog(vec![item_a, item_b]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let run_phases: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::RunPhase { item_id, .. } if item_id == "WRK-001"))
        .collect();

    assert_eq!(
        run_phases.len(),
        1,
        "InProgress item with met dep should get RunPhase action"
    );
}

#[test]
fn test_scoping_with_unmet_dep_no_phase_action() {
    let mut item_a = make_scoping_item("WRK-001", "Has unmet dep", "research");
    item_a.dependencies = vec!["WRK-002".to_string()];
    let item_b = make_item("WRK-002", "Still Ready", ItemStatus::Ready);

    let snapshot = make_backlog(vec![item_a, item_b]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let run_phases: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::RunPhase { item_id, .. } if item_id == "WRK-001"))
        .collect();

    assert!(
        run_phases.is_empty(),
        "Scoping item with unmet dep should not get RunPhase action"
    );
}

#[test]
fn test_new_item_with_unmet_dep_not_triaged() {
    let mut item_a = make_item("WRK-001", "New with unmet dep", ItemStatus::New);
    item_a.dependencies = vec!["WRK-002".to_string()];
    let item_b = make_item("WRK-002", "Still Ready", ItemStatus::Ready);

    let snapshot = make_backlog(vec![item_a, item_b]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let triages: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::Triage(id) if id == "WRK-001"))
        .collect();

    assert!(
        triages.is_empty(),
        "New item with unmet dep should not be triaged"
    );
}

#[test]
fn test_new_item_with_met_dep_triaged() {
    let mut item_a = make_item("WRK-001", "New with met dep", ItemStatus::New);
    item_a.dependencies = vec!["WRK-002".to_string()];
    let item_b = make_item("WRK-002", "Done dep", ItemStatus::Done);

    let snapshot = make_backlog(vec![item_a, item_b]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let triages: Vec<&SchedulerAction> = actions
        .iter()
        .filter(|a| matches!(a, SchedulerAction::Triage(id) if id == "WRK-001"))
        .collect();

    assert_eq!(triages.len(), 1, "New item with met dep should be triaged");
}

#[test]
fn test_no_deps_scheduled_normally() {
    let item_a = make_ready_item("WRK-001", "No deps", Some(DimensionLevel::High));

    let snapshot = make_backlog(vec![item_a]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let promotions: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            SchedulerAction::Promote(id) => Some(id.clone()),
            _ => None,
        })
        .collect();

    assert!(
        promotions.contains(&"WRK-001".to_string()),
        "Item with no deps should be scheduled normally"
    );
}

#[test]
fn test_unmet_dep_does_not_consume_wip_slot() {
    // max_wip=1, two Ready items: WRK-001 has unmet dep, WRK-002 doesn't
    // WRK-001 should be skipped and WRK-002 should be promoted
    let mut item_a = make_ready_item("WRK-001", "Has unmet dep", Some(DimensionLevel::High));
    item_a.dependencies = vec!["WRK-003".to_string()];
    let item_b = make_ready_item("WRK-002", "No deps", Some(DimensionLevel::Low));
    let item_c = make_item("WRK-003", "Scoping dep", ItemStatus::Scoping);

    let snapshot = make_backlog(vec![item_a, item_b, item_c]);
    let running = RunningTasks::new();
    let config = ExecutionConfig {
        max_wip: 1,
        ..default_execution_config()
    };
    let pipelines = default_pipelines();

    let actions = select_actions(&snapshot, &running, &config, &pipelines);

    let promotions: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            SchedulerAction::Promote(id) => Some(id.clone()),
            _ => None,
        })
        .collect();

    assert_eq!(promotions.len(), 1, "Exactly one item should be promoted");
    assert_eq!(
        promotions[0], "WRK-002",
        "Item without unmet deps should be promoted, not the one with unmet deps"
    );
}

// ============================================================
// Dependency filtering tests — select_targeted_actions()
// ============================================================

#[test]
fn test_targeted_with_unmet_dep_returns_empty() {
    let mut item_a = make_in_progress_item("WRK-001", "Target with unmet dep", "build");
    item_a.dependencies = vec!["WRK-002".to_string()];
    let item_b = make_item("WRK-002", "Still Ready", ItemStatus::Ready);

    let snapshot = make_backlog(vec![item_a, item_b]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_targeted_actions(&snapshot, &running, &config, &pipelines, "WRK-001");

    assert!(
        actions.is_empty(),
        "Targeted item with unmet dep should return empty actions"
    );
}

#[test]
fn test_targeted_with_met_dep_returns_action() {
    let mut item_a = make_in_progress_item("WRK-001", "Target with met dep", "build");
    item_a.dependencies = vec!["WRK-002".to_string()];
    let item_b = make_item("WRK-002", "Done dep", ItemStatus::Done);

    let snapshot = make_backlog(vec![item_a, item_b]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_targeted_actions(&snapshot, &running, &config, &pipelines, "WRK-001");

    assert!(
        !actions.is_empty(),
        "Targeted item with met dep should return actions"
    );
}

#[test]
fn test_targeted_with_absent_dep_returns_action() {
    let mut item_a = make_in_progress_item("WRK-001", "Target with absent dep", "build");
    item_a.dependencies = vec!["WRK-ARCHIVED".to_string()];

    let snapshot = make_backlog(vec![item_a]);
    let running = RunningTasks::new();
    let config = default_execution_config();
    let pipelines = default_pipelines();

    let actions = select_targeted_actions(&snapshot, &running, &config, &pipelines, "WRK-001");

    assert!(
        !actions.is_empty(),
        "Targeted item with absent dep should return actions (absent = archived = met)"
    );
}

// ============================================================
// unmet_dep_summary() unit tests
// ============================================================

#[test]
fn test_unmet_dep_summary_no_unmet_deps() {
    // All deps are Done → None
    let mut item = make_item("WRK-001", "Item", ItemStatus::Ready);
    item.dependencies = vec!["WRK-002".to_string()];
    let dep = make_item("WRK-002", "Done dep", ItemStatus::Done);

    let result = unmet_dep_summary(&item, &[item.clone(), dep]);
    assert_eq!(result, None, "No unmet deps should return None");
}

#[test]
fn test_unmet_dep_summary_single_unmet_dep() {
    let mut item = make_item("WRK-001", "Item", ItemStatus::Ready);
    item.dependencies = vec!["WRK-002".to_string()];
    let dep = make_item("WRK-002", "Ready dep", ItemStatus::Ready);

    let result = unmet_dep_summary(&item, &[item.clone(), dep]);
    let summary = result.expect("Should return Some for unmet deps");
    assert!(
        summary.contains("WRK-002"),
        "Should contain the unmet dep ID"
    );
    assert!(summary.contains("Ready"), "Should contain the dep status");
}

#[test]
fn test_unmet_dep_summary_multiple_unmet_deps() {
    let mut item = make_item("WRK-001", "Item", ItemStatus::Ready);
    item.dependencies = vec!["WRK-002".to_string(), "WRK-003".to_string()];
    let dep_a = make_item("WRK-002", "Ready dep", ItemStatus::Ready);
    let dep_b = make_in_progress_item("WRK-003", "InProgress dep", "build");

    let result = unmet_dep_summary(&item, &[item.clone(), dep_a, dep_b]);
    let summary = result.expect("Should return Some for unmet deps");
    assert!(
        summary.contains("WRK-002"),
        "Should contain first unmet dep"
    );
    assert!(
        summary.contains("WRK-003"),
        "Should contain second unmet dep"
    );
    assert!(
        summary.contains(", "),
        "Multiple deps should be comma-separated"
    );
}

#[test]
fn test_unmet_dep_summary_mix_of_met_and_unmet() {
    let mut item = make_item("WRK-001", "Item", ItemStatus::Ready);
    item.dependencies = vec![
        "WRK-002".to_string(),
        "WRK-003".to_string(),
        "WRK-004".to_string(),
    ];
    let dep_done = make_item("WRK-002", "Done dep", ItemStatus::Done);
    let dep_ready = make_item("WRK-003", "Ready dep", ItemStatus::Ready);
    // WRK-004 is absent (not in the list) → met

    let result = unmet_dep_summary(&item, &[item.clone(), dep_done, dep_ready]);
    let summary = result.expect("Should return Some for unmet deps");
    assert!(!summary.contains("WRK-002"), "Done dep should not appear");
    assert!(!summary.contains("WRK-004"), "Absent dep should not appear");
    assert!(summary.contains("WRK-003"), "Unmet Ready dep should appear");
    assert!(
        !summary.contains(", "),
        "Only one unmet dep so no comma separator"
    );
}

// ============================================================
// advance_to_next_active_target() unit tests
// ============================================================

#[test]
fn test_advance_skips_done_targets() {
    let done_item = make_item("WRK-001", "Done target", ItemStatus::Done);
    let active_item = make_in_progress_item("WRK-002", "Active target", "build");
    let snapshot = make_backlog(vec![done_item, active_item]);

    let result = advance_to_next_active_target(
        &["WRK-001".to_string(), "WRK-002".to_string()],
        0,
        &[],
        &snapshot,
    );
    assert_eq!(
        result, 1,
        "Should skip Done WRK-001 and return index 1 for active WRK-002"
    );
}

#[test]
fn test_advance_skips_archived_targets() {
    // WRK-001 not in snapshot (archived), WRK-002 is active
    let active_item = make_in_progress_item("WRK-002", "Active target", "build");
    let snapshot = make_backlog(vec![active_item]);

    let result = advance_to_next_active_target(
        &["WRK-001".to_string(), "WRK-002".to_string()],
        0,
        &[],
        &snapshot,
    );
    assert_eq!(result, 1, "Should skip archived WRK-001 and return index 1");
}

#[test]
fn test_advance_all_exhausted() {
    let done_item = make_item("WRK-001", "Done", ItemStatus::Done);
    let snapshot = make_backlog(vec![done_item]);

    let result = advance_to_next_active_target(
        &["WRK-001".to_string(), "WRK-099".to_string()],
        0,
        &[],
        &snapshot,
    );
    assert!(
        result >= 2,
        "Should return index >= len when all targets exhausted"
    );
}

#[test]
fn test_advance_first_is_active() {
    let active_item = make_in_progress_item("WRK-001", "Active", "build");
    let snapshot = make_backlog(vec![active_item]);

    let result = advance_to_next_active_target(&["WRK-001".to_string()], 0, &[], &snapshot);
    assert_eq!(result, 0, "Should return 0 when first target is active");
}

#[test]
fn test_advance_mixed_states() {
    let done_item = make_item("WRK-001", "Done", ItemStatus::Done);
    // WRK-002 not in snapshot (archived)
    let active_item = make_in_progress_item("WRK-003", "Active", "build");
    let snapshot = make_backlog(vec![done_item, active_item]);

    let result = advance_to_next_active_target(
        &[
            "WRK-001".to_string(),
            "WRK-002".to_string(),
            "WRK-003".to_string(),
        ],
        0,
        &[],
        &snapshot,
    );
    assert_eq!(result, 2, "Should skip Done and archived, return index 2");
}

#[test]
fn test_advance_empty_targets() {
    let snapshot = make_backlog(vec![]);

    let result = advance_to_next_active_target(&[], 0, &[], &snapshot);
    assert_eq!(
        result, 0,
        "Empty targets should return 0 (immediately >= len)"
    );
}

#[test]
fn test_advance_skips_pre_blocked_targets() {
    let mut blocked_item = make_item("WRK-001", "Pre-blocked target", ItemStatus::Blocked);
    blocked_item.blocked_from_status = Some(ItemStatus::InProgress);
    blocked_item.blocked_reason = Some("needs input".to_string());
    let active_item = make_in_progress_item("WRK-002", "Active target", "build");
    let snapshot = make_backlog(vec![blocked_item, active_item]);

    let result = advance_to_next_active_target(
        &["WRK-001".to_string(), "WRK-002".to_string()],
        0,
        &[],
        &snapshot,
    );
    assert_eq!(
        result, 1,
        "Should skip pre-Blocked WRK-001 and return index 1 for active WRK-002"
    );
}

#[test]
fn test_advance_skips_items_in_completed_list() {
    // WRK-001 is in items_completed but still InProgress in snapshot (race condition)
    let item = make_in_progress_item("WRK-001", "Completed via items_completed", "build");
    let active_item = make_in_progress_item("WRK-002", "Active", "build");
    let snapshot = make_backlog(vec![item, active_item]);

    let result = advance_to_next_active_target(
        &["WRK-001".to_string(), "WRK-002".to_string()],
        0,
        &["WRK-001".to_string()],
        &snapshot,
    );
    assert_eq!(result, 1, "Should skip WRK-001 that's in items_completed");
}

// ============================================================
// Multi-target integration tests
// ============================================================

#[tokio::test]
async fn test_multi_target_processes_in_order() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item1 = make_in_progress_item("WRK-001", "First", "build");
    let item2 = make_in_progress_item("WRK-002", "Second", "build");
    let backlog = common::make_backlog(vec![item1, item2]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
        Ok(phase_complete_result("WRK-002", "build")),
        Ok(phase_complete_result("WRK-002", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec!["WRK-001".to_string(), "WRK-002".to_string()],
        filter: None,
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert!(summary.items_completed.contains(&"WRK-001".to_string()));
    assert!(summary.items_completed.contains(&"WRK-002".to_string()));
    assert_eq!(summary.halt_reason, HaltReason::TargetCompleted);
}

#[tokio::test]
async fn test_multi_target_halts_on_block() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item1 = make_in_progress_item("WRK-001", "First", "build");
    let item2 = make_in_progress_item("WRK-002", "Second", "build");
    let backlog = common::make_backlog(vec![item1, item2]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
        Ok(blocked_result("WRK-002", "build")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec!["WRK-001".to_string(), "WRK-002".to_string()],
        filter: None,
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert!(summary.items_completed.contains(&"WRK-001".to_string()));
    assert!(summary.items_blocked.contains(&"WRK-002".to_string()));
    assert_eq!(summary.halt_reason, HaltReason::TargetBlocked);
}

#[tokio::test]
async fn test_multi_target_all_done_at_startup() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item1 = make_item("WRK-001", "Done 1", ItemStatus::Done);
    let item2 = make_item("WRK-002", "Done 2", ItemStatus::Done);
    let backlog = common::make_backlog(vec![item1, item2]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec!["WRK-001".to_string(), "WRK-002".to_string()],
        filter: None,
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.halt_reason, HaltReason::TargetCompleted);
    assert_eq!(summary.phases_executed, 0);
}

#[tokio::test]
async fn test_multi_target_skips_done_targets() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item1 = make_item("WRK-001", "Already done", ItemStatus::Done);
    let item2 = make_in_progress_item("WRK-002", "Active", "build");
    let backlog = common::make_backlog(vec![item1, item2]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-002", "build")),
        Ok(phase_complete_result("WRK-002", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec!["WRK-001".to_string(), "WRK-002".to_string()],
        filter: None,
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert!(summary.items_completed.contains(&"WRK-002".to_string()));
    assert_eq!(summary.halt_reason, HaltReason::TargetCompleted);
}

#[tokio::test]
async fn test_multi_target_single_element_backward_compat() {
    // Single target in Vec should behave identically to pre-change behavior
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec!["WRK-001".to_string()],
        filter: None,
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.items_completed, vec!["WRK-001"]);
    assert_eq!(summary.halt_reason, HaltReason::TargetCompleted);
}

#[tokio::test]
async fn test_multi_target_target_archived_during_run() {
    // Target not in snapshot should be skipped (treated as done/archived)
    let dir = common::setup_test_env();
    let root = dir.path();

    // Only WRK-002 in backlog; WRK-001 is "archived" (not present)
    let item2 = make_in_progress_item("WRK-002", "Active", "build");
    let backlog = common::make_backlog(vec![item2]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-002", "build")),
        Ok(phase_complete_result("WRK-002", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec!["WRK-001".to_string(), "WRK-002".to_string()],
        filter: None,
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert!(summary.items_completed.contains(&"WRK-002".to_string()));
    assert_eq!(summary.halt_reason, HaltReason::TargetCompleted);
}

#[tokio::test]
async fn test_multi_target_skips_pre_blocked_targets() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let mut blocked_item = make_item("WRK-001", "Pre-blocked", ItemStatus::Blocked);
    blocked_item.blocked_from_status = Some(ItemStatus::InProgress);
    blocked_item.blocked_reason = Some("needs input".to_string());
    let item2 = make_in_progress_item("WRK-002", "Active", "build");
    let backlog = common::make_backlog(vec![blocked_item, item2]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-002", "build")),
        Ok(phase_complete_result("WRK-002", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec!["WRK-001".to_string(), "WRK-002".to_string()],
        filter: None,
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert!(summary.items_completed.contains(&"WRK-002".to_string()));
    assert_eq!(summary.halt_reason, HaltReason::TargetCompleted);
}

// ============================================================
// Filter scheduling tests
// ============================================================

#[tokio::test]
async fn test_filter_restricts_scheduler_to_matching_items() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let mut high_item = make_in_progress_item("WRK-001", "High impact", "build");
    high_item.impact = Some(DimensionLevel::High);
    let mut low_item = make_in_progress_item("WRK-002", "Low impact", "build");
    low_item.impact = Some(DimensionLevel::Low);
    let backlog = common::make_backlog(vec![high_item, low_item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec![],
        filter: Some(filter::parse_filter("impact=high").unwrap()),
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    // Only WRK-001 (high impact) should be processed
    assert!(summary.items_completed.contains(&"WRK-001".to_string()));
    assert!(!summary.items_completed.contains(&"WRK-002".to_string()));
    assert_eq!(summary.halt_reason, HaltReason::FilterExhausted);
}

#[tokio::test]
async fn test_filter_no_matching_items_halts() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let mut low_item = make_in_progress_item("WRK-001", "Low impact", "build");
    low_item.impact = Some(DimensionLevel::Low);
    let backlog = common::make_backlog(vec![low_item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec![],
        filter: Some(filter::parse_filter("impact=high").unwrap()),
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.halt_reason, HaltReason::NoMatchingItems);
    assert_eq!(summary.phases_executed, 0);
}

#[tokio::test]
async fn test_filter_all_exhausted_halts() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let mut done_item = make_item("WRK-001", "Done high impact", ItemStatus::Done);
    done_item.impact = Some(DimensionLevel::High);
    let backlog = common::make_backlog(vec![done_item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec![],
        filter: Some(filter::parse_filter("impact=high").unwrap()),
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.halt_reason, HaltReason::FilterExhausted);
    assert_eq!(summary.phases_executed, 0);
}

// ============================================================
// Phase 4: End-to-end integration tests
// ============================================================

#[tokio::test]
async fn test_integration_single_target_backward_compat() {
    // Single target in Vec should behave identically to pre-change behavior:
    // same halt reason, items_completed, and action sequence.
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec!["WRK-001".to_string()],
        filter: None,
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.items_completed, vec!["WRK-001"]);
    assert!(summary.items_blocked.is_empty());
    assert_eq!(summary.halt_reason, HaltReason::TargetCompleted);
    // All phases of the single target should have been executed
    assert!(
        summary.phases_executed >= 2,
        "Both build and review phases should execute"
    );
}

#[tokio::test]
async fn test_integration_multi_target_sequential() {
    // Three targets processed in order, all completing successfully
    let dir = common::setup_test_env();
    let root = dir.path();

    let item1 = make_in_progress_item("WRK-001", "First", "build");
    let item2 = make_in_progress_item("WRK-002", "Second", "build");
    let item3 = make_in_progress_item("WRK-003", "Third", "build");
    let backlog = common::make_backlog(vec![item1, item2, item3]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
        Ok(phase_complete_result("WRK-002", "build")),
        Ok(phase_complete_result("WRK-002", "review")),
        Ok(phase_complete_result("WRK-003", "build")),
        Ok(phase_complete_result("WRK-003", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec![
            "WRK-001".to_string(),
            "WRK-002".to_string(),
            "WRK-003".to_string(),
        ],
        filter: None,
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    // All three items completed in order
    assert!(summary.items_completed.contains(&"WRK-001".to_string()));
    assert!(summary.items_completed.contains(&"WRK-002".to_string()));
    assert!(summary.items_completed.contains(&"WRK-003".to_string()));
    assert_eq!(summary.halt_reason, HaltReason::TargetCompleted);
    assert!(
        summary.phases_executed >= 6,
        "All 6 phases should execute (2 per item × 3 items)"
    );
}

#[tokio::test]
async fn test_integration_multi_target_with_block() {
    // First target completes, second target blocks → halt
    let dir = common::setup_test_env();
    let root = dir.path();

    let item1 = make_in_progress_item("WRK-001", "First", "build");
    let item2 = make_in_progress_item("WRK-002", "Second (will block)", "build");
    let backlog = common::make_backlog(vec![item1, item2]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
        Ok(blocked_result("WRK-002", "build")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec!["WRK-001".to_string(), "WRK-002".to_string()],
        filter: None,
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert!(summary.items_completed.contains(&"WRK-001".to_string()));
    assert!(summary.items_blocked.contains(&"WRK-002".to_string()));
    assert_eq!(summary.halt_reason, HaltReason::TargetBlocked);
}

#[tokio::test]
async fn test_integration_filter_impact_high() {
    // Filter with impact=high processes only high-impact items, ignores others
    let dir = common::setup_test_env();
    let root = dir.path();

    let mut high1 = make_in_progress_item("WRK-001", "High impact 1", "build");
    high1.impact = Some(DimensionLevel::High);
    let mut high2 = make_in_progress_item("WRK-002", "High impact 2", "build");
    high2.impact = Some(DimensionLevel::High);
    let mut medium_item = make_in_progress_item("WRK-003", "Medium impact", "build");
    medium_item.impact = Some(DimensionLevel::Medium);
    let mut low_item = make_in_progress_item("WRK-004", "Low impact", "build");
    low_item.impact = Some(DimensionLevel::Low);
    let backlog = common::make_backlog(vec![high1, high2, medium_item, low_item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
        Ok(phase_complete_result("WRK-002", "build")),
        Ok(phase_complete_result("WRK-002", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec![],
        filter: Some(filter::parse_filter("impact=high").unwrap()),
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    // Only WRK-001 and WRK-002 (high impact) should be processed
    assert!(summary.items_completed.contains(&"WRK-001".to_string()));
    assert!(summary.items_completed.contains(&"WRK-002".to_string()));
    assert!(!summary.items_completed.contains(&"WRK-003".to_string()));
    assert!(!summary.items_completed.contains(&"WRK-004".to_string()));
    assert_eq!(summary.halt_reason, HaltReason::FilterExhausted);
}

#[tokio::test]
async fn test_integration_filter_no_matches() {
    // Filter with no matching items halts immediately with NoMatchingItems
    let dir = common::setup_test_env();
    let root = dir.path();

    let mut item1 = make_in_progress_item("WRK-001", "Medium impact", "build");
    item1.impact = Some(DimensionLevel::Medium);
    let mut item2 = make_in_progress_item("WRK-002", "Low impact", "build");
    item2.impact = Some(DimensionLevel::Low);
    let backlog = common::make_backlog(vec![item1, item2]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = RunParams {
        targets: vec![],
        filter: Some(filter::parse_filter("impact=high").unwrap()),
        cap: 100,
        root: root.to_path_buf(),
        config_base: root.to_path_buf(),
    };

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.halt_reason, HaltReason::NoMatchingItems);
    assert_eq!(summary.phases_executed, 0);
    assert!(summary.items_completed.is_empty());
}

// ============================================================
// previous_summaries cleanup tests (WRK-022)
// ============================================================

#[test]
fn cleanup_terminal_summary_removes_entry_and_noop_for_missing() {
    let mut summaries: HashMap<String, String> = HashMap::new();
    summaries.insert("WRK-001".to_string(), "Phase completed".to_string());
    summaries.insert("WRK-002".to_string(), "Another phase".to_string());

    // Remove existing entry
    summaries.remove("WRK-001");
    assert!(!summaries.contains_key("WRK-001"));
    assert!(summaries.contains_key("WRK-002"));
    assert_eq!(summaries.len(), 1);

    // No-op for missing entry
    summaries.remove("WRK-999");
    assert_eq!(summaries.len(), 1);
}

#[tokio::test]
async fn cleanup_done_via_handle_phase_success() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.items_completed, vec!["WRK-001"]);
    assert!(summary.items_blocked.is_empty());
    assert_eq!(summary.halt_reason, HaltReason::AllDoneOrBlocked);
}

#[tokio::test]
async fn cleanup_blocked_via_handle_phase_failed() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    // Single attempt (max_retries=0) that fails → handle_phase_failed
    let runner = MockAgentRunner::new(vec![Ok(failed_result("WRK-001", "build"))]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();
    config.execution.max_retries = 0;
    config.execution.max_concurrent = 1;

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert!(summary.items_completed.is_empty());
    assert_eq!(summary.items_blocked, vec!["WRK-001"]);
}

#[tokio::test]
async fn cleanup_blocked_via_handle_phase_blocked() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![Ok(blocked_result("WRK-001", "build"))]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert!(summary.items_completed.is_empty());
    assert_eq!(summary.items_blocked, vec!["WRK-001"]);
    assert_eq!(summary.halt_reason, HaltReason::AllDoneOrBlocked);
}

#[tokio::test]
async fn non_terminal_phase_retains_summary() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    // Subphase → phase complete → review complete
    // The summary from subphase should persist for the next build invocation
    let runner = MockAgentRunner::new(vec![
        Ok(subphase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.items_completed, vec!["WRK-001"]);
    assert_eq!(summary.phases_executed, 3);
}

#[tokio::test]
async fn many_items_complete_with_bounded_summaries() {
    let dir = common::setup_test_env();
    let root = dir.path();

    // 4 items, max_wip=2 — items processed in batches
    let item1 = make_in_progress_item("WRK-001", "Feature 1", "build");
    let item2 = make_in_progress_item("WRK-002", "Feature 2", "build");
    let item3 = make_in_progress_item("WRK-003", "Feature 3", "build");
    let item4 = make_in_progress_item("WRK-004", "Feature 4", "build");
    let backlog = common::make_backlog(vec![item1, item2, item3, item4]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    let runner = MockAgentRunner::new(vec![
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
        Ok(phase_complete_result("WRK-002", "build")),
        Ok(phase_complete_result("WRK-002", "review")),
        Ok(phase_complete_result("WRK-003", "build")),
        Ok(phase_complete_result("WRK-003", "review")),
        Ok(phase_complete_result("WRK-004", "build")),
        Ok(phase_complete_result("WRK-004", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();
    config.execution.max_wip = 2;
    config.execution.max_concurrent = 1;

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.items_completed.len(), 4);
    assert!(summary.items_blocked.is_empty());
    assert_eq!(summary.halt_reason, HaltReason::AllDoneOrBlocked);
}

#[tokio::test]
async fn retry_then_success_summary_persists() {
    let dir = common::setup_test_env();
    let root = dir.path();

    let item = make_in_progress_item("WRK-001", "Feature", "build");
    let backlog = common::make_backlog(vec![item]);
    backlog::save(&backlog_path(root), &backlog).unwrap();

    // First attempt fails, second succeeds (executor retries internally)
    let runner = MockAgentRunner::new(vec![
        Ok(failed_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "build")),
        Ok(phase_complete_result("WRK-001", "review")),
    ]);

    let mut config = default_config();
    config.pipelines = simple_pipeline();
    config.execution.max_retries = 1;

    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_path(root),
        root.join("BACKLOG_INBOX.yaml"),
        root.to_path_buf(),
        "WRK".to_string(),
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let params = run_params(root, None, 100);

    let summary =
        scheduler::run_scheduler(coordinator_handle, Arc::new(runner), config, params, cancel)
            .await
            .expect("Scheduler should succeed");

    assert_eq!(summary.items_completed, vec!["WRK-001"]);
    assert!(summary.items_blocked.is_empty());
}
