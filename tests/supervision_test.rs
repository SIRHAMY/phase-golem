mod common;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use phase_golem::agent::{TrustedExecutionRequest, TrustedExecutorAdapter};
use phase_golem::config::{
    PhaseGolemConfig, PublicExecutionPolicy, TrustedExecutorProfile, VerificationPlan,
};
use phase_golem::coordinator::spawn_coordinator;
use phase_golem::executor::{supervise_trusted_executor, DeterministicVerifier};
use phase_golem::materialization::{
    X_PG_EXECUTION_POLICY, X_PG_EXECUTOR_PROFILE, X_PG_TEMPLATE_NODE_KEY, X_PG_VERIFICATION,
};
use phase_golem::types::MAX_ATTEMPT_NOTE_TEXT_BYTES;
use phase_golem::types::{SupervisedOutcome, TrustedResultCode};
use task_golem::events::append::MAX_EVENT_LINE_BYTES;
use task_golem::events::{Event, EventType};
use task_golem::model::status::Status;
use task_golem::store::Store;

const PROFILE: &str = "release-local";
const PHASE: &str = "build";
const SECRET: &str = "runtime-secret-value";

struct FakeAdapter {
    store: Store,
    responses: Mutex<VecDeque<Result<String, String>>>,
    invocations: Mutex<Vec<(TrustedExecutorProfile, TrustedExecutionRequest)>>,
    sequence: Arc<Mutex<Vec<&'static str>>>,
}

impl FakeAdapter {
    fn new(
        store: Store,
        responses: Vec<Result<String, String>>,
        sequence: Arc<Mutex<Vec<&'static str>>>,
    ) -> Self {
        Self {
            store,
            responses: Mutex::new(responses.into()),
            invocations: Mutex::new(Vec::new()),
            sequence,
        }
    }
}

impl TrustedExecutorAdapter for FakeAdapter {
    fn invoke(
        &self,
        profile: &TrustedExecutorProfile,
        request: &TrustedExecutionRequest,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send {
        let claimed = self
            .store
            .load_active()
            .expect("load item during invocation")
            .into_iter()
            .find(|item| item.id == request.item_id)
            .expect("find claimed item");
        assert_eq!(claimed.status, Status::Doing);
        assert_eq!(claimed.claimed_by.as_deref(), Some("phase-golem"));
        self.sequence.lock().expect("sequence lock").push("invoke");
        self.invocations
            .lock()
            .expect("invocations lock")
            .push((profile.clone(), request.clone()));
        let response = self
            .responses
            .lock()
            .expect("responses lock")
            .pop_front()
            .expect("configured adapter response");
        async move { response }
    }
}

struct FakeVerifier {
    result: Result<Vec<String>, String>,
    calls: Mutex<Vec<(VerificationPlan, TrustedExecutionRequest, Vec<String>)>>,
    sequence: Arc<Mutex<Vec<&'static str>>>,
}

struct MutatingVerifier {
    store: Store,
}

impl DeterministicVerifier for MutatingVerifier {
    async fn verify(
        &self,
        _plan: &VerificationPlan,
        _request: &TrustedExecutionRequest,
        _executor_evidence: &[String],
    ) -> Result<Vec<String>, String> {
        let store = self.store.clone();
        store
            .with_lock(|store| {
                let mut items = store.load_active()?;
                let item = items.first_mut().expect("mutate active item");
                item.extensions.insert(
                    X_PG_EXECUTION_POLICY.to_string(),
                    serde_json::json!({
                        "timeout_minutes": 99,
                        "max_retries": 0,
                        "destructive": true,
                        "workflows": ["changed.md"]
                    }),
                );
                store.save_active(&items)
            })
            .map_err(|error| error.to_string())?;
        Ok(Vec::new())
    }
}

impl FakeVerifier {
    fn new(result: Result<Vec<String>, String>, sequence: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            result,
            calls: Mutex::new(Vec::new()),
            sequence,
        }
    }
}

impl DeterministicVerifier for FakeVerifier {
    fn verify(
        &self,
        plan: &VerificationPlan,
        request: &TrustedExecutionRequest,
        executor_evidence: &[String],
    ) -> impl std::future::Future<Output = Result<Vec<String>, String>> + Send {
        self.sequence.lock().expect("sequence lock").push("verify");
        self.calls.lock().expect("calls lock").push((
            plan.clone(),
            request.clone(),
            executor_evidence.to_vec(),
        ));
        let result = self.result.clone();
        async move { result }
    }
}

#[tokio::test]
async fn complete_and_blocked_results_are_verified_evidenced_and_transitioned_once() {
    for result_code in [TrustedResultCode::Complete, TrustedResultCode::Blocked] {
        // Arrange
        let (coordinator, store, _directory, config, policy, verification) = setup(0);
        let sequence = Arc::new(Mutex::new(Vec::new()));
        let adapter = FakeAdapter::new(
            store.clone(),
            vec![Ok(result_payload(common::ID_1, PHASE, result_code.clone()))],
            sequence.clone(),
        );
        let verifier = FakeVerifier::new(
            Ok(vec!["verification:cargo test".to_string()]),
            sequence.clone(),
        );

        // Act
        let outcome =
            supervise_trusted_executor(common::ID_1, &config, &coordinator, &adapter, &verifier)
                .await
                .expect("supervise trusted executor");

        // Assert
        let expected_outcome = match result_code {
            TrustedResultCode::Complete => SupervisedOutcome::Complete,
            TrustedResultCode::Blocked => SupervisedOutcome::Blocked,
        };
        assert_eq!(outcome, expected_outcome);
        assert_eq!(
            *sequence.lock().expect("sequence lock"),
            vec!["invoke", "verify"]
        );

        let invocations = adapter.invocations.lock().expect("invocations lock");
        assert_eq!(invocations.len(), 1);
        let (resolved_profile, request) = &invocations[0];
        assert_eq!(
            request.policy, policy,
            "policy must come from the task snapshot"
        );
        assert_eq!(request.phase, PHASE);
        assert_eq!(resolved_profile.command, "/trusted/release-executor");
        assert_eq!(resolved_profile.environment["RELEASE_TOKEN"], SECRET);
        assert!(!serde_json::to_string(request)
            .expect("serialize public request")
            .contains(SECRET));

        let verification_calls = verifier.calls.lock().expect("verification calls lock");
        assert_eq!(verification_calls.len(), 1);
        assert_eq!(verification_calls[0].0, verification);
        assert_eq!(verification_calls[0].2, vec!["artifact:release.tar"]);

        let events = persisted_events(&store, result_code == TrustedResultCode::Complete);
        let statuses = events
            .iter()
            .filter_map(|event| event.status)
            .collect::<Vec<_>>();
        assert_eq!(statuses.len(), 2, "claim plus one supervised transition");
        assert_eq!(statuses[0], Status::Doing);
        assert_eq!(
            statuses[1],
            if result_code == TrustedResultCode::Complete {
                Status::Done
            } else {
                Status::Blocked
            }
        );
        let note_index = events
            .iter()
            .position(|event| event.event_type == EventType::Note)
            .expect("PG attempt evidence note");
        let terminal_index = events
            .iter()
            .rposition(|event| event.event_type == EventType::StatusTransition)
            .expect("terminal transition");
        assert!(
            note_index < terminal_index,
            "attempt evidence must precede transition"
        );
        let note = &events[note_index].text;
        assert!(note.contains("phase-golem/trusted-executor-attempt/v1"));
        assert!(note.contains("artifact:release.tar"));
        assert!(note.contains("verification:cargo test"));
        assert!(!note.contains(SECRET));
        assert!(!note.contains("/trusted/release-executor"));

        if result_code == TrustedResultCode::Complete {
            assert!(store.load_active().expect("load active").is_empty());
            assert_eq!(
                store.load_all_archive().expect("load archive")[0].status,
                Status::Done
            );
        } else {
            let blocked = store.load_active().expect("load blocked item").remove(0);
            assert_eq!(blocked.status, Status::Blocked);
            assert_eq!(blocked.blocked_reason.as_deref(), Some("executor summary"));
        }
    }
}

#[tokio::test]
async fn malformed_and_identity_mismatched_results_are_rejected_without_result_transition() {
    let cases = [
        (
            "malformed outcome",
            serde_json::json!({
                "item_id": common::ID_1,
                "phase": PHASE,
                "result": "failed",
                "summary": "not an accepted outcome",
                "evidence_references": ["artifact:failed"]
            })
            .to_string(),
        ),
        (
            "identity mismatch",
            result_payload(common::ID_2, PHASE, TrustedResultCode::Complete),
        ),
    ];

    for (name, payload) in cases {
        // Arrange
        let (coordinator, store, _directory, config, _policy, _verification) = setup(0);
        let sequence = Arc::new(Mutex::new(Vec::new()));
        let adapter = FakeAdapter::new(store.clone(), vec![Ok(payload)], sequence.clone());
        let verifier = FakeVerifier::new(Ok(Vec::new()), sequence);

        // Act
        let result =
            supervise_trusted_executor(common::ID_1, &config, &coordinator, &adapter, &verifier)
                .await;

        // Assert
        assert!(result.is_err(), "{name} must be rejected");
        assert!(verifier
            .calls
            .lock()
            .expect("verification calls lock")
            .is_empty());
        let item = store.load_active().expect("load rejected item").remove(0);
        assert_eq!(item.status, Status::Doing);
        let events = persisted_events(&store, false);
        assert_eq!(
            events
                .iter()
                .filter_map(|event| event.status)
                .collect::<Vec<_>>(),
            vec![Status::Doing],
            "{name} must not cause a result transition"
        );
        assert!(events
            .iter()
            .any(|event| event.event_type == EventType::Note && event.text.contains("rejected")));
    }
}

#[tokio::test]
async fn credential_bearing_completed_result_is_redacted_without_affecting_verification() {
    // Arrange
    let (coordinator, store, _directory, config, _policy, _verification) = setup(0);
    let sequence = Arc::new(Mutex::new(Vec::new()));
    let adapter = FakeAdapter::new(
        store.clone(),
        vec![Ok(serde_json::json!({
            "item_id": common::ID_1,
            "phase": PHASE,
            "result": "complete",
            "summary": format!("completed with {SECRET}"),
            "evidence_references": [format!("artifact:{SECRET}")]
        })
        .to_string())],
        sequence.clone(),
    );
    let verifier = FakeVerifier::new(Ok(vec![format!("verification:{SECRET}")]), sequence);

    // Act
    let outcome =
        supervise_trusted_executor(common::ID_1, &config, &coordinator, &adapter, &verifier)
            .await
            .expect("supervise credential-bearing completed result");

    // Assert
    assert_eq!(outcome, SupervisedOutcome::Complete);
    assert_eq!(
        verifier.calls.lock().expect("verification calls lock")[0].2,
        vec![format!("artifact:{SECRET}")],
        "verification must receive the original executor evidence"
    );
    let durable = std::fs::read_to_string(store.events_archive_path()).expect("read PG events");
    assert!(!durable.contains(SECRET));
    assert!(durable.contains("[REDACTED]"));
}

#[tokio::test]
async fn credential_bearing_adapter_error_is_redacted_before_persistence() {
    // Arrange
    let (coordinator, store, _directory, config, _policy, _verification) = setup(0);
    let sequence = Arc::new(Mutex::new(Vec::new()));
    let adapter = FakeAdapter::new(
        store.clone(),
        vec![Err(format!("executor failed: {SECRET}"))],
        sequence.clone(),
    );
    let verifier = FakeVerifier::new(Ok(Vec::new()), sequence);

    // Act
    let result =
        supervise_trusted_executor(common::ID_1, &config, &coordinator, &adapter, &verifier).await;

    // Assert
    assert!(result.is_err());
    let durable = std::fs::read_to_string(store.events_path()).expect("read PG events");
    assert!(!durable.contains(SECRET));
    assert!(durable.contains("[REDACTED]"));
    let statuses = persisted_events(&store, false)
        .iter()
        .filter_map(|event| event.status)
        .collect::<Vec<_>>();
    assert_eq!(statuses, vec![Status::Doing]);
}

#[tokio::test]
async fn credential_bearing_verifier_evidence_is_redacted_before_persistence() {
    // Arrange
    let (coordinator, store, _directory, config, _policy, _verification) = setup(0);
    let sequence = Arc::new(Mutex::new(Vec::new()));
    let adapter = FakeAdapter::new(
        store.clone(),
        vec![Ok(result_payload(
            common::ID_1,
            PHASE,
            TrustedResultCode::Complete,
        ))],
        sequence.clone(),
    );
    let verifier = FakeVerifier::new(Ok(vec![format!("verification:{SECRET}")]), sequence);

    // Act
    let outcome =
        supervise_trusted_executor(common::ID_1, &config, &coordinator, &adapter, &verifier)
            .await
            .expect("supervise credential-bearing verifier evidence");

    // Assert
    assert_eq!(outcome, SupervisedOutcome::Complete);
    let durable = std::fs::read_to_string(store.events_archive_path()).expect("read PG events");
    assert!(!durable.contains(SECRET));
    assert!(durable.contains("[REDACTED]"));
    let statuses = persisted_events(&store, true)
        .iter()
        .filter_map(|event| event.status)
        .collect::<Vec<_>>();
    assert_eq!(statuses, vec![Status::Doing, Status::Done]);
}

#[tokio::test]
async fn missing_executor_profile_does_not_claim_or_invoke() {
    // Arrange
    let (coordinator, store, _directory, mut config, _policy, _verification) = setup(0);
    config.executor_profiles.clear();
    let sequence = Arc::new(Mutex::new(Vec::new()));
    let adapter = FakeAdapter::new(store.clone(), Vec::new(), sequence.clone());
    let verifier = FakeVerifier::new(Ok(Vec::new()), sequence);

    // Act
    let result =
        supervise_trusted_executor(common::ID_1, &config, &coordinator, &adapter, &verifier).await;

    // Assert
    assert!(result
        .expect_err("missing profile must fail")
        .contains("not configured"));
    assert_preclaim_rejection(&store, &adapter);
}

#[tokio::test]
async fn malformed_execution_snapshot_does_not_claim_or_invoke() {
    // Arrange
    let (coordinator, store, _directory, config, _policy, _verification) = setup(0);
    let mut item = store.load_active().expect("load item").remove(0);
    item.extensions.insert(
        X_PG_EXECUTION_POLICY.to_string(),
        serde_json::json!("malformed"),
    );
    store
        .save_active(&[item])
        .expect("persist malformed snapshot");
    let sequence = Arc::new(Mutex::new(Vec::new()));
    let adapter = FakeAdapter::new(store.clone(), Vec::new(), sequence.clone());
    let verifier = FakeVerifier::new(Ok(Vec::new()), sequence);

    // Act
    let result =
        supervise_trusted_executor(common::ID_1, &config, &coordinator, &adapter, &verifier).await;

    // Assert
    assert!(result
        .expect_err("malformed snapshot must fail")
        .contains("invalid execution policy snapshot"));
    assert_preclaim_rejection(&store, &adapter);
}

#[tokio::test]
async fn oversized_result_records_bounded_rejection_without_terminal_transition() {
    // Arrange
    let (coordinator, store, _directory, config, _policy, _verification) = setup(0);
    let oversized_payload = serde_json::json!({
        "item_id": common::ID_1,
        "phase": PHASE,
        "result": "complete",
        "summary": "s".repeat(5_000),
        "evidence_references": ["e".repeat(5_000)]
    })
    .to_string();
    let sequence = Arc::new(Mutex::new(Vec::new()));
    let adapter = FakeAdapter::new(store.clone(), vec![Ok(oversized_payload)], sequence.clone());
    let verifier = FakeVerifier::new(Ok(Vec::new()), sequence);

    // Act
    let result =
        supervise_trusted_executor(common::ID_1, &config, &coordinator, &adapter, &verifier).await;

    // Assert
    assert!(result
        .expect_err("oversized result must fail")
        .contains("evidence budget"));
    assert!(verifier
        .calls
        .lock()
        .expect("verification calls lock")
        .is_empty());
    let item = store.load_active().expect("load claimed item").remove(0);
    assert_eq!(item.status, Status::Doing);
    let events = persisted_events(&store, false);
    assert_eq!(
        events
            .iter()
            .filter_map(|event| event.status)
            .collect::<Vec<_>>(),
        vec![Status::Doing]
    );
    let rejection = events
        .iter()
        .find(|event| event.event_type == EventType::Note)
        .expect("bounded rejection evidence");
    assert!(rejection
        .text
        .contains("exceeds the PG attempt evidence budget"));
    assert!(rejection.text.len() <= MAX_ATTEMPT_NOTE_TEXT_BYTES);
    assert!(
        serde_json::to_string(rejection)
            .expect("serialize rejection event")
            .len()
            < MAX_EVENT_LINE_BYTES
    );
}

#[tokio::test]
async fn failed_deterministic_verification_records_evidence_without_result_transition() {
    // Arrange
    let (coordinator, store, _directory, config, _policy, _verification) = setup(0);
    let sequence = Arc::new(Mutex::new(Vec::new()));
    let adapter = FakeAdapter::new(
        store.clone(),
        vec![Ok(result_payload(
            common::ID_1,
            PHASE,
            TrustedResultCode::Complete,
        ))],
        sequence.clone(),
    );
    let verifier = FakeVerifier::new(Err(format!("cargo test failed: {SECRET}")), sequence);

    // Act
    let result =
        supervise_trusted_executor(common::ID_1, &config, &coordinator, &adapter, &verifier).await;

    // Assert
    assert!(result
        .expect_err("verification must fail")
        .contains("Deterministic verification failed"));
    let item = store
        .load_active()
        .expect("load untransitioned item")
        .remove(0);
    assert_eq!(item.status, Status::Doing);
    let events = persisted_events(&store, false);
    assert_eq!(
        events
            .iter()
            .filter_map(|event| event.status)
            .collect::<Vec<_>>(),
        vec![Status::Doing]
    );
    let evidence = events
        .iter()
        .find(|event| event.event_type == EventType::Note)
        .expect("rejected attempt evidence");
    assert!(!evidence.text.contains(SECRET));
    assert!(evidence.text.contains("[REDACTED]"));
}

#[tokio::test]
async fn execution_contract_mutation_after_verification_cannot_finalize_stale_work() {
    // Arrange
    let (coordinator, store, _directory, config, _policy, _verification) = setup(0);
    let sequence = Arc::new(Mutex::new(Vec::new()));
    let adapter = FakeAdapter::new(
        store.clone(),
        vec![Ok(result_payload(
            common::ID_1,
            PHASE,
            TrustedResultCode::Complete,
        ))],
        sequence,
    );
    let verifier = MutatingVerifier {
        store: store.clone(),
    };

    // Act
    let result =
        supervise_trusted_executor(common::ID_1, &config, &coordinator, &adapter, &verifier).await;

    // Assert
    assert!(result
        .expect_err("stale execution contract must not finalize")
        .to_string()
        .contains("execution snapshot changed"));
    let item = store
        .load_active()
        .expect("load unfinalized item")
        .remove(0);
    assert_eq!(item.status, Status::Doing);
    assert_eq!(item.claimed_by.as_deref(), Some("phase-golem"));
    assert_eq!(persisted_events(&store, false).len(), 1);
}

#[tokio::test]
async fn snapshotted_retry_budget_records_each_attempt_and_one_result_transition() {
    // Arrange
    let (coordinator, store, _directory, config, policy, _verification) = setup(1);
    let sequence = Arc::new(Mutex::new(Vec::new()));
    let adapter = FakeAdapter::new(
        store.clone(),
        vec![
            Err("temporary executor failure".to_string()),
            Ok(result_payload(
                common::ID_1,
                PHASE,
                TrustedResultCode::Complete,
            )),
        ],
        sequence.clone(),
    );
    let verifier = FakeVerifier::new(Ok(Vec::new()), sequence);

    // Act
    let outcome =
        supervise_trusted_executor(common::ID_1, &config, &coordinator, &adapter, &verifier)
            .await
            .expect("retry trusted executor");

    // Assert
    assert_eq!(outcome, SupervisedOutcome::Complete);
    let invocations = adapter.invocations.lock().expect("invocations lock");
    assert_eq!(
        invocations
            .iter()
            .map(|(_, request)| request.attempt)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert!(invocations
        .iter()
        .all(|(_, request)| request.policy == policy));

    let events = persisted_events(&store, true);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::Note)
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter_map(|event| event.status)
            .collect::<Vec<_>>(),
        vec![Status::Doing, Status::Done]
    );
}

fn setup(
    max_retries: u32,
) -> (
    phase_golem::coordinator::CoordinatorHandle,
    Store,
    tempfile::TempDir,
    PhaseGolemConfig,
    PublicExecutionPolicy,
    VerificationPlan,
) {
    let directory = common::setup_test_env();
    let store = common::setup_task_golem_store(directory.path());
    let policy = PublicExecutionPolicy {
        timeout_minutes: 7,
        max_retries,
        destructive: true,
        workflows: vec!["snapshotted-build.md".to_string()],
    };
    let verification = VerificationPlan {
        required_checks: vec!["cargo test".to_string()],
    };
    let mut item = common::make_pg_item(common::ID_1, Status::Todo);
    item.0
        .extensions
        .insert(X_PG_TEMPLATE_NODE_KEY.to_string(), serde_json::json!(PHASE));
    item.0.extensions.insert(
        X_PG_EXECUTOR_PROFILE.to_string(),
        serde_json::json!(PROFILE),
    );
    item.0.extensions.insert(
        X_PG_EXECUTION_POLICY.to_string(),
        serde_json::to_value(&policy).expect("serialize policy"),
    );
    item.0.extensions.insert(
        X_PG_VERIFICATION.to_string(),
        serde_json::to_value(&verification).expect("serialize verification"),
    );
    store
        .save_active(&[item.0])
        .expect("seed materialized item");

    let mut config = PhaseGolemConfig::default();
    config.execution.phase_timeout_minutes = 99;
    config.execution.max_retries = 99;
    config.executor_profiles = std::collections::HashMap::from([(
        PROFILE.to_string(),
        TrustedExecutorProfile {
            command: "/trusted/release-executor".to_string(),
            args: vec!["--local".to_string()],
            environment: BTreeMap::from([("RELEASE_TOKEN".to_string(), SECRET.to_string())]),
        },
    )]);
    let (coordinator, _task) = spawn_coordinator(store.clone(), directory.path().to_path_buf());
    (coordinator, store, directory, config, policy, verification)
}

fn result_payload(item_id: &str, phase: &str, result: TrustedResultCode) -> String {
    serde_json::json!({
        "item_id": item_id,
        "phase": phase,
        "result": result,
        "summary": "executor summary",
        "evidence_references": ["artifact:release.tar"]
    })
    .to_string()
}

fn assert_preclaim_rejection(store: &Store, adapter: &FakeAdapter) {
    let item = store.load_active().expect("load unclaimed item").remove(0);
    assert_eq!(item.status, Status::Todo);
    assert!(item.claimed_by.is_none());
    assert!(adapter
        .invocations
        .lock()
        .expect("invocations lock")
        .is_empty());
    assert!(persisted_events(store, false).is_empty());
}

fn persisted_events(store: &Store, is_archived: bool) -> Vec<Event> {
    let path = if is_archived {
        store.events_archive_path()
    } else {
        store.events_path()
    };
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse event"))
        .collect()
}
