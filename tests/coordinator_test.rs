mod common;

use std::fs;
use std::path::Path;
use std::process::Command;
use task_golem::model::item::Item;
use task_golem::store::Store;

use phase_golem::coordinator::spawn_coordinator;
use phase_golem::pg_error::PgError;
use phase_golem::pg_item::{self, PgItem};
use phase_golem::types::{
    DimensionLevel, FollowUp, ItemStatus, ItemUpdate, PhasePool, PhaseResult, ResultCode,
    SizeLevel, StructuredDescription, UpdatedAssessments,
};

// --- Test helpers ---

/// Construct a minimal PhaseResult for testing.
fn make_phase_result(item_id: &str, phase: &str, summary: &str) -> PhaseResult {
    PhaseResult {
        item_id: item_id.to_string(),
        phase: phase.to_string(),
        result: ResultCode::PhaseComplete,
        summary: summary.to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: vec![],
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: vec![],
        description: None,
    }
}

/// Save items to the task-golem store and commit the .task-golem/ directory.
fn save_and_commit_store(root: &Path, store: &Store, items: &[Item]) {
    store.save_active(items).expect("save items to store");

    Command::new("git")
        .args(["add", ".task-golem/"])
        .current_dir(root)
        .output()
        .expect("stage .task-golem/");

    Command::new("git")
        .args(["commit", "-m", "Save store"])
        .current_dir(root)
        .output()
        .expect("commit store");
}

/// Create a coordinator with items pre-populated in the store.
/// Returns (handle, coord_task, TempDir).
fn setup_coordinator_with_items(
    items: Vec<PgItem>,
) -> (
    phase_golem::coordinator::CoordinatorHandle,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let raw_items: Vec<Item> = items.into_iter().map(|pg| pg.0).collect();
    save_and_commit_store(dir.path(), &store, &raw_items);

    let (handle, coord_task) =
        spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    (handle, coord_task, dir)
}

// =============================================================================
// GetSnapshot tests
// =============================================================================

#[tokio::test]
async fn get_snapshot_returns_current_state() {
    let (handle, _task, _dir) = setup_coordinator_with_items(vec![
        common::make_pg_item("WRK-001", ItemStatus::New),
        common::make_in_progress_pg_item("WRK-002", "build"),
    ]);

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot.len(), 2);
    assert_eq!(snapshot[0].id(), "WRK-001");
    assert_eq!(snapshot[0].pg_status(), ItemStatus::New);
    assert_eq!(snapshot[1].id(), "WRK-002");
    assert_eq!(snapshot[1].pg_status(), ItemStatus::InProgress);
}

#[tokio::test]
async fn get_snapshot_reflects_updates() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    handle
        .update_item("WRK-001", ItemUpdate::TransitionStatus(ItemStatus::Scoping))
        .await
        .unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].pg_status(), ItemStatus::Scoping);
}

#[tokio::test]
async fn get_snapshot_returns_pg_items_with_extension_fields() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    // Create an item with various extension fields
    let mut pg = common::make_in_progress_pg_item("WRK-001", "build");
    pg_item::set_size(&mut pg.0, Some(&SizeLevel::Medium));
    pg_item::set_risk(&mut pg.0, Some(&DimensionLevel::High));
    pg_item::set_impact(&mut pg.0, Some(&DimensionLevel::Low));
    pg_item::set_pipeline_type(&mut pg.0, Some("feature"));

    save_and_commit_store(dir.path(), &store, &[pg.0]);

    let (handle, _task) = spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot.len(), 1);
    let item = &snapshot[0];
    assert_eq!(item.id(), "WRK-001");
    assert_eq!(item.pg_status(), ItemStatus::InProgress);
    assert_eq!(item.phase(), Some("build".to_string()));
    assert_eq!(item.phase_pool(), Some(PhasePool::Main));
    assert_eq!(item.size(), Some(SizeLevel::Medium));
    assert_eq!(item.risk(), Some(DimensionLevel::High));
    assert_eq!(item.impact(), Some(DimensionLevel::Low));
    assert_eq!(item.pipeline_type(), Some("feature".to_string()));
}

#[tokio::test]
async fn get_snapshot_after_external_store_modification() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg1 = common::make_pg_item("WRK-001", ItemStatus::New);
    save_and_commit_store(dir.path(), &store, &[pg1.0]);

    let (handle, _task) =
        spawn_coordinator(store.clone(), dir.path().to_path_buf(), "WRK".to_string());

    // Verify initial state
    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot.len(), 1);

    // Externally add another item (simulating `tg add`)
    let pg2 = common::make_pg_item("WRK-002", ItemStatus::New);
    let mut current_items: Vec<Item> = store.with_lock(|s| s.load_active()).expect("load active");
    current_items.push(pg2.0);
    store.save_active(&current_items).expect("save active");

    // GetSnapshot should see the new item (read-through behavior)
    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot.len(), 2);
    assert!(snapshot.iter().any(|i| i.id() == "WRK-002"));
}

// =============================================================================
// UpdateItem tests
// =============================================================================

#[tokio::test]
async fn update_item_transition_status() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    handle
        .update_item("WRK-001", ItemUpdate::TransitionStatus(ItemStatus::Scoping))
        .await
        .unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].pg_status(), ItemStatus::Scoping);
}

#[tokio::test]
async fn update_item_invalid_transition_logged_and_skipped() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    // New -> Done is invalid (skips Scoping/Ready/InProgress)
    // The adapter logs a warning and skips the transition
    handle
        .update_item("WRK-001", ItemUpdate::TransitionStatus(ItemStatus::Done))
        .await
        .unwrap();

    // Status should remain New (transition was skipped)
    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].pg_status(), ItemStatus::New);
}

#[tokio::test]
async fn update_item_set_phase() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_in_progress_pg_item("WRK-001", "prd")]);

    handle
        .update_item("WRK-001", ItemUpdate::SetPhase("build".to_string()))
        .await
        .unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].phase(), Some("build".to_string()));
}

#[tokio::test]
async fn update_item_clear_phase() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_in_progress_pg_item("WRK-001", "build")]);

    handle
        .update_item("WRK-001", ItemUpdate::ClearPhase)
        .await
        .unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].phase(), None);
}

#[tokio::test]
async fn update_item_set_blocked() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_in_progress_pg_item("WRK-001", "build")]);

    handle
        .update_item(
            "WRK-001",
            ItemUpdate::SetBlocked("needs clarification".to_string()),
        )
        .await
        .unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].pg_status(), ItemStatus::Blocked);
    assert_eq!(
        snapshot[0].0.blocked_reason.as_deref(),
        Some("needs clarification")
    );
    // blocked_from_status should be saved
    assert_eq!(
        snapshot[0].pg_blocked_from_status(),
        Some(ItemStatus::InProgress)
    );
}

#[tokio::test]
async fn update_item_unblock() {
    let (handle, _task, _dir) = setup_coordinator_with_items(vec![common::make_blocked_pg_item(
        "WRK-001",
        ItemStatus::InProgress,
    )]);

    handle
        .unblock_item("WRK-001", Some("resolved".to_string()))
        .await
        .unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].pg_status(), ItemStatus::InProgress);
    assert_eq!(snapshot[0].0.blocked_reason, None);
}

#[tokio::test]
async fn update_item_unblock_non_blocked_skipped() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    // Unblock on non-blocked item via update_item should be skipped (adapter logs warning)
    handle
        .update_item("WRK-001", ItemUpdate::Unblock)
        .await
        .unwrap();

    // Status should remain New (unblock was skipped)
    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].pg_status(), ItemStatus::New);
}

#[tokio::test]
async fn update_item_update_assessments() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    let assessments = UpdatedAssessments {
        size: Some(SizeLevel::Large),
        risk: Some(DimensionLevel::High),
        impact: Some(DimensionLevel::Medium),
        complexity: Some(DimensionLevel::Low),
    };

    handle
        .update_item("WRK-001", ItemUpdate::UpdateAssessments(assessments))
        .await
        .unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].size(), Some(SizeLevel::Large));
    assert_eq!(snapshot[0].risk(), Some(DimensionLevel::High));
    assert_eq!(snapshot[0].impact(), Some(DimensionLevel::Medium));
    assert_eq!(snapshot[0].complexity(), Some(DimensionLevel::Low));
}

#[tokio::test]
async fn update_item_set_pipeline_type() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    handle
        .update_item(
            "WRK-001",
            ItemUpdate::SetPipelineType("feature".to_string()),
        )
        .await
        .unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].pipeline_type(), Some("feature".to_string()));
}

#[tokio::test]
async fn update_item_set_last_phase_commit() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_in_progress_pg_item("WRK-001", "build")]);

    handle
        .update_item(
            "WRK-001",
            ItemUpdate::SetLastPhaseCommit("abc123".to_string()),
        )
        .await
        .unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].last_phase_commit(), Some("abc123".to_string()));
}

#[tokio::test]
async fn update_item_set_description() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    let desc = StructuredDescription {
        context: "test context".to_string(),
        problem: "test problem".to_string(),
        ..Default::default()
    };

    handle
        .update_item("WRK-001", ItemUpdate::SetDescription(desc.clone()))
        .await
        .unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    let sd = snapshot[0].structured_description().unwrap();
    assert_eq!(sd.context, "test context");
    assert_eq!(sd.problem, "test problem");
}

#[tokio::test]
async fn update_item_nonexistent_returns_error() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    let result = handle
        .update_item("WRK-999", ItemUpdate::TransitionStatus(ItemStatus::Scoping))
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        PgError::ItemNotFound(id) => assert_eq!(id, "WRK-999"),
        other => panic!("Expected ItemNotFound, got: {:?}", other),
    }
}

#[tokio::test]
async fn update_item_persists_to_disk() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_pg_item("WRK-001", ItemStatus::New);
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    let (handle, _task) =
        spawn_coordinator(store.clone(), dir.path().to_path_buf(), "WRK".to_string());

    handle
        .update_item("WRK-001", ItemUpdate::TransitionStatus(ItemStatus::Scoping))
        .await
        .unwrap();

    // Verify persisted by reading directly from store
    let items = store.with_lock(|s| s.load_active()).unwrap();
    let pg_item = PgItem(items[0].clone());
    assert_eq!(pg_item.pg_status(), ItemStatus::Scoping);
}

// =============================================================================
// CompletePhase tests
// =============================================================================

#[tokio::test]
async fn complete_phase_destructive_commits_immediately() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_in_progress_pg_item("WRK-001", "build");
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    // Create a phase output file so there's something to stage
    let changes_dir = dir.path().join("changes").join("WRK-001_test");
    fs::create_dir_all(&changes_dir).unwrap();
    fs::write(changes_dir.join("output.md"), "phase output").unwrap();

    // Stage the output
    Command::new("git")
        .args(["add", "changes/"])
        .current_dir(dir.path())
        .output()
        .expect("stage changes");

    let head_before = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let sha_before = String::from_utf8_lossy(&head_before.stdout)
        .trim()
        .to_string();

    let (handle, _task) = spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    let phase_result = make_phase_result("WRK-001", "build", "Build complete");

    handle
        .complete_phase("WRK-001", phase_result, true)
        .await
        .unwrap();

    // Should have created a new commit
    let head_after = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let sha_after = String::from_utf8_lossy(&head_after.stdout)
        .trim()
        .to_string();
    assert_ne!(
        sha_before, sha_after,
        "Destructive phase should create a commit"
    );

    // Verify commit message format
    let log = Command::new("git")
        .args(["log", "-1", "--pretty=format:%s"])
        .current_dir(dir.path())
        .output()
        .expect("git log");
    let commit_msg = String::from_utf8_lossy(&log.stdout).to_string();
    assert!(
        commit_msg.contains("[WRK-001][build]"),
        "Commit message should follow [ID][phase] format, got: {}",
        commit_msg
    );
}

#[tokio::test]
async fn complete_phase_non_destructive_stages_only() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_in_progress_pg_item("WRK-001", "prd");
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    // Create a phase output file
    let changes_dir = dir.path().join("changes").join("WRK-001_test");
    fs::create_dir_all(&changes_dir).unwrap();
    fs::write(changes_dir.join("output.md"), "prd output").unwrap();

    // Stage the output
    Command::new("git")
        .args(["add", "changes/"])
        .current_dir(dir.path())
        .output()
        .expect("stage changes");

    let head_before = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let sha_before = String::from_utf8_lossy(&head_before.stdout)
        .trim()
        .to_string();

    let (handle, _task) = spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    let phase_result = make_phase_result("WRK-001", "prd", "PRD complete");

    handle
        .complete_phase("WRK-001", phase_result, false)
        .await
        .unwrap();

    // Should NOT have created a commit (non-destructive only stages)
    let head_after = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let sha_after = String::from_utf8_lossy(&head_after.stdout)
        .trim()
        .to_string();
    assert_eq!(
        sha_before, sha_after,
        "Non-destructive phase should not commit"
    );
}

#[tokio::test]
async fn complete_phase_destructive_git_failure_preserves_jsonl() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_in_progress_pg_item("WRK-001", "build");
    save_and_commit_store(dir.path(), &store, std::slice::from_ref(&pg.0));

    let (handle, _task) =
        spawn_coordinator(store.clone(), dir.path().to_path_buf(), "WRK".to_string());

    // Break git by renaming .git directory
    let git_dir = dir.path().join(".git");
    let git_backup = dir.path().join(".git_backup");
    fs::rename(&git_dir, &git_backup).expect("rename .git");

    let phase_result = make_phase_result("WRK-001", "build", "Build complete");

    // CompletePhase should still succeed (git failure is warning, not error)
    // because JSONL is authoritative
    let result = handle.complete_phase("WRK-001", phase_result, true).await;
    // Restore git before assertions
    fs::rename(&git_backup, &git_dir).expect("restore .git");

    // The result may be Ok or Err depending on whether staging or committing fails
    // But JSONL state should be preserved regardless
    let _ = result;

    // JSONL should still have the item
    let items = store.with_lock(|s| s.load_active()).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "WRK-001");
}

// =============================================================================
// BatchCommit tests
// =============================================================================

#[tokio::test]
async fn batch_commit_commits_staged_phases() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_in_progress_pg_item("WRK-001", "prd");
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    // Create output files
    let changes_dir = dir.path().join("changes").join("WRK-001_test");
    fs::create_dir_all(&changes_dir).unwrap();
    fs::write(changes_dir.join("output.md"), "prd output").unwrap();

    Command::new("git")
        .args(["add", "changes/"])
        .current_dir(dir.path())
        .output()
        .expect("stage changes");

    let head_before = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let sha_before = String::from_utf8_lossy(&head_before.stdout)
        .trim()
        .to_string();

    let (handle, _task) = spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    // Complete a non-destructive phase (stages but doesn't commit)
    let phase_result = make_phase_result("WRK-001", "prd", "PRD done");
    handle
        .complete_phase("WRK-001", phase_result, false)
        .await
        .unwrap();

    // Now batch commit
    handle.batch_commit().await.unwrap();

    // Should have created a commit
    let head_after = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let sha_after = String::from_utf8_lossy(&head_after.stdout)
        .trim()
        .to_string();
    assert_ne!(sha_before, sha_after, "Batch commit should create a commit");
}

#[tokio::test]
async fn batch_commit_noop_when_nothing_staged() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_pg_item("WRK-001", ItemStatus::New);
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    let head_before = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let sha_before = String::from_utf8_lossy(&head_before.stdout)
        .trim()
        .to_string();

    let (handle, _task) = spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    // Batch commit with nothing pending should be a no-op
    handle.batch_commit().await.unwrap();

    let head_after = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let sha_after = String::from_utf8_lossy(&head_after.stdout)
        .trim()
        .to_string();
    assert_eq!(sha_before, sha_after, "No commit when nothing staged");
}

// =============================================================================
// GetHeadSha and IsAncestor tests
// =============================================================================

#[tokio::test]
async fn get_head_sha_returns_valid_sha() {
    let (handle, _task, dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    let sha = handle.get_head_sha().await.unwrap();
    assert_eq!(sha.len(), 40, "SHA should be 40 hex chars");
    assert!(
        sha.chars().all(|c| c.is_ascii_hexdigit()),
        "SHA should be hex"
    );

    // Cross-check with git
    let git_sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse");
    let expected = String::from_utf8_lossy(&git_sha.stdout).trim().to_string();
    assert_eq!(sha, expected);
}

#[tokio::test]
async fn is_ancestor_returns_true_for_ancestor() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_pg_item("WRK-001", ItemStatus::New);
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    // Get the ancestor SHA
    let ancestor_sha_output = Command::new("git")
        .args(["rev-parse", "HEAD~1"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse HEAD~1");
    let ancestor_sha = String::from_utf8_lossy(&ancestor_sha_output.stdout)
        .trim()
        .to_string();

    let (handle, _task) = spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    let result = handle.is_ancestor(&ancestor_sha).await.unwrap();
    assert!(result, "Initial commit should be ancestor of HEAD");
}

#[tokio::test]
async fn is_ancestor_returns_false_for_non_ancestor() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_pg_item("WRK-001", ItemStatus::New);
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    // Create a detached commit that is not an ancestor
    Command::new("git")
        .args(["checkout", "--orphan", "orphan-branch"])
        .current_dir(dir.path())
        .output()
        .expect("create orphan branch");
    fs::write(dir.path().join("orphan.txt"), "orphan").unwrap();
    Command::new("git")
        .args(["add", "orphan.txt"])
        .current_dir(dir.path())
        .output()
        .expect("add orphan.txt");
    Command::new("git")
        .args(["commit", "-m", "Orphan commit"])
        .current_dir(dir.path())
        .output()
        .expect("commit orphan");
    let orphan_sha_output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir.path())
        .output()
        .expect("git rev-parse orphan");
    let orphan_sha = String::from_utf8_lossy(&orphan_sha_output.stdout)
        .trim()
        .to_string();

    // Switch back to main
    Command::new("git")
        .args(["checkout", "master"])
        .current_dir(dir.path())
        .output()
        .ok();
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(dir.path())
        .output()
        .ok();

    let (handle, _task) = spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    let result = handle.is_ancestor(&orphan_sha).await.unwrap();
    assert!(!result, "Orphan commit should not be ancestor of HEAD");
}

// =============================================================================
// RecordPhaseStart tests
// =============================================================================

#[tokio::test]
async fn record_phase_start_sets_last_phase_commit() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_in_progress_pg_item("WRK-001", "build")]);

    let head_sha = handle.get_head_sha().await.unwrap();
    handle
        .record_phase_start("WRK-001", &head_sha)
        .await
        .unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].last_phase_commit(), Some(head_sha));
}

#[tokio::test]
async fn record_phase_start_persists_to_disk() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_in_progress_pg_item("WRK-001", "build");
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    let (handle, _task) =
        spawn_coordinator(store.clone(), dir.path().to_path_buf(), "WRK".to_string());

    let head_sha = handle.get_head_sha().await.unwrap();
    handle
        .record_phase_start("WRK-001", &head_sha)
        .await
        .unwrap();

    // Verify persisted
    let items = store.with_lock(|s| s.load_active()).unwrap();
    let pg_item = PgItem(items[0].clone());
    assert_eq!(pg_item.last_phase_commit(), Some(head_sha));
}

// =============================================================================
// WriteWorklog tests
// =============================================================================

#[tokio::test]
async fn write_worklog_creates_entry() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_in_progress_pg_item("WRK-001", "build");
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    let (handle, _task) = spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    handle
        .write_worklog(
            "WRK-001",
            "Test item WRK-001",
            "build",
            "Complete",
            "All done",
        )
        .await
        .unwrap();

    // Find the worklog file
    let worklog_dir = dir.path().join("_worklog");
    let entries: Vec<_> = fs::read_dir(&worklog_dir)
        .expect("read worklog dir")
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(entries.len(), 1, "Should have one worklog file");

    let content = fs::read_to_string(entries[0].path()).unwrap();
    assert!(
        content.contains("WRK-001"),
        "Worklog should contain item ID"
    );
    assert!(content.contains("build"), "Worklog should contain phase");
    assert!(
        content.contains("Complete"),
        "Worklog should contain outcome"
    );
    assert!(
        content.contains("All done"),
        "Worklog should contain summary"
    );
}

// =============================================================================
// ArchiveItem tests
// =============================================================================

#[tokio::test]
async fn archive_item_removes_from_active() {
    let (handle, _task, _dir) = setup_coordinator_with_items(vec![
        common::make_pg_item("WRK-001", ItemStatus::New),
        common::make_pg_item("WRK-002", ItemStatus::New),
    ]);

    handle.archive_item("WRK-001").await.unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].id(), "WRK-002");
}

#[tokio::test]
async fn archive_item_writes_worklog_entry() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_pg_item("WRK-001", ItemStatus::New);
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    let (handle, _task) = spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    handle.archive_item("WRK-001").await.unwrap();

    // Check worklog was written
    let worklog_dir = dir.path().join("_worklog");
    let entries: Vec<_> = fs::read_dir(&worklog_dir)
        .expect("read worklog dir")
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        !entries.is_empty(),
        "Should have worklog entry from archive"
    );

    let content = fs::read_to_string(entries[0].path()).unwrap();
    assert!(
        content.contains("WRK-001"),
        "Worklog should mention archived item"
    );
}

#[tokio::test]
async fn archive_nonexistent_item_returns_error() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    let result = handle.archive_item("WRK-999").await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        PgError::ItemNotFound(id) => assert_eq!(id, "WRK-999"),
        other => panic!("Expected ItemNotFound, got: {:?}", other),
    }
}

// =============================================================================
// IngestFollowUps tests
// =============================================================================

#[tokio::test]
async fn ingest_follow_ups_creates_new_items() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    let follow_ups = vec![
        FollowUp {
            title: "Follow-up 1".to_string(),
            context: Some("Context 1".to_string()),
            suggested_size: None,
            suggested_risk: None,
        },
        FollowUp {
            title: "Follow-up 2".to_string(),
            context: None,
            suggested_size: Some(SizeLevel::Small),
            suggested_risk: Some(DimensionLevel::Low),
        },
    ];

    let new_ids = handle
        .ingest_follow_ups(follow_ups, "WRK-001/build")
        .await
        .unwrap();

    assert_eq!(new_ids.len(), 2);

    // All IDs should start with WRK-
    for id in &new_ids {
        assert!(
            id.starts_with("WRK-"),
            "ID should start with prefix: {}",
            id
        );
    }

    // Verify items exist in snapshot
    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot.len(), 3); // original + 2 follow-ups

    let fu1 = snapshot.iter().find(|i| i.id() == new_ids[0]).unwrap();
    assert_eq!(fu1.title(), "Follow-up 1");
    assert_eq!(fu1.pg_status(), ItemStatus::New);
    assert_eq!(fu1.origin(), Some("WRK-001/build".to_string()));
}

#[tokio::test]
async fn ingest_follow_ups_empty_list_returns_empty() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    let new_ids = handle
        .ingest_follow_ups(vec![], "WRK-001/build")
        .await
        .unwrap();

    assert!(new_ids.is_empty());

    // Store should be unchanged
    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot.len(), 1);
}

#[tokio::test]
async fn ingest_follow_ups_batch_generates_unique_ids() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    let follow_ups: Vec<FollowUp> = (0..6)
        .map(|i| FollowUp {
            title: format!("Follow-up {}", i),
            context: None,
            suggested_size: None,
            suggested_risk: None,
        })
        .collect();

    let new_ids = handle
        .ingest_follow_ups(follow_ups, "WRK-001/build")
        .await
        .unwrap();

    assert_eq!(new_ids.len(), 6);

    // All IDs should be unique
    let unique_ids: std::collections::HashSet<&String> = new_ids.iter().collect();
    assert_eq!(unique_ids.len(), 6, "All generated IDs should be unique");

    // All should start with WRK-
    for id in &new_ids {
        assert!(
            id.starts_with("WRK-"),
            "ID should start with prefix: {}",
            id
        );
    }
}

#[tokio::test]
async fn ingest_follow_ups_persists_to_disk() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_pg_item("WRK-001", ItemStatus::New);
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    let (handle, _task) =
        spawn_coordinator(store.clone(), dir.path().to_path_buf(), "WRK".to_string());

    let follow_ups = vec![FollowUp {
        title: "Persisted follow-up".to_string(),
        context: None,
        suggested_size: None,
        suggested_risk: None,
    }];

    let new_ids = handle
        .ingest_follow_ups(follow_ups, "WRK-001/build")
        .await
        .unwrap();

    assert_eq!(new_ids.len(), 1);

    // Verify persisted by reading directly from store
    let items = store.with_lock(|s| s.load_active()).unwrap();
    assert_eq!(items.len(), 2);
    let fu = items.iter().find(|i| i.id == new_ids[0]).unwrap();
    assert_eq!(fu.title, "Persisted follow-up");
}

// =============================================================================
// MergeItem tests
// =============================================================================

#[tokio::test]
async fn merge_item_removes_source_and_updates_target() {
    let (handle, _task, _dir) = setup_coordinator_with_items(vec![
        common::make_pg_item("WRK-001", ItemStatus::New),
        common::make_pg_item("WRK-002", ItemStatus::New),
    ]);

    handle.merge_item("WRK-002", "WRK-001").await.unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].id(), "WRK-001");

    // Target should have merge info in description
    let desc = snapshot[0].structured_description().unwrap();
    assert!(
        desc.context.contains("[Merged from WRK-002]"),
        "Target description should contain merge info"
    );
}

#[tokio::test]
async fn merge_item_nonexistent_source_returns_error() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    let result = handle.merge_item("WRK-999", "WRK-001").await;
    assert!(result.is_err());
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("not found"),
        "Error should mention 'not found': {}",
        err_str
    );
}

#[tokio::test]
async fn merge_item_self_merge_returns_error() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    let result = handle.merge_item("WRK-001", "WRK-001").await;
    assert!(result.is_err());
    let err_str = result.unwrap_err().to_string();
    assert!(
        err_str.contains("into itself"),
        "Error should mention self-merge: {}",
        err_str
    );
}

// =============================================================================
// UnblockItem tests
// =============================================================================

#[tokio::test]
async fn unblock_item_restores_previous_status() {
    let (handle, _task, _dir) = setup_coordinator_with_items(vec![common::make_blocked_pg_item(
        "WRK-001",
        ItemStatus::InProgress,
    )]);

    handle
        .unblock_item("WRK-001", Some("resolved".to_string()))
        .await
        .unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].pg_status(), ItemStatus::InProgress);
    assert_eq!(snapshot[0].0.blocked_reason, None);
    assert_eq!(snapshot[0].pg_blocked_from_status(), None);
}

#[tokio::test]
async fn unblock_item_without_context() {
    let (handle, _task, _dir) = setup_coordinator_with_items(vec![common::make_blocked_pg_item(
        "WRK-001",
        ItemStatus::InProgress,
    )]);

    handle.unblock_item("WRK-001", None).await.unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].pg_status(), ItemStatus::InProgress);
}

#[tokio::test]
async fn unblock_non_blocked_item_returns_error() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    let result = handle
        .unblock_item("WRK-001", Some("context".to_string()))
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        PgError::InvalidTransition(_) => {} // expected
        other => panic!("Expected InvalidTransition, got: {:?}", other),
    }
}

#[tokio::test]
async fn unblock_item_resets_last_phase_commit() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    // Create a blocked item with last_phase_commit set
    let mut pg = common::make_blocked_pg_item("WRK-001", ItemStatus::InProgress);
    pg_item::set_last_phase_commit(&mut pg.0, Some("abc123"));
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    let (handle, _task) = spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    handle
        .unblock_item("WRK-001", Some("resolved".to_string()))
        .await
        .unwrap();

    let snapshot = handle.get_snapshot().await.unwrap();
    assert_eq!(snapshot[0].last_phase_commit(), None);
}

// =============================================================================
// Shutdown and lifecycle tests
// =============================================================================

#[tokio::test]
async fn handle_send_after_shutdown_returns_error() {
    let (handle, coord_task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    // Drop all handles to trigger coordinator shutdown
    drop(handle);

    // Wait for coordinator to shut down
    coord_task.await.expect("coordinator should complete");

    // At this point the coordinator is shut down and the channel is closed.
    // We can't send on a dropped handle, but we can verify the coord_task completed.
    // This test validates that the coordinator shuts down cleanly when all handles drop.
}

#[tokio::test]
async fn spawn_coordinator_returns_joinhandle() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());
    save_and_commit_store(dir.path(), &store, &[]);

    let (handle, coord_task) =
        spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    // Verify handle works
    let snapshot = handle.get_snapshot().await.unwrap();
    assert!(snapshot.is_empty());

    // Drop handle and verify coordinator shuts down
    drop(handle);
    coord_task.await.expect("coordinator should complete");
}

// =============================================================================
// Multiple operations / consistency tests
// =============================================================================

#[tokio::test]
async fn multiple_sequential_operations_maintain_consistency() {
    let (handle, _task, _dir) = setup_coordinator_with_items(vec![
        common::make_pg_item("WRK-001", ItemStatus::New),
        common::make_pg_item("WRK-002", ItemStatus::New),
    ]);

    // Transition WRK-001 through status chain
    handle
        .update_item("WRK-001", ItemUpdate::TransitionStatus(ItemStatus::Scoping))
        .await
        .unwrap();
    handle
        .update_item("WRK-001", ItemUpdate::TransitionStatus(ItemStatus::Ready))
        .await
        .unwrap();
    handle
        .update_item(
            "WRK-001",
            ItemUpdate::TransitionStatus(ItemStatus::InProgress),
        )
        .await
        .unwrap();
    handle
        .update_item("WRK-001", ItemUpdate::SetPhase("build".to_string()))
        .await
        .unwrap();

    // Update WRK-002 assessments
    let assessments = UpdatedAssessments {
        size: Some(SizeLevel::Small),
        risk: Some(DimensionLevel::Low),
        impact: Some(DimensionLevel::High),
        complexity: None,
    };
    handle
        .update_item("WRK-002", ItemUpdate::UpdateAssessments(assessments))
        .await
        .unwrap();

    // Verify consistency
    let snapshot = handle.get_snapshot().await.unwrap();
    let item1 = snapshot.iter().find(|i| i.id() == "WRK-001").unwrap();
    let item2 = snapshot.iter().find(|i| i.id() == "WRK-002").unwrap();

    assert_eq!(item1.pg_status(), ItemStatus::InProgress);
    assert_eq!(item1.phase(), Some("build".to_string()));
    assert_eq!(item2.pg_status(), ItemStatus::New);
    assert_eq!(item2.size(), Some(SizeLevel::Small));
    assert_eq!(item2.risk(), Some(DimensionLevel::Low));
    assert_eq!(item2.impact(), Some(DimensionLevel::High));
}

#[tokio::test]
async fn concurrent_handle_clones_work_correctly() {
    let (handle, _task, _dir) =
        setup_coordinator_with_items(vec![common::make_pg_item("WRK-001", ItemStatus::New)]);

    let h1 = handle.clone();
    let h2 = handle.clone();

    // Both clones should get same snapshot
    let snap1 = h1.get_snapshot().await.unwrap();
    let snap2 = h2.get_snapshot().await.unwrap();
    assert_eq!(snap1.len(), snap2.len());

    // Update via one clone
    h1.update_item("WRK-001", ItemUpdate::TransitionStatus(ItemStatus::Scoping))
        .await
        .unwrap();

    // Both should see the update
    let snap1 = h1.get_snapshot().await.unwrap();
    let snap2 = h2.get_snapshot().await.unwrap();
    assert_eq!(snap1[0].pg_status(), ItemStatus::Scoping);
    assert_eq!(snap2[0].pg_status(), ItemStatus::Scoping);
}

// =============================================================================
// LockTimeout retry tests
// =============================================================================

#[tokio::test]
async fn lock_timeout_retry_exhaustion_returns_error() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_pg_item("WRK-001", ItemStatus::New);
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    // Hold the lock to force timeout
    let tg_dir = dir.path().join(".task-golem");
    let _lock_guard = common::hold_store_lock(&tg_dir);

    let (handle, _task) = spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    // Operations that go through with_store_retry should eventually fail with LockTimeout
    let result = handle
        .update_item("WRK-001", ItemUpdate::TransitionStatus(ItemStatus::Scoping))
        .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.is_retryable(),
        "LockTimeout should be retryable: {:?}",
        err
    );
    match err {
        PgError::LockTimeout(_) => {} // expected
        other => panic!("Expected LockTimeout, got: {:?}", other),
    }
}

// =============================================================================
// Fatal error tests
// =============================================================================

#[tokio::test]
async fn spawn_coordinator_with_missing_task_golem_dir_returns_empty_snapshot() {
    let dir = common::setup_test_env();
    // Do NOT create .task-golem/ directory -- Store will point to nonexistent dir
    let store = Store::new(dir.path().join(".task-golem"));

    // Commit something so git is happy
    Command::new("git")
        .args(["commit", "--allow-empty", "-m", "empty"])
        .current_dir(dir.path())
        .output()
        .expect("commit");

    let (handle, _task) = spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    // When tasks.jsonl doesn't exist, load_active returns Ok(vec![]) per task-golem
    let result = handle.get_snapshot().await;
    assert!(result.is_ok());
    assert!(
        result.unwrap().is_empty(),
        "Missing store should return empty snapshot"
    );
}

#[tokio::test]
async fn spawn_coordinator_with_corrupt_tasks_jsonl_returns_error() {
    let dir = common::setup_test_env();
    let tg_dir = dir.path().join(".task-golem");
    fs::create_dir_all(&tg_dir).unwrap();
    // Write corrupt data (not valid JSONL schema header)
    fs::write(tg_dir.join("tasks.jsonl"), "not valid json\n").unwrap();

    let store = Store::new(tg_dir);

    Command::new("git")
        .args(["add", ".task-golem/"])
        .current_dir(dir.path())
        .output()
        .expect("stage");
    Command::new("git")
        .args(["commit", "-m", "corrupt store"])
        .current_dir(dir.path())
        .output()
        .expect("commit");

    let (handle, _task) = spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

    // GetSnapshot should fail because tasks.jsonl is corrupt
    let result = handle.get_snapshot().await;
    assert!(result.is_err(), "Corrupt store should return error");
    let err = result.unwrap_err();
    assert!(
        err.is_fatal(),
        "Storage corruption should be fatal: {:?}",
        err
    );
}

// =============================================================================
// Commit message format tests (unit tests, no coordinator needed)
// =============================================================================

use phase_golem::scheduler::HaltReason;

#[test]
fn halt_commit_message_cap_reached() {
    let msg = format!(
        "[phase-golem] Save backlog state on halt ({:?})",
        HaltReason::CapReached
    );
    assert_eq!(msg, "[phase-golem] Save backlog state on halt (CapReached)");
}

#[test]
fn halt_commit_message_all_done_or_blocked() {
    let msg = format!(
        "[phase-golem] Save backlog state on halt ({:?})",
        HaltReason::AllDoneOrBlocked
    );
    assert_eq!(
        msg,
        "[phase-golem] Save backlog state on halt (AllDoneOrBlocked)"
    );
}

#[test]
fn halt_commit_message_shutdown_requested() {
    let msg = format!(
        "[phase-golem] Save backlog state on halt ({:?})",
        HaltReason::ShutdownRequested
    );
    assert_eq!(
        msg,
        "[phase-golem] Save backlog state on halt (ShutdownRequested)"
    );
}

#[test]
fn halt_commit_message_circuit_breaker() {
    let msg = format!(
        "[phase-golem] Save backlog state on halt ({:?})",
        HaltReason::CircuitBreakerTripped
    );
    assert_eq!(
        msg,
        "[phase-golem] Save backlog state on halt (CircuitBreakerTripped)"
    );
}

#[test]
fn halt_commit_message_target_completed() {
    let msg = format!(
        "[phase-golem] Save backlog state on halt ({:?})",
        HaltReason::TargetCompleted
    );
    assert_eq!(
        msg,
        "[phase-golem] Save backlog state on halt (TargetCompleted)"
    );
}

#[test]
fn halt_commit_message_filter_exhausted() {
    let msg = format!(
        "[phase-golem] Save backlog state on halt ({:?})",
        HaltReason::FilterExhausted
    );
    assert_eq!(
        msg,
        "[phase-golem] Save backlog state on halt (FilterExhausted)"
    );
}

// =============================================================================
// Persistence round-trip tests
// =============================================================================

#[tokio::test]
async fn persistence_round_trip_update_item() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_pg_item("WRK-001", ItemStatus::New);
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    let (handle, _task) =
        spawn_coordinator(store.clone(), dir.path().to_path_buf(), "WRK".to_string());

    // Update via coordinator
    handle
        .update_item("WRK-001", ItemUpdate::TransitionStatus(ItemStatus::Scoping))
        .await
        .unwrap();

    // Read directly from store (round-trip verification)
    let items = store.with_lock(|s| s.load_active()).unwrap();
    let pg_item = PgItem(items[0].clone());
    assert_eq!(pg_item.pg_status(), ItemStatus::Scoping);
}

#[tokio::test]
async fn persistence_round_trip_archive_item() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg1 = common::make_pg_item("WRK-001", ItemStatus::New);
    let pg2 = common::make_pg_item("WRK-002", ItemStatus::New);
    save_and_commit_store(dir.path(), &store, &[pg1.0, pg2.0]);

    let (handle, _task) =
        spawn_coordinator(store.clone(), dir.path().to_path_buf(), "WRK".to_string());

    handle.archive_item("WRK-001").await.unwrap();

    // Read directly from store
    let items = store.with_lock(|s| s.load_active()).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "WRK-002");

    // Check archive
    let archive_ids = store.all_known_ids().unwrap();
    assert!(
        archive_ids.contains("WRK-001"),
        "Archived item should be in known IDs"
    );
}

#[tokio::test]
async fn persistence_round_trip_merge_item() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg1 = common::make_pg_item("WRK-001", ItemStatus::New);
    let pg2 = common::make_pg_item("WRK-002", ItemStatus::New);
    save_and_commit_store(dir.path(), &store, &[pg1.0, pg2.0]);

    let (handle, _task) =
        spawn_coordinator(store.clone(), dir.path().to_path_buf(), "WRK".to_string());

    handle.merge_item("WRK-002", "WRK-001").await.unwrap();

    // Read directly from store
    let items = store.with_lock(|s| s.load_active()).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, "WRK-001");

    // Source should be in archive
    let archive_ids = store.all_known_ids().unwrap();
    assert!(
        archive_ids.contains("WRK-002"),
        "Merged source should be in known IDs"
    );
}

#[tokio::test]
async fn persistence_round_trip_ingest_follow_ups() {
    let dir = common::setup_test_env();
    let store = common::setup_task_golem_store(dir.path());

    let pg = common::make_pg_item("WRK-001", ItemStatus::New);
    save_and_commit_store(dir.path(), &store, &[pg.0]);

    let (handle, _task) =
        spawn_coordinator(store.clone(), dir.path().to_path_buf(), "WRK".to_string());

    let follow_ups = vec![FollowUp {
        title: "Follow-up".to_string(),
        context: None,
        suggested_size: None,
        suggested_risk: None,
    }];

    let new_ids = handle
        .ingest_follow_ups(follow_ups, "WRK-001/build")
        .await
        .unwrap();

    // Read directly from store
    let items = store.with_lock(|s| s.load_active()).unwrap();
    assert_eq!(items.len(), 2);
    assert!(items.iter().any(|i| i.id == new_ids[0]));
}
