mod common;

use std::collections::{BTreeMap, VecDeque};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use phase_golem::agent::{TrustedExecutionRequest, TrustedExecutorAdapter};
use phase_golem::config::{
    PhaseGolemConfig, PublicExecutionPolicy, TrustedExecutorProfile, VerificationPlan,
};
use phase_golem::coordinator::spawn_coordinator;
use phase_golem::executor::DeterministicVerifier;
use phase_golem::pg_item::{
    X_PG_EXECUTION_POLICY, X_PG_EXECUTOR_PROFILE, X_PG_HUMAN_DECISION, X_PG_OWNER,
    X_PG_TEMPLATE_NODE_KEY, X_PG_VERIFICATION,
};
use phase_golem::scheduler::{
    run_foreground_supervisor, run_scheduled_wakeup, HaltReason, RunParams,
};
use phase_golem::types::TrustedResultCode;
use task_golem::model::item::Item;
use task_golem::model::status::Status;
use tokio_util::sync::CancellationToken;

const PROFILE: &str = "test-executor";
const PHASE: &str = "build";

struct FakeAdapter {
    responses: Mutex<VecDeque<Result<String, String>>>,
    active_invocations: Arc<AtomicUsize>,
    peak_invocations: Arc<AtomicUsize>,
    invoked_item_ids: Arc<Mutex<Vec<String>>>,
    execution_started: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    discovery_created: Mutex<Option<tokio::sync::oneshot::Receiver<String>>>,
    discovered_item_id: Arc<Mutex<Option<String>>>,
}

impl FakeAdapter {
    fn new(responses: Vec<Result<String, String>>) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            active_invocations: Arc::new(AtomicUsize::new(0)),
            peak_invocations: Arc::new(AtomicUsize::new(0)),
            invoked_item_ids: Arc::new(Mutex::new(Vec::new())),
            execution_started: None,
            discovery_created: Mutex::new(None),
            discovered_item_id: Arc::new(Mutex::new(None)),
        }
    }

    fn with_discovery_harness(
        mut self,
        execution_started: tokio::sync::mpsc::UnboundedSender<String>,
        discovery_created: tokio::sync::oneshot::Receiver<String>,
    ) -> Self {
        self.execution_started = Some(execution_started);
        self.discovery_created = Mutex::new(Some(discovery_created));
        self
    }

    fn peak_invocations(&self) -> usize {
        self.peak_invocations.load(Ordering::SeqCst)
    }

    fn invoked_item_ids(&self) -> Vec<String> {
        self.invoked_item_ids
            .lock()
            .expect("invoked item IDs lock")
            .clone()
    }
}

impl TrustedExecutorAdapter for FakeAdapter {
    fn invoke(
        &self,
        _profile: &TrustedExecutorProfile,
        request: &TrustedExecutionRequest,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send {
        let discovery_created = self
            .discovery_created
            .lock()
            .expect("discovery completion lock")
            .take();
        if discovery_created.is_some() {
            let execution_started = self
                .execution_started
                .as_ref()
                .expect("discovery harness is configured");
            execution_started
                .send(request.item_id.clone())
                .expect("discovery harness receives execution start");
        }

        let response = if self.execution_started.is_some() {
            Ok(result_payload(
                &request.item_id,
                TrustedResultCode::Complete,
            ))
        } else {
            self.responses
                .lock()
                .expect("responses lock")
                .pop_front()
                .expect("configured executor response")
        };
        let active_invocations = self.active_invocations.clone();
        let peak_invocations = self.peak_invocations.clone();
        let invoked_item_ids = self.invoked_item_ids.clone();
        let discovered_item_id = self.discovered_item_id.clone();
        let request_id = request.item_id.clone();

        async move {
            let response = if let Some(discovery_created) = discovery_created {
                let discovered_id = discovery_created
                    .await
                    .map_err(|_| "discovery harness stopped before creating work".to_string())?;
                *discovered_item_id.lock().expect("discovered item ID lock") = Some(discovered_id);
                response
            } else if response
                .as_ref()
                .is_ok_and(|payload| payload.contains(common::ID_3))
            {
                let discovered_id = discovered_item_id
                    .lock()
                    .expect("discovered item ID lock")
                    .clone()
                    .expect("discovered item ID is available");
                Ok(result_payload(&discovered_id, TrustedResultCode::Complete))
            } else {
                response
            };
            let executing = active_invocations.fetch_add(1, Ordering::SeqCst) + 1;
            peak_invocations.fetch_max(executing, Ordering::SeqCst);
            invoked_item_ids
                .lock()
                .expect("invoked item IDs lock")
                .push(request_id);
            tokio::task::yield_now().await;
            active_invocations.fetch_sub(1, Ordering::SeqCst);
            response
        }
    }
}

struct PassingVerifier;

impl DeterministicVerifier for PassingVerifier {
    async fn verify(
        &self,
        _plan: &VerificationPlan,
        _request: &TrustedExecutionRequest,
        _executor_evidence: &[String],
    ) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }
}

struct BlockingAdapter {
    execution_started: tokio::sync::mpsc::UnboundedSender<String>,
}

struct BlockingVerifier {
    verification_started: tokio::sync::mpsc::UnboundedSender<String>,
}

struct CancelAfterVerification {
    cancellation: CancellationToken,
}

impl DeterministicVerifier for BlockingVerifier {
    fn verify(
        &self,
        _plan: &VerificationPlan,
        request: &TrustedExecutionRequest,
        _executor_evidence: &[String],
    ) -> impl std::future::Future<Output = Result<Vec<String>, String>> + Send {
        self.verification_started
            .send(request.item_id.clone())
            .expect("shutdown test receives verification start");
        std::future::pending()
    }
}

impl DeterministicVerifier for CancelAfterVerification {
    async fn verify(
        &self,
        _plan: &VerificationPlan,
        _request: &TrustedExecutionRequest,
        _executor_evidence: &[String],
    ) -> Result<Vec<String>, String> {
        let cancellation = self.cancellation.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            cancellation.cancel();
        });
        Ok(Vec::new())
    }
}

impl TrustedExecutorAdapter for BlockingAdapter {
    fn invoke(
        &self,
        _profile: &TrustedExecutorProfile,
        request: &TrustedExecutionRequest,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send {
        self.execution_started
            .send(request.item_id.clone())
            .expect("shutdown test receives execution start");
        std::future::pending()
    }
}

#[tokio::test]
async fn foreground_loop_drains_ready_work_sequentially_and_discovers_new_work() {
    // Arrange
    let directory = common::setup_test_env();
    initialize_task_golem_project(directory.path());
    let store = common::setup_task_golem_store(directory.path());
    let first = materialized_item(common::ID_1, Vec::new(), false);
    let second = materialized_item(common::ID_2, vec![common::ID_1.to_string()], false);
    store
        .save_active(&[first, second])
        .expect("seed workflow work");
    commit_task_golem_seed(directory.path());
    let (coordinator, _task) = spawn_coordinator(store.clone(), directory.path().to_path_buf());
    let (execution_started, mut execution_starts) = tokio::sync::mpsc::unbounded_channel();
    let (discovery_created, discovery_complete) = tokio::sync::oneshot::channel();
    let adapter =
        FakeAdapter::new(Vec::new()).with_discovery_harness(execution_started, discovery_complete);
    let discovery_root = directory.path().to_path_buf();
    let discovery_harness = tokio::spawn(async move {
        execution_starts
            .recv()
            .await
            .expect("trusted adapter starts first execution");
        let discovered_id = tokio::task::spawn_blocking(move || {
            create_discovered_item_through_tg_crud(&discovery_root)
        })
        .await
        .expect("discovery harness task")
        .expect("create discovered work through TG CRUD");
        discovery_created
            .send(discovered_id.clone())
            .expect("trusted adapter waits for discovered work");
        discovered_id
    });

    // Act
    let summary = run_foreground_supervisor(
        coordinator,
        &config(),
        &run_params(10),
        &CancellationToken::new(),
        &adapter,
        &PassingVerifier,
    )
    .await
    .expect("foreground supervisor result");
    let discovered_id = discovery_harness
        .await
        .expect("discovery harness completes");

    // Assert
    assert_eq!(summary.halt_reason, HaltReason::SelectedScopeComplete);
    assert_eq!(summary.tasks_executed, 3);
    assert_eq!(adapter.peak_invocations(), 1);
    assert_eq!(
        adapter.invoked_item_ids(),
        vec![common::ID_1, common::ID_2, discovered_id.as_str()]
    );
    assert!(store.load_active().expect("load active work").is_empty());
    assert_eq!(
        store.load_all_archive().expect("load completed work").len(),
        3
    );
}

#[tokio::test]
async fn foreground_loop_stops_for_idle_human_gate_budget_and_shutdown() {
    for (name, item, cap, cancelled, expected) in [
        ("idle", None, 1, false, HaltReason::Idle),
        (
            "human gate",
            Some(materialized_item(common::ID_1, Vec::new(), true)),
            1,
            false,
            HaltReason::ReadyHumanGate,
        ),
        (
            "budget",
            Some(materialized_item(common::ID_1, Vec::new(), false)),
            0,
            false,
            HaltReason::BudgetReached,
        ),
        (
            "shutdown",
            Some(materialized_item(common::ID_1, Vec::new(), false)),
            1,
            true,
            HaltReason::ShutdownRequested,
        ),
    ] {
        // Arrange
        let directory = common::setup_test_env();
        let store = common::setup_task_golem_store(directory.path());
        if let Some(item) = item {
            store.save_active(&[item]).expect("seed item");
        }
        let (coordinator, _task) = spawn_coordinator(store.clone(), directory.path().to_path_buf());
        let cancellation = CancellationToken::new();
        if cancelled {
            cancellation.cancel();
        }
        let adapter = FakeAdapter::new(Vec::new());

        // Act
        let summary = run_foreground_supervisor(
            coordinator,
            &config(),
            &run_params(cap),
            &cancellation,
            &adapter,
            &PassingVerifier,
        )
        .await
        .expect("foreground supervisor result");

        // Assert
        assert_eq!(summary.halt_reason, expected, "{name}");
        assert_eq!(adapter.peak_invocations(), 0, "{name}");
    }
}

#[tokio::test]
async fn foreground_selection_executes_before_a_later_ready_human_gate() {
    // Arrange
    let directory = common::setup_test_env();
    let store = common::setup_task_golem_store(directory.path());
    store
        .save_active(&[
            materialized_item(common::ID_1, Vec::new(), false),
            materialized_item(common::ID_2, Vec::new(), true),
        ])
        .expect("seed executable work and a gate");
    let (coordinator, _task) = spawn_coordinator(store.clone(), directory.path().to_path_buf());
    let adapter = FakeAdapter::new(vec![Ok(result_payload(
        common::ID_1,
        TrustedResultCode::Complete,
    ))]);

    // Act
    let summary = run_foreground_supervisor(
        coordinator,
        &config(),
        &run_params(10),
        &CancellationToken::new(),
        &adapter,
        &PassingVerifier,
    )
    .await
    .expect("foreground supervisor result");

    // Assert
    assert_eq!(summary.halt_reason, HaltReason::ReadyHumanGate);
    assert_eq!(summary.items_completed, vec![common::ID_1]);
    let gate = store.load_active().expect("load gate").remove(0);
    assert_eq!(gate.id, common::ID_2);
    assert_eq!(gate.status, Status::Todo);
    assert!(gate.claimed_by.is_none());
}

#[tokio::test]
async fn unrecoverable_execution_failure_blocks_the_claimed_task_and_stops() {
    // Arrange
    let directory = common::setup_test_env();
    let store = common::setup_task_golem_store(directory.path());
    store
        .save_active(&[materialized_item(common::ID_1, Vec::new(), false)])
        .expect("seed work");
    let (coordinator, _task) = spawn_coordinator(store.clone(), directory.path().to_path_buf());
    let adapter = FakeAdapter::new(vec![Err("executor unavailable".to_string())]);

    // Act
    let summary = run_foreground_supervisor(
        coordinator,
        &config(),
        &run_params(1),
        &CancellationToken::new(),
        &adapter,
        &PassingVerifier,
    )
    .await
    .expect("foreground supervisor result");

    // Assert
    assert_eq!(summary.halt_reason, HaltReason::UnrecoverableFailure);
    assert_eq!(summary.items_blocked, vec![common::ID_1]);
    let blocked = store.load_active().expect("load blocked work").remove(0);
    assert_eq!(blocked.status, Status::Blocked);
    assert!(blocked
        .blocked_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("Unrecoverable supervised execution failure")));
}

#[tokio::test]
async fn preclaim_supervision_failure_leaves_the_task_unattempted_and_stops() {
    // Arrange
    let directory = common::setup_test_env();
    let store = common::setup_task_golem_store(directory.path());
    store
        .save_active(&[materialized_item(common::ID_1, Vec::new(), false)])
        .expect("seed work");
    let (coordinator, _task) = spawn_coordinator(store.clone(), directory.path().to_path_buf());
    let mut config = config();
    config.executor_profiles.clear();
    let adapter = FakeAdapter::new(Vec::new());

    // Act
    let summary = run_foreground_supervisor(
        coordinator,
        &config,
        &run_params(1),
        &CancellationToken::new(),
        &adapter,
        &PassingVerifier,
    )
    .await
    .expect("foreground supervisor result");

    // Assert
    assert_eq!(summary.halt_reason, HaltReason::UnrecoverableFailure);
    assert_eq!(summary.tasks_executed, 0);
    assert!(summary.items_blocked.is_empty());
    let item = store
        .load_active()
        .expect("load unattempted work")
        .remove(0);
    assert_eq!(item.status, Status::Todo);
    assert!(item.claimed_by.is_none());
    assert_eq!(adapter.peak_invocations(), 0);
}

#[tokio::test]
async fn targeted_run_does_not_report_success_for_unselected_or_incomplete_targets() {
    for (name, item, cap) in [
        (
            "non-PG target",
            {
                let mut item = materialized_item(common::ID_1, Vec::new(), false);
                item.extensions.remove(X_PG_OWNER);
                item
            },
            1,
        ),
        (
            "preclaimed target",
            {
                let mut item = materialized_item(common::ID_1, Vec::new(), false);
                item.status = Status::Doing;
                item.claimed_by = Some("another-process".to_string());
                item.claimed_at = Some(item.updated_at);
                item
            },
            1,
        ),
        (
            "budget-limited target",
            materialized_item(common::ID_1, Vec::new(), false),
            0,
        ),
    ] {
        // Arrange
        let directory = common::setup_test_env();
        let store = common::setup_task_golem_store(directory.path());
        store.save_active(&[item]).expect("seed targeted item");
        let (coordinator, _task) = spawn_coordinator(store, directory.path().to_path_buf());
        let adapter = FakeAdapter::new(Vec::new());

        // Act
        let summary = run_foreground_supervisor(
            coordinator,
            &config(),
            &targeted_run_params(common::ID_1, cap),
            &CancellationToken::new(),
            &adapter,
            &PassingVerifier,
        )
        .await
        .expect("foreground supervisor result");

        // Assert
        assert_ne!(
            summary.halt_reason,
            HaltReason::SelectedScopeComplete,
            "{name} must not be reported as complete"
        );
        assert_eq!(summary.tasks_executed, 0, "{name}");
    }
}

#[tokio::test]
async fn targeted_scope_completes_a_child_without_rolling_up_or_touching_its_parent() {
    // Arrange
    let directory = common::setup_test_env();
    let store = common::setup_task_golem_store(directory.path());
    let parent = materialized_item(common::ID_2, Vec::new(), false);
    let mut child = materialized_item(common::ID_1, Vec::new(), false);
    child.parent = Some(common::ID_2.to_string());
    store
        .save_active(&[parent, child])
        .expect("seed parent and child work");
    let (coordinator, _task) = spawn_coordinator(store.clone(), directory.path().to_path_buf());
    let adapter = FakeAdapter::new(vec![Ok(result_payload(
        common::ID_1,
        TrustedResultCode::Complete,
    ))]);

    // Act
    let summary = run_foreground_supervisor(
        coordinator,
        &config(),
        &targeted_run_params(common::ID_1, 10),
        &CancellationToken::new(),
        &adapter,
        &PassingVerifier,
    )
    .await
    .expect("foreground supervisor result");

    // Assert
    assert_eq!(summary.halt_reason, HaltReason::SelectedScopeComplete);
    assert_eq!(summary.items_completed, vec![common::ID_1]);
    let remaining = store.load_active().expect("load parent work");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].id, common::ID_2);
    assert_eq!(remaining[0].status, Status::Todo);
}

#[tokio::test]
async fn foreground_and_scheduled_wakeup_share_the_same_finite_loop() {
    // Arrange
    let directory = common::setup_test_env();
    let store = common::setup_task_golem_store(directory.path());
    store
        .save_active(&[
            materialized_item(common::ID_1, Vec::new(), false),
            materialized_item(common::ID_2, vec![common::ID_1.to_string()], false),
        ])
        .expect("seed ordered work");
    let adapter = FakeAdapter::new(vec![
        Ok(result_payload(common::ID_1, TrustedResultCode::Complete)),
        Ok(result_payload(common::ID_2, TrustedResultCode::Complete)),
    ]);

    // Act
    let (first_coordinator, _task) =
        spawn_coordinator(store.clone(), directory.path().to_path_buf());
    let first = run_foreground_supervisor(
        first_coordinator,
        &config(),
        &run_params(1),
        &CancellationToken::new(),
        &adapter,
        &PassingVerifier,
    )
    .await
    .expect("manual foreground result");
    let (second_coordinator, _task) =
        spawn_coordinator(store.clone(), directory.path().to_path_buf());
    let second = run_scheduled_wakeup(
        second_coordinator,
        &config(),
        &run_params(1),
        &CancellationToken::new(),
        &adapter,
        &PassingVerifier,
    )
    .await
    .expect("scheduled wakeup result");

    // Assert
    assert_eq!(first.halt_reason, HaltReason::BudgetReached);
    assert_eq!(second.halt_reason, HaltReason::BudgetReached);
    assert!(store.load_active().expect("load active work").is_empty());
    assert_eq!(adapter.peak_invocations(), 1);
}

#[tokio::test]
async fn shutdown_during_adapter_execution_stops_without_transitioning_the_task() {
    // Arrange
    let directory = common::setup_test_env();
    let store = common::setup_task_golem_store(directory.path());
    store
        .save_active(&[materialized_item(common::ID_1, Vec::new(), false)])
        .expect("seed work");
    let (coordinator, _task) = spawn_coordinator(store.clone(), directory.path().to_path_buf());
    let cancellation = CancellationToken::new();
    let cancellation_for_supervisor = cancellation.clone();
    let (execution_started, mut execution_starts) = tokio::sync::mpsc::unbounded_channel();
    let config = config();
    let supervisor = tokio::spawn(async move {
        run_foreground_supervisor(
            coordinator,
            &config,
            &run_params(1),
            &cancellation_for_supervisor,
            &BlockingAdapter { execution_started },
            &PassingVerifier,
        )
        .await
    });

    let claimed_item_id = execution_starts
        .recv()
        .await
        .expect("trusted adapter starts claimed execution");
    let claimed = store
        .load_active()
        .expect("load claimed work")
        .into_iter()
        .find(|item| item.id == claimed_item_id)
        .expect("claimed work remains active");
    assert_eq!(claimed.status, Status::Doing);
    assert_eq!(claimed.claimed_by.as_deref(), Some("phase-golem"));

    // Act
    cancellation.cancel();
    let summary = tokio::time::timeout(Duration::from_secs(1), supervisor)
        .await
        .expect("supervisor stops promptly")
        .expect("supervisor task")
        .expect("foreground supervisor result");

    // Assert
    assert_eq!(summary.halt_reason, HaltReason::ShutdownRequested);
    assert_eq!(summary.tasks_executed, 0);
    assert!(summary.items_blocked.is_empty());
    let claimed = store.load_active().expect("load claimed work").remove(0);
    assert_eq!(claimed.status, Status::Doing);
    assert_eq!(claimed.claimed_by.as_deref(), Some("phase-golem"));
}

#[tokio::test]
async fn shutdown_during_verification_stops_without_transitioning_the_task() {
    // Arrange
    let directory = common::setup_test_env();
    let store = common::setup_task_golem_store(directory.path());
    store
        .save_active(&[materialized_item(common::ID_1, Vec::new(), false)])
        .expect("seed work");
    let (coordinator, _task) = spawn_coordinator(store.clone(), directory.path().to_path_buf());
    let cancellation = CancellationToken::new();
    let cancellation_for_supervisor = cancellation.clone();
    let (verification_started, mut verification_starts) = tokio::sync::mpsc::unbounded_channel();
    let config = config();
    let supervisor = tokio::spawn(async move {
        run_foreground_supervisor(
            coordinator,
            &config,
            &run_params(1),
            &cancellation_for_supervisor,
            &FakeAdapter::new(vec![Ok(result_payload(
                common::ID_1,
                TrustedResultCode::Complete,
            ))]),
            &BlockingVerifier {
                verification_started,
            },
        )
        .await
    });

    let claimed_item_id = verification_starts
        .recv()
        .await
        .expect("deterministic verification starts");
    assert_eq!(claimed_item_id, common::ID_1);

    // Act
    cancellation.cancel();
    let summary = tokio::time::timeout(Duration::from_secs(1), supervisor)
        .await
        .expect("supervisor stops promptly")
        .expect("supervisor task")
        .expect("foreground supervisor result");

    // Assert
    assert_eq!(summary.halt_reason, HaltReason::ShutdownRequested);
    assert_eq!(summary.tasks_executed, 0);
    assert!(summary.items_blocked.is_empty());
    let claimed = store.load_active().expect("load claimed work").remove(0);
    assert_eq!(claimed.status, Status::Doing);
    assert_eq!(claimed.claimed_by.as_deref(), Some("phase-golem"));
}

#[tokio::test]
async fn shutdown_after_finalized_outcome_accounts_for_terminal_result() {
    for result_code in [TrustedResultCode::Complete, TrustedResultCode::Blocked] {
        // Arrange
        let directory = common::setup_test_env();
        let store = common::setup_task_golem_store(directory.path());
        store
            .save_active(&[materialized_item(common::ID_1, Vec::new(), false)])
            .expect("seed work");
        let (coordinator, _task) = spawn_coordinator(store.clone(), directory.path().to_path_buf());
        let cancellation = CancellationToken::new();
        let adapter = FakeAdapter::new(vec![Ok(result_payload(common::ID_1, result_code.clone()))]);
        let verifier = CancelAfterVerification {
            cancellation: cancellation.clone(),
        };

        // Act
        let summary = run_foreground_supervisor(
            coordinator,
            &config(),
            &run_params(1),
            &cancellation,
            &adapter,
            &verifier,
        )
        .await
        .expect("foreground supervisor result");

        // Assert
        assert_eq!(summary.halt_reason, HaltReason::ShutdownRequested);
        assert_eq!(summary.tasks_executed, 1);
        match result_code {
            TrustedResultCode::Complete => {
                assert_eq!(summary.items_completed, vec![common::ID_1]);
                assert!(summary.items_blocked.is_empty());
                assert!(store.load_active().expect("load active work").is_empty());
                assert_eq!(
                    store.load_all_archive().expect("load completed work")[0].status,
                    Status::Done
                );
            }
            TrustedResultCode::Blocked => {
                assert!(summary.items_completed.is_empty());
                assert_eq!(summary.items_blocked, vec![common::ID_1]);
                assert_eq!(
                    store.load_active().expect("load blocked work")[0].status,
                    Status::Blocked
                );
            }
        }
    }
}

#[tokio::test]
async fn persisted_dangling_dependency_halts_without_execution_or_mutation() {
    // Arrange
    let directory = common::setup_test_env();
    let store = common::setup_task_golem_store(directory.path());
    store
        .save_active(&[materialized_item(common::ID_1, Vec::new(), false)])
        .expect("seed work");
    let mut item = store.load_active().expect("load work").remove(0);
    item.dependencies = vec![common::ID_2.to_string()];
    std::fs::write(
        store.tasks_path(),
        format!(
            "{{\"schema_version\":1}}\n{}\n",
            serde_json::to_string(&item).expect("serialize dangling dependency fixture")
        ),
    )
    .expect("persist dangling dependency fixture");
    let (coordinator, _task) = spawn_coordinator(store.clone(), directory.path().to_path_buf());
    let adapter = FakeAdapter::new(Vec::new());

    // Act
    let summary = run_foreground_supervisor(
        coordinator,
        &config(),
        &run_params(1),
        &CancellationToken::new(),
        &adapter,
        &PassingVerifier,
    )
    .await
    .expect("foreground supervisor result");

    // Assert
    assert_eq!(summary.halt_reason, HaltReason::UnrecoverableFailure);
    assert_eq!(summary.tasks_executed, 0);
    assert!(summary.items_completed.is_empty());
    assert!(summary.items_blocked.is_empty());
    assert_eq!(adapter.peak_invocations(), 0);
    let item = store.load_active().expect("load affected work").remove(0);
    assert_eq!(item.status, Status::Todo);
    assert!(item.claimed_by.is_none());
    assert_eq!(item.dependencies, vec![common::ID_2]);
    assert!(!store.events_path().exists());
}

fn materialized_item(id: &str, dependencies: Vec<String>, human_gate: bool) -> Item {
    let mut item = common::make_pg_item(id, Status::Todo).0;
    item.dependencies = dependencies;
    item.extensions
        .insert(X_PG_OWNER.to_string(), serde_json::json!("phase-golem"));
    item.extensions
        .insert(X_PG_TEMPLATE_NODE_KEY.to_string(), serde_json::json!(PHASE));
    item.extensions.insert(
        X_PG_EXECUTOR_PROFILE.to_string(),
        serde_json::json!(PROFILE),
    );
    item.extensions.insert(
        X_PG_EXECUTION_POLICY.to_string(),
        serde_json::to_value(policy()).expect("serialize policy"),
    );
    item.extensions.insert(
        X_PG_VERIFICATION.to_string(),
        serde_json::to_value(VerificationPlan::default()).expect("serialize verification"),
    );
    if human_gate {
        item.extensions
            .insert(X_PG_HUMAN_DECISION.to_string(), serde_json::json!(true));
    }
    item
}

fn policy() -> PublicExecutionPolicy {
    PublicExecutionPolicy {
        timeout_minutes: 1,
        max_retries: 0,
        destructive: false,
        workflows: Vec::new(),
    }
}

fn config() -> PhaseGolemConfig {
    let mut config = PhaseGolemConfig::default();
    config.executor_profiles.insert(
        PROFILE.to_string(),
        TrustedExecutorProfile {
            command: "test-executor".to_string(),
            args: Vec::new(),
            environment: BTreeMap::new(),
        },
    );
    config
}

fn run_params(cap: u32) -> RunParams {
    RunParams {
        targets: Vec::new(),
        filter: Vec::new(),
        cap,
    }
}

fn targeted_run_params(target: &str, cap: u32) -> RunParams {
    RunParams {
        targets: vec![target.to_string()],
        filter: Vec::new(),
        cap,
    }
}

fn result_payload(item_id: &str, result: TrustedResultCode) -> String {
    serde_json::json!({
        "item_id": item_id,
        "phase": PHASE,
        "result": result,
        "summary": "executor completed task",
        "evidence_references": ["artifact:result"]
    })
    .to_string()
}

fn initialize_task_golem_project(project_root: &std::path::Path) {
    let output = Command::new("tg")
        .arg("init")
        .current_dir(project_root)
        .output()
        .expect("run task-golem init");
    assert!(
        output.status.success(),
        "task-golem init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit_task_golem_seed(project_root: &std::path::Path) {
    let add = Command::new("git")
        .args(["add", ".task-golem"])
        .current_dir(project_root)
        .output()
        .expect("stage Task Golem seed");
    assert!(
        add.status.success(),
        "stage Task Golem seed failed: {}",
        String::from_utf8_lossy(&add.stderr)
    );
    let commit = Command::new("git")
        .args(["commit", "-m", "Seed Task Golem work"])
        .current_dir(project_root)
        .output()
        .expect("commit Task Golem seed");
    assert!(
        commit.status.success(),
        "commit Task Golem seed failed: {}",
        String::from_utf8_lossy(&commit.stderr)
    );
}

fn create_discovered_item_through_tg_crud(
    project_root: &std::path::Path,
) -> Result<String, String> {
    let output = Command::new("tg")
        .args([
            "--json",
            "add",
            "Discovered work",
            "--set",
            "x-pg-owner=\"phase-golem\"",
            "--set",
            "x-pg-template-node-key=\"build\"",
            "--set",
            "x-pg-executor-profile=\"test-executor\"",
            "--set",
            "x-pg-execution-policy={\"timeout_minutes\":1,\"max_retries\":0,\"destructive\":false,\"workflows\":[]}",
            "--set",
            "x-pg-verification={\"required_checks\":[]}",
        ])
        .current_dir(project_root)
        .output()
        .map_err(|error| format!("run task-golem add: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "task-golem add failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice::<Item>(&output.stdout)
        .map(|item| item.id)
        .map_err(|error| format!("parse task-golem add response: {error}"))
}
