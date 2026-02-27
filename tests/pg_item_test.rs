use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use task_golem::model::item::Item;
use task_golem::model::status::Status;
use task_golem::store::Store;

use phase_golem::pg_item::{self, PgItem};
use phase_golem::types::{
    BlockType, DimensionLevel, ItemStatus, ItemUpdate, PhasePool, SizeLevel, StructuredDescription,
    UpdatedAssessments,
};

// --- Helpers ---

fn make_test_item() -> Item {
    let now = DateTime::parse_from_rfc3339("2026-02-26T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    Item {
        id: "WRK-a1b2c".to_string(),
        title: "Test item".to_string(),
        status: Status::Todo,
        priority: 0,
        description: None,
        tags: vec!["backend".to_string()],
        dependencies: vec!["WRK-dep1".to_string()],
        created_at: now,
        updated_at: now,
        blocked_reason: None,
        blocked_from_status: None,
        claimed_by: None,
        claimed_at: None,
        extensions: BTreeMap::new(),
    }
}

fn make_item_with_ext(key: &str, value: serde_json::Value) -> Item {
    let mut item = make_test_item();
    item.extensions.insert(key.to_string(), value);
    item
}

// =====================================================================
// Status mapping tests
// =====================================================================

#[test]
fn pg_status_todo_absent_defaults_to_new() {
    let item = make_test_item(); // Todo, no x-pg-status
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::New);
}

#[test]
fn pg_status_todo_with_new() {
    let item = make_item_with_ext("x-pg-status", serde_json::json!("new"));
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::New);
}

#[test]
fn pg_status_todo_with_scoping() {
    let item = make_item_with_ext("x-pg-status", serde_json::json!("scoping"));
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::Scoping);
}

#[test]
fn pg_status_todo_with_ready() {
    let item = make_item_with_ext("x-pg-status", serde_json::json!("ready"));
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::Ready);
}

#[test]
fn pg_status_doing_maps_to_in_progress() {
    let mut item = make_test_item();
    item.status = Status::Doing;
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::InProgress);
}

#[test]
fn pg_status_done_maps_to_done() {
    let mut item = make_test_item();
    item.status = Status::Done;
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::Done);
}

#[test]
fn pg_status_blocked_maps_to_blocked() {
    let mut item = make_test_item();
    item.status = Status::Blocked;
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::Blocked);
}

#[test]
fn pg_status_doing_ignores_stale_extension() {
    let mut item = make_item_with_ext("x-pg-status", serde_json::json!("scoping"));
    item.status = Status::Doing;
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::InProgress);
}

#[test]
fn pg_status_done_ignores_stale_extension() {
    let mut item = make_item_with_ext("x-pg-status", serde_json::json!("new"));
    item.status = Status::Done;
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::Done);
}

#[test]
fn pg_status_blocked_ignores_stale_extension() {
    let mut item = make_item_with_ext("x-pg-status", serde_json::json!("ready"));
    item.status = Status::Blocked;
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::Blocked);
}

#[test]
fn pg_status_invalid_extension_defaults_to_new_with_warning() {
    let item = make_item_with_ext("x-pg-status", serde_json::json!("running"));
    let pg = PgItem(item);
    // Invalid value: should default to New
    assert_eq!(pg.pg_status(), ItemStatus::New);
}

// --- Bidirectional round-trip: all 6 ItemStatus variants ---

#[test]
fn set_pg_status_round_trip_new() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::New);
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::New);
    assert_eq!(pg.status(), Status::Todo);
}

#[test]
fn set_pg_status_round_trip_scoping() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::Scoping);
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::Scoping);
    assert_eq!(pg.status(), Status::Todo);
}

#[test]
fn set_pg_status_round_trip_ready() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::Ready);
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::Ready);
    assert_eq!(pg.status(), Status::Todo);
}

#[test]
fn set_pg_status_round_trip_in_progress() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::InProgress);
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::InProgress);
    assert_eq!(pg.status(), Status::Doing);
}

#[test]
fn set_pg_status_round_trip_done() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::Done);
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::Done);
    assert_eq!(pg.status(), Status::Done);
}

#[test]
fn set_pg_status_round_trip_blocked() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::Blocked);
    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::Blocked);
    assert_eq!(pg.status(), Status::Blocked);
}

#[test]
fn set_pg_status_clears_extension_for_non_todo() {
    let mut item = make_test_item();
    // First set to scoping (writes extension)
    pg_item::set_pg_status(&mut item, ItemStatus::Scoping);
    assert!(item.extensions.contains_key("x-pg-status"));
    // Then set to InProgress (should clear extension)
    pg_item::set_pg_status(&mut item, ItemStatus::InProgress);
    assert!(!item.extensions.contains_key("x-pg-status"));
}

// =====================================================================
// Extension field getter/setter round-trip tests
// =====================================================================

#[test]
fn phase_round_trip() {
    let mut item = make_test_item();
    assert!(PgItem(item.clone()).phase().is_none());

    pg_item::set_phase(&mut item, Some("prd"));
    assert_eq!(PgItem(item.clone()).phase().as_deref(), Some("prd"));

    pg_item::set_phase(&mut item, None);
    assert!(PgItem(item).phase().is_none());
}

#[test]
fn phase_pool_round_trip() {
    let mut item = make_test_item();
    assert!(PgItem(item.clone()).phase_pool().is_none());

    pg_item::set_phase_pool(&mut item, Some(&PhasePool::Pre));
    assert_eq!(PgItem(item.clone()).phase_pool(), Some(PhasePool::Pre));

    pg_item::set_phase_pool(&mut item, Some(&PhasePool::Main));
    assert_eq!(PgItem(item.clone()).phase_pool(), Some(PhasePool::Main));

    pg_item::set_phase_pool(&mut item, None);
    assert!(PgItem(item).phase_pool().is_none());
}

#[test]
fn size_round_trip() {
    let mut item = make_test_item();
    assert!(PgItem(item.clone()).size().is_none());

    pg_item::set_size(&mut item, Some(&SizeLevel::Small));
    assert_eq!(PgItem(item.clone()).size(), Some(SizeLevel::Small));

    pg_item::set_size(&mut item, Some(&SizeLevel::Medium));
    assert_eq!(PgItem(item.clone()).size(), Some(SizeLevel::Medium));

    pg_item::set_size(&mut item, Some(&SizeLevel::Large));
    assert_eq!(PgItem(item.clone()).size(), Some(SizeLevel::Large));

    pg_item::set_size(&mut item, None);
    assert!(PgItem(item).size().is_none());
}

#[test]
fn complexity_round_trip() {
    let mut item = make_test_item();
    pg_item::set_complexity(&mut item, Some(&DimensionLevel::Low));
    assert_eq!(PgItem(item.clone()).complexity(), Some(DimensionLevel::Low));

    pg_item::set_complexity(&mut item, Some(&DimensionLevel::High));
    assert_eq!(
        PgItem(item.clone()).complexity(),
        Some(DimensionLevel::High)
    );

    pg_item::set_complexity(&mut item, None);
    assert!(PgItem(item).complexity().is_none());
}

#[test]
fn risk_round_trip() {
    let mut item = make_test_item();
    pg_item::set_risk(&mut item, Some(&DimensionLevel::Medium));
    assert_eq!(PgItem(item.clone()).risk(), Some(DimensionLevel::Medium));

    pg_item::set_risk(&mut item, None);
    assert!(PgItem(item).risk().is_none());
}

#[test]
fn impact_round_trip() {
    let mut item = make_test_item();
    pg_item::set_impact(&mut item, Some(&DimensionLevel::High));
    assert_eq!(PgItem(item.clone()).impact(), Some(DimensionLevel::High));

    pg_item::set_impact(&mut item, None);
    assert!(PgItem(item).impact().is_none());
}

#[test]
fn requires_human_review_round_trip() {
    let mut item = make_test_item();
    assert!(!PgItem(item.clone()).requires_human_review());

    pg_item::set_requires_human_review(&mut item, true);
    assert!(PgItem(item.clone()).requires_human_review());

    pg_item::set_requires_human_review(&mut item, false);
    assert!(!PgItem(item).requires_human_review());
}

#[test]
fn pipeline_type_round_trip() {
    let mut item = make_test_item();
    pg_item::set_pipeline_type(&mut item, Some("feature"));
    assert_eq!(
        PgItem(item.clone()).pipeline_type().as_deref(),
        Some("feature")
    );

    pg_item::set_pipeline_type(&mut item, None);
    assert!(PgItem(item).pipeline_type().is_none());
}

#[test]
fn origin_round_trip() {
    let mut item = make_test_item();
    pg_item::set_origin(&mut item, Some("WRK-001"));
    assert_eq!(PgItem(item.clone()).origin().as_deref(), Some("WRK-001"));

    pg_item::set_origin(&mut item, None);
    assert!(PgItem(item).origin().is_none());
}

#[test]
fn blocked_type_round_trip() {
    let mut item = make_test_item();
    pg_item::set_blocked_type(&mut item, Some(&BlockType::Clarification));
    assert_eq!(
        PgItem(item.clone()).blocked_type(),
        Some(BlockType::Clarification)
    );

    pg_item::set_blocked_type(&mut item, Some(&BlockType::Decision));
    assert_eq!(
        PgItem(item.clone()).blocked_type(),
        Some(BlockType::Decision)
    );

    pg_item::set_blocked_type(&mut item, None);
    assert!(PgItem(item).blocked_type().is_none());
}

#[test]
fn unblock_context_round_trip() {
    let mut item = make_test_item();
    pg_item::set_unblock_context(&mut item, Some("resolved via discussion"));
    assert_eq!(
        PgItem(item.clone()).unblock_context().as_deref(),
        Some("resolved via discussion")
    );

    pg_item::set_unblock_context(&mut item, None);
    assert!(PgItem(item).unblock_context().is_none());
}

#[test]
fn last_phase_commit_round_trip() {
    let mut item = make_test_item();
    pg_item::set_last_phase_commit(&mut item, Some("abc123"));
    assert_eq!(
        PgItem(item.clone()).last_phase_commit().as_deref(),
        Some("abc123")
    );

    pg_item::set_last_phase_commit(&mut item, None);
    assert!(PgItem(item).last_phase_commit().is_none());
}

// =====================================================================
// StructuredDescription tests
// =====================================================================

#[test]
fn structured_description_round_trip() {
    let desc = StructuredDescription {
        context: "Migration context".to_string(),
        problem: "Too much code".to_string(),
        solution: "Use task-golem".to_string(),
        impact: "Less maintenance".to_string(),
        sizing_rationale: "Medium effort".to_string(),
    };

    let mut item = make_test_item();
    pg_item::set_structured_description(&mut item, Some(&desc));

    let pg = PgItem(item.clone());
    let retrieved = pg
        .structured_description()
        .expect("should have description");
    assert_eq!(retrieved, desc);
}

#[test]
fn structured_description_populates_native_description() {
    let desc = StructuredDescription {
        context: "Native description text".to_string(),
        problem: "problem".to_string(),
        solution: "solution".to_string(),
        impact: "impact".to_string(),
        sizing_rationale: "rationale".to_string(),
    };

    let mut item = make_test_item();
    pg_item::set_structured_description(&mut item, Some(&desc));
    assert_eq!(item.description.as_deref(), Some("Native description text"));
}

#[test]
fn structured_description_empty_context_clears_native() {
    let desc = StructuredDescription {
        context: "".to_string(),
        problem: "problem".to_string(),
        solution: "".to_string(),
        impact: "".to_string(),
        sizing_rationale: "".to_string(),
    };

    let mut item = make_test_item();
    item.description = Some("old description".to_string());
    pg_item::set_structured_description(&mut item, Some(&desc));
    assert!(item.description.is_none());
}

#[test]
fn structured_description_clear_removes_extension_and_native() {
    let desc = StructuredDescription {
        context: "context".to_string(),
        problem: "problem".to_string(),
        solution: "solution".to_string(),
        impact: "impact".to_string(),
        sizing_rationale: "rationale".to_string(),
    };

    let mut item = make_test_item();
    pg_item::set_structured_description(&mut item, Some(&desc));
    assert!(item.extensions.contains_key("x-pg-description"));

    pg_item::set_structured_description(&mut item, None);
    assert!(!item.extensions.contains_key("x-pg-description"));
    assert!(item.description.is_none());
}

#[test]
fn structured_description_corrupt_value_returns_none() {
    // Put a non-object value in x-pg-description
    let item = make_item_with_ext("x-pg-description", serde_json::json!("not an object"));
    let pg = PgItem(item);
    assert!(pg.structured_description().is_none());
}

#[test]
fn structured_description_empty_fields_returns_none() {
    // All empty strings = treated as absent
    let desc = StructuredDescription::default();
    let value = serde_json::to_value(&desc).unwrap();
    let item = make_item_with_ext("x-pg-description", value);
    let pg = PgItem(item);
    assert!(pg.structured_description().is_none());
}

// =====================================================================
// pg_blocked_from_status divergence detection tests
// =====================================================================

#[test]
fn pg_blocked_from_status_normal_case() {
    let mut item = make_test_item();
    item.status = Status::Blocked;
    item.blocked_from_status = Some(Status::Doing);
    item.extensions.insert(
        "x-pg-blocked-from-status".to_string(),
        serde_json::json!("in_progress"),
    );
    let pg = PgItem(item);
    assert_eq!(pg.pg_blocked_from_status(), Some(ItemStatus::InProgress));
}

#[test]
fn pg_blocked_from_status_divergence_native_cleared() {
    // Native cleared (tg unblock ran), extension still present: stale
    let mut item = make_test_item();
    item.blocked_from_status = None;
    item.extensions.insert(
        "x-pg-blocked-from-status".to_string(),
        serde_json::json!("ready"),
    );
    let pg = PgItem(item);
    assert!(pg.pg_blocked_from_status().is_none());
}

#[test]
fn pg_blocked_from_status_no_extension() {
    let mut item = make_test_item();
    item.blocked_from_status = Some(Status::Todo);
    // No extension set
    let pg = PgItem(item);
    assert!(pg.pg_blocked_from_status().is_none());
}

#[test]
fn pg_blocked_from_status_both_absent() {
    let item = make_test_item();
    let pg = PgItem(item);
    assert!(pg.pg_blocked_from_status().is_none());
}

#[test]
fn pg_blocked_from_status_invalid_extension_value() {
    let mut item = make_test_item();
    item.blocked_from_status = Some(Status::Doing);
    item.extensions.insert(
        "x-pg-blocked-from-status".to_string(),
        serde_json::json!("invalid_status"),
    );
    let pg = PgItem(item);
    // Invalid value should be treated as absent
    assert!(pg.pg_blocked_from_status().is_none());
}

// =====================================================================
// Invalid extension value handling tests
// =====================================================================

#[test]
fn invalid_phase_pool_returns_none() {
    let item = make_item_with_ext("x-pg-phase-pool", serde_json::json!("invalid"));
    let pg = PgItem(item);
    assert!(pg.phase_pool().is_none());
}

#[test]
fn invalid_size_returns_none() {
    let item = make_item_with_ext("x-pg-size", serde_json::json!("huge"));
    let pg = PgItem(item);
    assert!(pg.size().is_none());
}

#[test]
fn invalid_dimension_returns_none() {
    let item = make_item_with_ext("x-pg-complexity", serde_json::json!("extreme"));
    let pg = PgItem(item);
    assert!(pg.complexity().is_none());
}

#[test]
fn invalid_blocked_type_returns_none() {
    let item = make_item_with_ext("x-pg-blocked-type", serde_json::json!("unknown"));
    let pg = PgItem(item);
    assert!(pg.blocked_type().is_none());
}

#[test]
fn non_string_extension_value_returns_none() {
    // Numeric value for a string field
    let item = make_item_with_ext("x-pg-phase", serde_json::json!(42));
    let pg = PgItem(item);
    assert!(pg.phase().is_none());
}

#[test]
fn non_bool_requires_human_review_defaults_false() {
    let item = make_item_with_ext("x-pg-requires-human-review", serde_json::json!("yes"));
    let pg = PgItem(item);
    assert!(!pg.requires_human_review());
}

// =====================================================================
// new_from_parts constructor tests
// =====================================================================

#[test]
fn new_from_parts_defaults() {
    let pg = pg_item::new_from_parts(
        "WRK-abc".to_string(),
        "A new item".to_string(),
        ItemStatus::New,
        vec!["WRK-dep1".to_string()],
        vec!["backend".to_string()],
    );

    assert_eq!(pg.id(), "WRK-abc");
    assert_eq!(pg.title(), "A new item");
    assert_eq!(pg.pg_status(), ItemStatus::New);
    assert_eq!(pg.status(), Status::Todo);
    assert_eq!(pg.dependencies(), &["WRK-dep1"]);
    assert_eq!(pg.tags(), &["backend"]);
    assert_eq!(pg.0.priority, 0);
    assert!(pg.0.description.is_none());
    assert!(pg.0.blocked_reason.is_none());
    assert!(pg.0.blocked_from_status.is_none());
    assert!(pg.0.claimed_by.is_none());
    assert!(pg.0.claimed_at.is_none());
}

#[test]
fn new_from_parts_scoping_status() {
    let pg = pg_item::new_from_parts(
        "WRK-def".to_string(),
        "Scoping item".to_string(),
        ItemStatus::Scoping,
        vec![],
        vec![],
    );
    assert_eq!(pg.pg_status(), ItemStatus::Scoping);
    assert_eq!(pg.status(), Status::Todo);
}

#[test]
fn new_from_parts_in_progress() {
    let pg = pg_item::new_from_parts(
        "WRK-ghi".to_string(),
        "In progress item".to_string(),
        ItemStatus::InProgress,
        vec![],
        vec![],
    );
    assert_eq!(pg.pg_status(), ItemStatus::InProgress);
    assert_eq!(pg.status(), Status::Doing);
}

#[test]
fn new_from_parts_timestamps_are_recent() {
    let before = Utc::now();
    let pg = pg_item::new_from_parts(
        "WRK-t1".to_string(),
        "Timestamp test".to_string(),
        ItemStatus::New,
        vec![],
        vec![],
    );
    let after = Utc::now();

    assert!(pg.created_at() >= before && pg.created_at() <= after);
    assert!(pg.updated_at() >= before && pg.updated_at() <= after);
}

// =====================================================================
// apply_update tests
// =====================================================================

#[test]
fn apply_update_transition_status_forward() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::New);

    pg_item::apply_update(&mut item, ItemUpdate::TransitionStatus(ItemStatus::Scoping));
    assert_eq!(PgItem(item.clone()).pg_status(), ItemStatus::Scoping);

    pg_item::apply_update(&mut item, ItemUpdate::TransitionStatus(ItemStatus::Ready));
    assert_eq!(PgItem(item.clone()).pg_status(), ItemStatus::Ready);

    pg_item::apply_update(
        &mut item,
        ItemUpdate::TransitionStatus(ItemStatus::InProgress),
    );
    assert_eq!(PgItem(item.clone()).pg_status(), ItemStatus::InProgress);

    pg_item::apply_update(&mut item, ItemUpdate::TransitionStatus(ItemStatus::Done));
    assert_eq!(PgItem(item).pg_status(), ItemStatus::Done);
}

#[test]
fn apply_update_transition_to_blocked_saves_from_status() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::Ready);

    pg_item::apply_update(&mut item, ItemUpdate::TransitionStatus(ItemStatus::Blocked));
    assert_eq!(PgItem(item.clone()).pg_status(), ItemStatus::Blocked);

    // Should have saved the blocked_from_status
    let pg = PgItem(item);
    assert_eq!(pg.pg_blocked_from_status(), Some(ItemStatus::Ready));
}

#[test]
fn apply_update_transition_from_blocked_clears_fields() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::Ready);

    // Block it
    pg_item::apply_update(&mut item, ItemUpdate::TransitionStatus(ItemStatus::Blocked));
    item.blocked_reason = Some("test reason".to_string());
    pg_item::set_blocked_type(&mut item, Some(&BlockType::Decision));
    pg_item::set_unblock_context(&mut item, Some("context"));

    // Unblock via status transition
    pg_item::apply_update(&mut item, ItemUpdate::TransitionStatus(ItemStatus::Ready));

    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::Ready);
    assert!(pg.pg_blocked_from_status().is_none());
    assert!(pg.0.blocked_reason.is_none());
    assert!(pg.blocked_type().is_none());
    assert!(pg.unblock_context().is_none());
}

#[test]
fn apply_update_invalid_transition_is_skipped() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::Done);

    // Done -> New is invalid
    pg_item::apply_update(&mut item, ItemUpdate::TransitionStatus(ItemStatus::New));
    assert_eq!(PgItem(item).pg_status(), ItemStatus::Done);
}

#[test]
fn apply_update_set_phase() {
    let mut item = make_test_item();
    pg_item::apply_update(&mut item, ItemUpdate::SetPhase("build".to_string()));
    assert_eq!(PgItem(item).phase().as_deref(), Some("build"));
}

#[test]
fn apply_update_set_phase_pool() {
    let mut item = make_test_item();
    pg_item::apply_update(&mut item, ItemUpdate::SetPhasePool(PhasePool::Main));
    assert_eq!(PgItem(item).phase_pool(), Some(PhasePool::Main));
}

#[test]
fn apply_update_clear_phase() {
    let mut item = make_test_item();
    pg_item::set_phase(&mut item, Some("prd"));
    pg_item::set_phase_pool(&mut item, Some(&PhasePool::Pre));

    pg_item::apply_update(&mut item, ItemUpdate::ClearPhase);

    let pg = PgItem(item);
    assert!(pg.phase().is_none());
    assert!(pg.phase_pool().is_none());
}

#[test]
fn apply_update_set_blocked() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::InProgress);

    pg_item::apply_update(
        &mut item,
        ItemUpdate::SetBlocked("need API key".to_string()),
    );

    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::Blocked);
    assert_eq!(pg.0.blocked_reason.as_deref(), Some("need API key"));
    assert_eq!(pg.pg_blocked_from_status(), Some(ItemStatus::InProgress));
}

#[test]
fn apply_update_set_blocked_from_invalid_state_is_skipped() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::Done);

    pg_item::apply_update(&mut item, ItemUpdate::SetBlocked("reason".to_string()));
    assert_eq!(PgItem(item).pg_status(), ItemStatus::Done);
}

#[test]
fn apply_update_unblock() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::Ready);
    pg_item::set_blocked_from_status(&mut item, Some(&ItemStatus::Ready));
    pg_item::set_pg_status(&mut item, ItemStatus::Blocked);
    // Also set blocked_from_status on native side for divergence detection
    item.blocked_from_status = Some(Status::Todo);
    item.blocked_reason = Some("waiting".to_string());
    pg_item::set_blocked_type(&mut item, Some(&BlockType::Clarification));

    pg_item::apply_update(&mut item, ItemUpdate::Unblock);

    let pg = PgItem(item);
    assert_eq!(pg.pg_status(), ItemStatus::Ready);
    assert!(pg.0.blocked_reason.is_none());
    assert!(pg.blocked_type().is_none());
    assert!(pg.unblock_context().is_none());
}

#[test]
fn apply_update_unblock_non_blocked_is_skipped() {
    let mut item = make_test_item();
    pg_item::set_pg_status(&mut item, ItemStatus::InProgress);

    pg_item::apply_update(&mut item, ItemUpdate::Unblock);
    assert_eq!(PgItem(item).pg_status(), ItemStatus::InProgress);
}

#[test]
fn apply_update_unblock_without_saved_from_status_defaults_to_new() {
    let mut item = make_test_item();
    item.status = Status::Blocked;
    item.blocked_from_status = Some(Status::Todo);
    // No x-pg-blocked-from-status extension
    item.blocked_reason = Some("reason".to_string());

    pg_item::apply_update(&mut item, ItemUpdate::Unblock);
    // No pg_blocked_from_status found, defaults to New
    assert_eq!(PgItem(item).pg_status(), ItemStatus::New);
}

#[test]
fn apply_update_update_assessments() {
    let mut item = make_test_item();
    let assessments = UpdatedAssessments {
        size: Some(SizeLevel::Large),
        complexity: Some(DimensionLevel::High),
        risk: Some(DimensionLevel::Medium),
        impact: Some(DimensionLevel::Low),
    };

    pg_item::apply_update(&mut item, ItemUpdate::UpdateAssessments(assessments));

    let pg = PgItem(item);
    assert_eq!(pg.size(), Some(SizeLevel::Large));
    assert_eq!(pg.complexity(), Some(DimensionLevel::High));
    assert_eq!(pg.risk(), Some(DimensionLevel::Medium));
    assert_eq!(pg.impact(), Some(DimensionLevel::Low));
}

#[test]
fn apply_update_update_assessments_partial() {
    let mut item = make_test_item();
    pg_item::set_size(&mut item, Some(&SizeLevel::Small));
    pg_item::set_risk(&mut item, Some(&DimensionLevel::Low));

    let assessments = UpdatedAssessments {
        size: None,                       // keep existing
        complexity: None,                 // keep absent
        risk: Some(DimensionLevel::High), // override
        impact: None,                     // keep absent
    };

    pg_item::apply_update(&mut item, ItemUpdate::UpdateAssessments(assessments));

    let pg = PgItem(item);
    assert_eq!(pg.size(), Some(SizeLevel::Small)); // unchanged
    assert!(pg.complexity().is_none()); // still absent
    assert_eq!(pg.risk(), Some(DimensionLevel::High)); // updated
    assert!(pg.impact().is_none()); // still absent
}

#[test]
fn apply_update_set_pipeline_type() {
    let mut item = make_test_item();
    pg_item::apply_update(
        &mut item,
        ItemUpdate::SetPipelineType("feature".to_string()),
    );
    assert_eq!(PgItem(item).pipeline_type().as_deref(), Some("feature"));
}

#[test]
fn apply_update_set_last_phase_commit() {
    let mut item = make_test_item();
    pg_item::apply_update(
        &mut item,
        ItemUpdate::SetLastPhaseCommit("abc123def".to_string()),
    );
    assert_eq!(
        PgItem(item).last_phase_commit().as_deref(),
        Some("abc123def")
    );
}

#[test]
fn apply_update_set_description() {
    let desc = StructuredDescription {
        context: "Context for this item".to_string(),
        problem: "The problem".to_string(),
        solution: "The solution".to_string(),
        impact: "The impact".to_string(),
        sizing_rationale: "Small".to_string(),
    };

    let mut item = make_test_item();
    pg_item::apply_update(&mut item, ItemUpdate::SetDescription(desc.clone()));

    let pg = PgItem(item);
    assert_eq!(pg.structured_description(), Some(desc));
    assert_eq!(pg.0.description.as_deref(), Some("Context for this item"));
}

// =====================================================================
// Native field delegate tests
// =====================================================================

#[test]
fn native_field_delegates() {
    let item = make_test_item();
    let pg = PgItem(item);

    assert_eq!(pg.id(), "WRK-a1b2c");
    assert_eq!(pg.title(), "Test item");
    assert_eq!(pg.status(), Status::Todo);
    assert_eq!(pg.dependencies(), &["WRK-dep1"]);
    assert_eq!(pg.tags(), &["backend"]);
    assert_eq!(
        pg.created_at(),
        DateTime::parse_from_rfc3339("2026-02-26T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    );
}

// =====================================================================
// JSONL round-trip integration test via Store
// =====================================================================

#[test]
fn jsonl_round_trip_all_extensions() {
    let tmp = tempfile::tempdir().unwrap();
    let store_dir = tmp.path().join(".task-golem");
    std::fs::create_dir_all(&store_dir).unwrap();

    let store = Store::new(store_dir);

    // Create a fully-populated PgItem with all 15 extension fields set
    let mut pg = pg_item::new_from_parts(
        "WRK-round".to_string(),
        "JSONL round-trip test".to_string(),
        ItemStatus::Scoping,
        vec!["WRK-dep1".to_string()],
        vec!["test".to_string()],
    );

    // Set all extension fields on the inner Item
    let item = &mut pg.0;
    pg_item::set_phase(item, Some("build"));
    pg_item::set_phase_pool(item, Some(&PhasePool::Main));
    pg_item::set_size(item, Some(&SizeLevel::Medium));
    pg_item::set_complexity(item, Some(&DimensionLevel::High));
    pg_item::set_risk(item, Some(&DimensionLevel::Low));
    pg_item::set_impact(item, Some(&DimensionLevel::Medium));
    pg_item::set_requires_human_review(item, true);
    pg_item::set_pipeline_type(item, Some("feature"));
    pg_item::set_origin(item, Some("WRK-001"));
    pg_item::set_blocked_type(item, Some(&BlockType::Decision));
    pg_item::set_blocked_from_status(item, Some(&ItemStatus::Ready));
    pg_item::set_unblock_context(item, Some("discussed in standup"));
    pg_item::set_last_phase_commit(item, Some("abc123"));
    pg_item::set_structured_description(
        item,
        Some(&StructuredDescription {
            context: "Round-trip context".to_string(),
            problem: "Round-trip problem".to_string(),
            solution: "Round-trip solution".to_string(),
            impact: "Round-trip impact".to_string(),
            sizing_rationale: "Round-trip rationale".to_string(),
        }),
    );

    // Save via Store
    store.save_active(std::slice::from_ref(&pg.0)).unwrap();

    // Load back
    let loaded_items = store.load_active().unwrap();
    assert_eq!(loaded_items.len(), 1);

    let loaded_pg = PgItem(loaded_items.into_iter().next().unwrap());

    // Verify all 15 extension fields survived the round-trip
    assert_eq!(loaded_pg.pg_status(), ItemStatus::Scoping);
    assert_eq!(loaded_pg.phase().as_deref(), Some("build"));
    assert_eq!(loaded_pg.phase_pool(), Some(PhasePool::Main));
    assert_eq!(loaded_pg.size(), Some(SizeLevel::Medium));
    assert_eq!(loaded_pg.complexity(), Some(DimensionLevel::High));
    assert_eq!(loaded_pg.risk(), Some(DimensionLevel::Low));
    assert_eq!(loaded_pg.impact(), Some(DimensionLevel::Medium));
    assert!(loaded_pg.requires_human_review());
    assert_eq!(loaded_pg.pipeline_type().as_deref(), Some("feature"));
    assert_eq!(loaded_pg.origin().as_deref(), Some("WRK-001"));
    assert_eq!(loaded_pg.blocked_type(), Some(BlockType::Decision));
    // Note: blocked_from_status needs native field to also be set for divergence check
    // Just verify the extension value is present in the raw map
    assert!(loaded_pg
        .0
        .extensions
        .contains_key("x-pg-blocked-from-status"));
    assert_eq!(
        loaded_pg.unblock_context().as_deref(),
        Some("discussed in standup")
    );
    assert_eq!(loaded_pg.last_phase_commit().as_deref(), Some("abc123"));

    let desc = loaded_pg
        .structured_description()
        .expect("should have description");
    assert_eq!(desc.context, "Round-trip context");
    assert_eq!(desc.problem, "Round-trip problem");
    assert_eq!(desc.solution, "Round-trip solution");
    assert_eq!(desc.impact, "Round-trip impact");
    assert_eq!(desc.sizing_rationale, "Round-trip rationale");

    // Also verify native description was populated
    assert_eq!(
        loaded_pg.0.description.as_deref(),
        Some("Round-trip context")
    );
}

// =====================================================================
// spawn_blocking + with_lock smoke test
// =====================================================================

#[tokio::test]
async fn spawn_blocking_with_lock_smoke_test() {
    let tmp = tempfile::tempdir().unwrap();
    let store_dir = tmp.path().join(".task-golem");
    std::fs::create_dir_all(&store_dir).unwrap();

    let store = Store::new(store_dir);

    // Save initial empty state
    store.save_active(&[]).unwrap();

    // Use spawn_blocking + with_lock pattern that Phase 3 depends on
    let store_clone = store.clone();
    let result = tokio::task::spawn_blocking(move || {
        store_clone.with_lock(|s| {
            let items = s.load_active()?;
            s.save_active(&items)?;
            Ok(items.len())
        })
    })
    .await;

    let count = result
        .expect("spawn_blocking should not panic")
        .expect("with_lock should succeed");

    assert_eq!(count, 0);
}

#[tokio::test]
async fn spawn_blocking_with_lock_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let store_dir = tmp.path().join(".task-golem");
    std::fs::create_dir_all(&store_dir).unwrap();

    let store = Store::new(store_dir);

    // Create an item and save it
    let pg = pg_item::new_from_parts(
        "WRK-async".to_string(),
        "Async round-trip".to_string(),
        ItemStatus::New,
        vec![],
        vec![],
    );
    store.save_active(std::slice::from_ref(&pg.0)).unwrap();

    // Verify we can read it back inside spawn_blocking + with_lock
    let store_clone = store.clone();
    let result = tokio::task::spawn_blocking(move || {
        store_clone.with_lock(|s| {
            let items = s.load_active()?;
            assert_eq!(items.len(), 1);
            let loaded = PgItem(items.into_iter().next().unwrap());
            assert_eq!(loaded.id(), "WRK-async");
            assert_eq!(loaded.pg_status(), ItemStatus::New);
            Ok(())
        })
    })
    .await;

    result
        .expect("spawn_blocking should not panic")
        .expect("with_lock should succeed");
}

// =====================================================================
// blocked_from_status setter round-trip
// =====================================================================

#[test]
fn blocked_from_status_set_and_read() {
    let mut item = make_test_item();
    item.status = Status::Blocked;
    item.blocked_from_status = Some(Status::Todo); // native field set

    pg_item::set_blocked_from_status(&mut item, Some(&ItemStatus::Ready));
    let pg = PgItem(item);
    assert_eq!(pg.pg_blocked_from_status(), Some(ItemStatus::Ready));
}

#[test]
fn blocked_from_status_all_valid_values() {
    for status in &[
        ItemStatus::New,
        ItemStatus::Scoping,
        ItemStatus::Ready,
        ItemStatus::InProgress,
    ] {
        let mut item = make_test_item();
        item.status = Status::Blocked;
        item.blocked_from_status = Some(Status::Todo);
        pg_item::set_blocked_from_status(&mut item, Some(status));
        let pg = PgItem(item);
        assert_eq!(
            pg.pg_blocked_from_status(),
            Some(status.clone()),
            "Failed for {:?}",
            status
        );
    }
}
