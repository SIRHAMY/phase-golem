mod common;

use phase_golem::config::{PublicExecutionPolicy, VerificationPlan};
use phase_golem::coordinator::{spawn_coordinator, ExpectedExecutionSnapshot};
use phase_golem::materialization::{
    X_PG_EXECUTION_POLICY, X_PG_EXECUTOR_PROFILE, X_PG_TEMPLATE_NODE_KEY, X_PG_VERIFICATION,
};
use phase_golem::pg_item::{PgItem, X_PG_HUMAN_DECISION, X_PG_OWNER};
use phase_golem::types::{FollowUp, ItemUpdate, SizeLevel};
use task_golem::model::status::Status;

fn setup(
    items: Vec<PgItem>,
) -> (
    phase_golem::coordinator::CoordinatorHandle,
    task_golem::store::Store,
    tempfile::TempDir,
) {
    let directory = common::setup_test_env();
    let store = common::setup_task_golem_store(directory.path());
    let raw_items = items.into_iter().map(|item| item.0).collect::<Vec<_>>();
    store.save_active(&raw_items).expect("seed items");
    let (handle, _task) = spawn_coordinator(store.clone(), directory.path().to_path_buf());
    (handle, store, directory)
}

#[tokio::test]
async fn snapshot_contains_items_and_matching_tg_dependency_evaluation() {
    // Arrange
    let prerequisite = common::make_pg_item(common::ID_1, Status::Todo);
    let mut dependent = common::make_pg_item(common::ID_2, Status::Todo);
    dependent.0.dependencies = vec![common::ID_1.to_string()];
    let (handle, _store, _directory) = setup(vec![dependent, prerequisite]);

    // Act
    let snapshot = handle.get_snapshot().await.expect("load snapshot");

    // Assert
    assert_eq!(snapshot.len(), 2);
    assert!(
        !snapshot
            .dependency_evaluation
            .readiness_for(common::ID_2)
            .expect("dependent readiness")
            .is_ready
    );
    assert_eq!(
        snapshot.dependency_evaluation.ready_items[0].id,
        common::ID_1
    );
}

#[tokio::test]
async fn claim_atomically_rechecks_readiness_and_records_tg_claim() {
    // Arrange
    let item = common::make_pg_item(common::ID_1, Status::Todo);
    let (handle, store, _directory) = setup(vec![item]);

    // Act
    handle.claim_item(common::ID_1).await.expect("claim item");

    // Assert
    let claimed = store.load_active().expect("load claimed item").remove(0);
    assert_eq!(claimed.status, Status::Doing);
    assert_eq!(claimed.claimed_by.as_deref(), Some("phase-golem"));
    assert!(claimed.claimed_at.is_some());
    let events = std::fs::read_to_string(store.events_path()).expect("read claim event");
    assert!(events.contains("\"status\":\"doing\""));
}

#[tokio::test]
async fn concurrent_claims_allow_exactly_one_winner() {
    // Arrange
    let item = common::make_pg_item(common::ID_1, Status::Todo);
    let (handle, store, _directory) = setup(vec![item]);
    let competing_handle = handle.clone();

    // Act
    let (first, second) = tokio::join!(
        handle.claim_item(common::ID_1),
        competing_handle.claim_item(common::ID_1)
    );

    // Assert
    assert_ne!(first.is_ok(), second.is_ok());
    let item = store.load_active().expect("load claimed task").remove(0);
    assert_eq!(item.status, Status::Doing);
    assert_eq!(item.claimed_by.as_deref(), Some("phase-golem"));
}

#[tokio::test]
async fn guarded_claim_rejects_a_stale_execution_snapshot_without_claiming() {
    // Arrange
    let mut item = common::make_pg_item(common::ID_1, Status::Todo);
    item.0.extensions.insert(
        X_PG_TEMPLATE_NODE_KEY.to_string(),
        serde_json::json!("build"),
    );
    item.0.extensions.insert(
        X_PG_EXECUTOR_PROFILE.to_string(),
        serde_json::json!("trusted-local"),
    );
    item.0.extensions.insert(
        X_PG_EXECUTION_POLICY.to_string(),
        serde_json::to_value(PublicExecutionPolicy {
            timeout_minutes: 5,
            max_retries: 0,
            destructive: false,
            workflows: Vec::new(),
        })
        .expect("serialize policy"),
    );
    item.0.extensions.insert(
        X_PG_VERIFICATION.to_string(),
        serde_json::to_value(VerificationPlan::default()).expect("serialize verification"),
    );
    let expected = ExpectedExecutionSnapshot::from_item(&item).expect("prepare snapshot");
    item.0.extensions.insert(
        X_PG_TEMPLATE_NODE_KEY.to_string(),
        serde_json::json!("review"),
    );
    let (handle, store, _directory) = setup(vec![item]);

    // Act
    let result = handle
        .claim_item_with_expected_execution_snapshot(common::ID_1, expected)
        .await;

    // Assert
    assert!(result
        .expect_err("stale execution snapshot must reject the claim")
        .to_string()
        .contains("execution snapshot changed"));
    let item = store.load_active().expect("load unclaimed item").remove(0);
    assert_eq!(item.status, Status::Todo);
    assert!(item.claimed_by.is_none());
    assert!(!store.events_path().exists());
}

#[tokio::test]
async fn claim_rejects_an_item_that_is_no_longer_ready() {
    // Arrange
    let prerequisite = common::make_pg_item(common::ID_1, Status::Todo);
    let mut dependent = common::make_pg_item(common::ID_2, Status::Todo);
    dependent.0.dependencies = vec![common::ID_1.to_string()];
    let (handle, store, _directory) = setup(vec![prerequisite, dependent]);

    // Act
    let result = handle.claim_item(common::ID_2).await;

    // Assert
    assert!(result.is_err());
    let dependent = store
        .load_active()
        .expect("load active")
        .into_iter()
        .find(|item| item.id == common::ID_2)
        .expect("find dependent");
    assert_eq!(dependent.status, Status::Todo);
    assert!(dependent.claimed_by.is_none());
}

#[tokio::test]
async fn claim_rejects_human_gates_and_non_pg_tasks() {
    // Arrange
    let mut gate = common::make_pg_item(common::ID_1, Status::Todo);
    gate.0
        .extensions
        .insert(X_PG_HUMAN_DECISION.to_string(), serde_json::json!(true));
    let mut non_pg = common::make_pg_item(common::ID_2, Status::Todo);
    non_pg.0.extensions.remove(X_PG_OWNER);
    let (handle, store, _directory) = setup(vec![gate, non_pg]);

    // Act
    let gate_result = handle.claim_item(common::ID_1).await;
    let non_pg_result = handle.claim_item(common::ID_2).await;

    // Assert
    assert!(gate_result.is_err());
    assert!(non_pg_result.is_err());
    assert!(store
        .load_active()
        .expect("load unclaimed tasks")
        .iter()
        .all(|item| item.status == Status::Todo && item.claimed_by.is_none()));
}

#[tokio::test]
async fn execution_failure_unblocks_to_todo_and_can_be_reclaimed() {
    // Arrange
    let item = common::make_doing_pg_item(common::ID_1, "build");
    let (handle, store, _directory) = setup(vec![item]);

    // Act
    handle
        .update_item(
            common::ID_1,
            ItemUpdate::SetBlocked("executor exited 17".to_string()),
        )
        .await
        .expect("block item");
    handle
        .unblock_item(common::ID_1, Some("operator repaired input".to_string()))
        .await
        .expect("unblock item");

    // Verify unblock returns the item to unclaimed Todo.
    let restored = store.load_active().expect("load restored item").remove(0);
    assert_eq!(restored.status, Status::Todo);
    assert!(restored.claimed_by.is_none());
    assert!(restored.claimed_at.is_none());
    assert!(restored.blocked_reason.is_none());
    assert!(restored.blocked_from_status.is_none());
    assert_eq!(
        restored.extensions["x-pg-unblock-context"],
        "operator repaired input"
    );
    let events = std::fs::read_to_string(store.events_path()).expect("read lifecycle events");
    assert!(events.contains("executor exited 17"));
    assert!(events.contains("\"status\":\"todo\""));

    // Verify the coordinator can claim the reset item again.
    handle.claim_item(common::ID_1).await.expect("reclaim item");
    let reclaimed = store.load_active().expect("load reclaimed item").remove(0);
    assert_eq!(reclaimed.status, Status::Doing);
    assert_eq!(reclaimed.claimed_by.as_deref(), Some("phase-golem"));
}

#[tokio::test]
async fn done_archives_only_the_explicit_child_and_leaves_parent_unchanged() {
    // Arrange
    let parent = common::make_pg_item(common::ID_1, Status::Todo);
    let mut child = common::make_doing_pg_item(common::ID_2, "review");
    child.0.parent = Some(common::ID_1.to_string());
    let (handle, store, _directory) = setup(vec![parent, child]);

    // Act
    handle
        .update_item(common::ID_2, ItemUpdate::TransitionStatus(Status::Done))
        .await
        .expect("complete child");

    // Assert
    let active = store.load_active().expect("load parent");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, common::ID_1);
    assert_eq!(active[0].status, Status::Todo);
    let archived = store.load_all_archive().expect("load archived child");
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].id, common::ID_2);
    assert_eq!(archived[0].status, Status::Done);
    let snapshot = handle.get_snapshot().await.expect("load archive evidence");
    assert!(snapshot.archived_done_ids.contains(common::ID_2));
}

#[tokio::test]
async fn metadata_update_preserves_claim_and_status() {
    // Arrange
    let item = common::make_doing_pg_item(common::ID_1, "build");
    let (handle, store, _directory) = setup(vec![item]);

    // Act
    handle
        .update_item(
            common::ID_1,
            ItemUpdate::SetPipelineType("feature".to_string()),
        )
        .await
        .expect("update metadata");

    // Assert
    let item = store.load_active().expect("load item").remove(0);
    assert_eq!(item.status, Status::Doing);
    assert_eq!(item.claimed_by.as_deref(), Some("phase-golem"));
}

#[tokio::test]
async fn discovered_follow_ups_receive_canonical_uuidv7_identity() {
    // Arrange
    let (handle, store, _directory) = setup(vec![]);
    let follow_up = FollowUp {
        title: "Add regression coverage".to_string(),
        context: Some("Found during build".to_string()),
        suggested_size: Some(SizeLevel::Small),
        suggested_risk: None,
    };

    // Act
    let ids = handle
        .ingest_follow_ups(vec![follow_up], "build")
        .await
        .expect("ingest follow-up");

    // Assert
    assert_eq!(ids.len(), 1);
    task_golem::validate_id(&ids[0]).expect("canonical UUIDv7");
    let item = store.load_active().expect("load follow-up").remove(0);
    assert_eq!(item.status, Status::Todo);
    assert_eq!(item.extensions["x-pg-owner"], "phase-golem");
}

#[tokio::test]
async fn merge_archives_only_source_and_preserves_target_lifecycle() {
    // Arrange
    let source = common::make_doing_pg_item(common::ID_1, "build");
    let target = common::make_pg_item(common::ID_2, Status::Todo);
    let (handle, store, _directory) = setup(vec![source, target]);

    // Act
    handle
        .merge_item(common::ID_1, common::ID_2)
        .await
        .expect("merge items");

    // Assert
    let active = store.load_active().expect("load target");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id, common::ID_2);
    assert_eq!(active[0].status, Status::Todo);
    let archived = store.load_all_archive().expect("load merged source");
    assert_eq!(archived.len(), 1);
    assert_eq!(archived[0].id, common::ID_1);
    assert_eq!(archived[0].status, Status::Done);
}
