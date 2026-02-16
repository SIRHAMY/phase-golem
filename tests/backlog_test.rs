mod common;

use std::fs;

use orchestrate::backlog;
use orchestrate::types::{
    BacklogFile, BlockType, DimensionLevel, FollowUp, InboxItem, ItemStatus, SizeLevel,
    UpdatedAssessments,
};
use tempfile::TempDir;

// =============================================================================
// Load tests
// =============================================================================

#[test]
fn load_full_backlog_with_all_field_variations() {
    let fp = common::fixture_path("backlog_full.yaml");
    let backlog = backlog::load(&fp, fp.parent().unwrap()).unwrap();
    assert_eq!(backlog.schema_version, 3);
    assert_eq!(backlog.items.len(), 3);

    let item1 = &backlog.items[0];
    assert_eq!(item1.id, "WRK-001");
    assert_eq!(item1.status, ItemStatus::InProgress);
    assert_eq!(item1.phase, Some("build".to_string()));
    assert_eq!(item1.size, Some(SizeLevel::Medium));
    assert_eq!(item1.impact, Some(DimensionLevel::High));
    assert_eq!(item1.tags, vec!["auth", "security"]);
    assert_eq!(item1.dependencies, vec!["WRK-002"]);

    let item3 = &backlog.items[2];
    assert_eq!(item3.status, ItemStatus::Blocked);
    assert_eq!(item3.blocked_from_status, Some(ItemStatus::Scoping));
    assert_eq!(item3.blocked_type, Some(BlockType::Clarification));
    assert!(item3.requires_human_review);
}

#[test]
fn load_minimal_backlog() {
    let fp = common::fixture_path("backlog_minimal.yaml");
    let backlog = backlog::load(&fp, fp.parent().unwrap()).unwrap();
    assert_eq!(backlog.schema_version, 3);
    assert_eq!(backlog.items.len(), 1);
    let item = &backlog.items[0];
    assert_eq!(item.id, "WRK-001");
    assert_eq!(item.status, ItemStatus::New);
    assert_eq!(item.phase, None);
    assert_eq!(item.size, None);
    assert!(item.tags.is_empty());
    assert!(item.dependencies.is_empty());
}

#[test]
fn load_empty_backlog() {
    let fp = common::fixture_path("backlog_empty.yaml");
    let backlog = backlog::load(&fp, fp.parent().unwrap()).unwrap();
    assert_eq!(backlog.schema_version, 3);
    assert!(backlog.items.is_empty());
}

#[test]
fn load_unknown_fields_does_not_error() {
    let fp = common::fixture_path("backlog_unknown_fields.yaml");
    let backlog = backlog::load(&fp, fp.parent().unwrap()).unwrap();
    assert_eq!(backlog.items.len(), 1);
    assert_eq!(backlog.items[0].id, "WRK-001");
}

#[test]
fn load_wrong_schema_version_errors() {
    let fp = common::fixture_path("backlog_wrong_version.yaml");
    let result = backlog::load(&fp, fp.parent().unwrap());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("schema_version"));
    assert!(err.contains("99"));
}

#[test]
fn load_nonexistent_file_errors() {
    let fp = common::fixture_path("does_not_exist.yaml");
    let result = backlog::load(&fp, fp.parent().unwrap());
    assert!(result.is_err());
}

// =============================================================================
// Save tests (atomic write)
// =============================================================================

#[test]
fn save_and_reload_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG.yaml");

    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-001", ItemStatus::New));
    backlog.items[0].size = Some(SizeLevel::Small);
    backlog.items[0].tags = vec!["test".to_string()];

    backlog::save(&path, &backlog).unwrap();
    let reloaded = backlog::load(&path, path.parent().unwrap()).unwrap();

    assert_eq!(reloaded.schema_version, 3);
    assert_eq!(reloaded.items.len(), 1);
    assert_eq!(reloaded.items[0].id, "WRK-001");
    assert_eq!(reloaded.items[0].size, Some(SizeLevel::Small));
    assert_eq!(reloaded.items[0].tags, vec!["test"]);
}

#[test]
fn save_overwrites_existing_file_atomically() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG.yaml");

    // Write initial version
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-001", ItemStatus::New));
    backlog::save(&path, &backlog).unwrap();

    // Overwrite with updated version
    backlog.items[0].status = ItemStatus::Scoping;
    backlog.items.push(common::make_item("WRK-002", ItemStatus::New));
    backlog::save(&path, &backlog).unwrap();

    // Verify new version
    let reloaded = backlog::load(&path, path.parent().unwrap()).unwrap();
    assert_eq!(reloaded.items.len(), 2);
    assert_eq!(reloaded.items[0].status, ItemStatus::Scoping);
}

#[test]
fn save_creates_parent_directory() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nested/dir/BACKLOG.yaml");

    let backlog = common::empty_backlog();
    backlog::save(&path, &backlog).unwrap();

    let reloaded = backlog::load(&path, path.parent().unwrap()).unwrap();
    assert!(reloaded.items.is_empty());
}

#[test]
fn save_yaml_round_trip_no_field_loss() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG.yaml");

    let mut backlog = common::empty_backlog();
    let mut item = common::make_item("WRK-001", ItemStatus::Blocked);
    item.phase = Some("design".to_string());
    item.size = Some(SizeLevel::Large);
    item.complexity = Some(DimensionLevel::High);
    item.risk = Some(DimensionLevel::Medium);
    item.impact = Some(DimensionLevel::Low);
    item.requires_human_review = true;
    item.origin = Some("WRK-000/prd".to_string());
    item.blocked_from_status = Some(ItemStatus::InProgress);
    item.blocked_reason = Some("Need decision".to_string());
    item.blocked_type = Some(BlockType::Decision);
    item.unblock_context = Some("Proceed with option A".to_string());
    item.tags = vec!["urgent".to_string(), "backend".to_string()];
    item.dependencies = vec!["WRK-002".to_string(), "WRK-003".to_string()];
    backlog.items.push(item);

    backlog::save(&path, &backlog).unwrap();
    let reloaded = backlog::load(&path, path.parent().unwrap()).unwrap();

    assert_eq!(backlog, reloaded);
}

// =============================================================================
// ID generation tests
// =============================================================================

#[test]
fn generate_id_empty_backlog() {
    let backlog = common::empty_backlog();
    let (id, _suffix) = backlog::generate_next_id(&backlog, "WRK");
    assert_eq!(id, "WRK-001");
}

#[test]
fn generate_id_sequential() {
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-001", ItemStatus::New));
    backlog.items.push(common::make_item("WRK-002", ItemStatus::New));
    let (id, _suffix) = backlog::generate_next_id(&backlog, "WRK");
    assert_eq!(id, "WRK-003");
}

#[test]
fn generate_id_with_gaps() {
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-001", ItemStatus::New));
    backlog.items.push(common::make_item("WRK-005", ItemStatus::New));
    // Should use max + 1, not fill gaps
    let (id, _suffix) = backlog::generate_next_id(&backlog, "WRK");
    assert_eq!(id, "WRK-006");
}

#[test]
fn generate_id_zero_padding() {
    let backlog = common::empty_backlog();
    let (id, _suffix) = backlog::generate_next_id(&backlog, "WRK");
    assert_eq!(id, "WRK-001"); // 3-digit zero-padded
}

#[test]
fn generate_id_different_prefix_ignored() {
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("OTHER-010", ItemStatus::New));
    // Items with different prefix should be ignored
    let (id, _suffix) = backlog::generate_next_id(&backlog, "WRK");
    assert_eq!(id, "WRK-001");
}

#[test]
fn generate_id_non_numeric_suffix_ignored() {
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-abc", ItemStatus::New));
    backlog.items.push(common::make_item("WRK-003", ItemStatus::New));
    let (id, _suffix) = backlog::generate_next_id(&backlog, "WRK");
    assert_eq!(id, "WRK-004");
}

// =============================================================================
// add_item tests
// =============================================================================

#[test]
fn add_item_creates_new_item() {
    let mut backlog = common::empty_backlog();
    let item = backlog::add_item(&mut backlog, "My new task", None, None, "WRK");

    assert_eq!(item.id, "WRK-001");
    assert_eq!(item.title, "My new task");
    assert_eq!(item.status, ItemStatus::New);
    assert_eq!(item.phase, None);
    assert!(!item.requires_human_review);
    assert_eq!(backlog.items.len(), 1);
}

#[test]
fn add_item_with_size_and_risk() {
    let mut backlog = common::empty_backlog();
    let item = backlog::add_item(
        &mut backlog,
        "Sized task",
        Some(SizeLevel::Large),
        Some(DimensionLevel::High),
        "WRK",
    );

    assert_eq!(item.size, Some(SizeLevel::Large));
    assert_eq!(item.risk, Some(DimensionLevel::High));
}

#[test]
fn add_item_sequential_ids() {
    let mut backlog = common::empty_backlog();
    let item1 = backlog::add_item(&mut backlog, "First", None, None, "WRK");
    let item2 = backlog::add_item(&mut backlog, "Second", None, None, "WRK");

    assert_eq!(item1.id, "WRK-001");
    assert_eq!(item2.id, "WRK-002");
    assert_eq!(backlog.items.len(), 2);
}

// =============================================================================
// Status transition tests
// =============================================================================

#[test]
fn transition_new_to_scoping() {
    let mut item = common::make_item("WRK-001", ItemStatus::New);
    backlog::transition_status(&mut item, ItemStatus::Scoping).unwrap();
    assert_eq!(item.status, ItemStatus::Scoping);
}

#[test]
fn transition_scoping_to_ready() {
    let mut item = common::make_item("WRK-001", ItemStatus::Scoping);
    backlog::transition_status(&mut item, ItemStatus::Ready).unwrap();
    assert_eq!(item.status, ItemStatus::Ready);
}

#[test]
fn transition_ready_to_in_progress() {
    let mut item = common::make_item("WRK-001", ItemStatus::Ready);
    backlog::transition_status(&mut item, ItemStatus::InProgress).unwrap();
    assert_eq!(item.status, ItemStatus::InProgress);
}

#[test]
fn transition_in_progress_to_done() {
    let mut item = common::make_item("WRK-001", ItemStatus::InProgress);
    backlog::transition_status(&mut item, ItemStatus::Done).unwrap();
    assert_eq!(item.status, ItemStatus::Done);
}

#[test]
fn transition_invalid_new_to_done() {
    let mut item = common::make_item("WRK-001", ItemStatus::New);
    let result = backlog::transition_status(&mut item, ItemStatus::Done);
    assert!(result.is_err());
    assert_eq!(item.status, ItemStatus::New); // unchanged
}

#[test]
fn transition_invalid_done_to_anything() {
    let mut item = common::make_item("WRK-001", ItemStatus::Done);
    assert!(backlog::transition_status(&mut item, ItemStatus::New).is_err());
    assert!(backlog::transition_status(&mut item, ItemStatus::InProgress).is_err());
    assert!(backlog::transition_status(&mut item, ItemStatus::Blocked).is_err());
}

#[test]
fn transition_invalid_skip_forward() {
    let mut item = common::make_item("WRK-001", ItemStatus::New);
    assert!(backlog::transition_status(&mut item, ItemStatus::Ready).is_err());
    assert!(backlog::transition_status(&mut item, ItemStatus::InProgress).is_err());
}

// =============================================================================
// Blocked/unblock cycle tests
// =============================================================================

#[test]
fn block_saves_from_status() {
    let mut item = common::make_item("WRK-001", ItemStatus::InProgress);
    backlog::transition_status(&mut item, ItemStatus::Blocked).unwrap();

    assert_eq!(item.status, ItemStatus::Blocked);
    assert_eq!(item.blocked_from_status, Some(ItemStatus::InProgress));
}

#[test]
fn block_from_new() {
    let mut item = common::make_item("WRK-001", ItemStatus::New);
    backlog::transition_status(&mut item, ItemStatus::Blocked).unwrap();
    assert_eq!(item.blocked_from_status, Some(ItemStatus::New));
}

#[test]
fn block_from_scoping() {
    let mut item = common::make_item("WRK-001", ItemStatus::Scoping);
    backlog::transition_status(&mut item, ItemStatus::Blocked).unwrap();
    assert_eq!(item.blocked_from_status, Some(ItemStatus::Scoping));
}

#[test]
fn block_from_ready() {
    let mut item = common::make_item("WRK-001", ItemStatus::Ready);
    backlog::transition_status(&mut item, ItemStatus::Blocked).unwrap();
    assert_eq!(item.blocked_from_status, Some(ItemStatus::Ready));
}

#[test]
fn unblock_clears_blocked_fields() {
    let mut item = common::make_item("WRK-001", ItemStatus::InProgress);
    backlog::transition_status(&mut item, ItemStatus::Blocked).unwrap();

    item.blocked_reason = Some("Needs clarification".to_string());
    item.blocked_type = Some(BlockType::Clarification);
    item.unblock_context = Some("Resolved via discussion".to_string());

    // Unblock back to InProgress
    backlog::transition_status(&mut item, ItemStatus::InProgress).unwrap();

    assert_eq!(item.status, ItemStatus::InProgress);
    assert_eq!(item.blocked_from_status, None);
    assert_eq!(item.blocked_reason, None);
    assert_eq!(item.blocked_type, None);
    assert_eq!(item.unblock_context, None);
}

#[test]
fn unblock_to_different_status() {
    let mut item = common::make_item("WRK-001", ItemStatus::Scoping);
    backlog::transition_status(&mut item, ItemStatus::Blocked).unwrap();

    // Blocked can transition to any non-terminal status
    backlog::transition_status(&mut item, ItemStatus::Ready).unwrap();
    assert_eq!(item.status, ItemStatus::Ready);
    assert_eq!(item.blocked_from_status, None);
}

#[test]
fn cannot_block_already_blocked() {
    let mut item = common::make_item("WRK-001", ItemStatus::New);
    backlog::transition_status(&mut item, ItemStatus::Blocked).unwrap();
    assert!(backlog::transition_status(&mut item, ItemStatus::Blocked).is_err());
}

// =============================================================================
// Assessment update tests
// =============================================================================

#[test]
fn update_assessments_partial() {
    let mut item = common::make_item("WRK-001", ItemStatus::New);
    item.size = Some(SizeLevel::Small);

    let assessments = UpdatedAssessments {
        size: None, // don't override
        complexity: Some(DimensionLevel::Medium),
        risk: None,
        impact: Some(DimensionLevel::High),
    };

    backlog::update_assessments(&mut item, &assessments);

    assert_eq!(item.size, Some(SizeLevel::Small)); // unchanged
    assert_eq!(item.complexity, Some(DimensionLevel::Medium)); // set
    assert_eq!(item.risk, None); // unchanged
    assert_eq!(item.impact, Some(DimensionLevel::High)); // set
}

#[test]
fn update_assessments_full_override() {
    let mut item = common::make_item("WRK-001", ItemStatus::New);
    item.size = Some(SizeLevel::Small);
    item.complexity = Some(DimensionLevel::Low);

    let assessments = UpdatedAssessments {
        size: Some(SizeLevel::Large),
        complexity: Some(DimensionLevel::High),
        risk: Some(DimensionLevel::Medium),
        impact: Some(DimensionLevel::High),
    };

    backlog::update_assessments(&mut item, &assessments);

    assert_eq!(item.size, Some(SizeLevel::Large));
    assert_eq!(item.complexity, Some(DimensionLevel::High));
    assert_eq!(item.risk, Some(DimensionLevel::Medium));
    assert_eq!(item.impact, Some(DimensionLevel::High));
}

// =============================================================================
// Archive tests
// =============================================================================

#[test]
fn archive_item_removes_from_backlog_and_writes_worklog() {
    let dir = TempDir::new().unwrap();
    let backlog_path = dir.path().join("BACKLOG.yaml");
    let worklog_path = dir.path().join("_worklog/2026-02.md");

    let mut backlog = common::empty_backlog();
    let mut item = common::make_item("WRK-001", ItemStatus::Done);
    item.phase = Some("review".to_string());
    backlog.items.push(item);
    backlog.items.push(common::make_item("WRK-002", ItemStatus::New));
    backlog::save(&backlog_path, &backlog).unwrap();

    backlog::archive_item(&mut backlog, "WRK-001", &backlog_path, &worklog_path).unwrap();

    // Backlog should only have WRK-002
    let reloaded = backlog::load(&backlog_path, backlog_path.parent().unwrap()).unwrap();
    assert_eq!(reloaded.items.len(), 1);
    assert_eq!(reloaded.items[0].id, "WRK-002");

    // Worklog should exist
    let worklog_contents = fs::read_to_string(&worklog_path).unwrap();
    assert!(worklog_contents.contains("WRK-001"));
    assert!(worklog_contents.contains("Done"));
}

#[test]
fn archive_item_strips_dependencies_from_remaining_items() {
    let dir = TempDir::new().unwrap();
    let backlog_path = dir.path().join("BACKLOG.yaml");
    let worklog_path = dir.path().join("_worklog/2026-02.md");

    let mut backlog = common::empty_backlog();
    let mut done_item = common::make_item("WRK-001", ItemStatus::Done);
    done_item.phase = Some("review".to_string());
    backlog.items.push(done_item);

    let mut dependent = common::make_item("WRK-002", ItemStatus::Ready);
    dependent.dependencies = vec!["WRK-001".to_string(), "WRK-003".to_string()];
    backlog.items.push(dependent);

    let mut also_dependent = common::make_item("WRK-003", ItemStatus::New);
    also_dependent.dependencies = vec!["WRK-001".to_string()];
    backlog.items.push(also_dependent);

    backlog::save(&backlog_path, &backlog).unwrap();

    backlog::archive_item(&mut backlog, "WRK-001", &backlog_path, &worklog_path).unwrap();

    // WRK-002 should only have WRK-003 as a dependency
    assert_eq!(backlog.items[0].id, "WRK-002");
    assert_eq!(backlog.items[0].dependencies, vec!["WRK-003"]);

    // WRK-003 should have no dependencies
    assert_eq!(backlog.items[1].id, "WRK-003");
    assert!(backlog.items[1].dependencies.is_empty());

    // Verify persisted to disk
    let reloaded = backlog::load(&backlog_path, backlog_path.parent().unwrap()).unwrap();
    assert_eq!(reloaded.items[0].dependencies, vec!["WRK-003"]);
    assert!(reloaded.items[1].dependencies.is_empty());
}

#[test]
fn archive_worklog_entry_appends_chronologically() {
    let dir = TempDir::new().unwrap();
    let backlog_path = dir.path().join("BACKLOG.yaml");
    let worklog_path = dir.path().join("_worklog/2026-02.md");

    let mut backlog = common::empty_backlog();
    let mut item1 = common::make_item("WRK-001", ItemStatus::Done);
    item1.phase = Some("review".to_string());
    backlog.items.push(item1);
    let mut item2 = common::make_item("WRK-002", ItemStatus::Done);
    item2.phase = Some("review".to_string());
    backlog.items.push(item2);
    backlog::save(&backlog_path, &backlog).unwrap();

    // Archive first item
    backlog::archive_item(&mut backlog, "WRK-001", &backlog_path, &worklog_path).unwrap();
    // Archive second item
    backlog::archive_item(&mut backlog, "WRK-002", &backlog_path, &worklog_path).unwrap();

    let worklog_contents = fs::read_to_string(&worklog_path).unwrap();
    let pos_first = worklog_contents
        .find("WRK-001")
        .expect("Expected WRK-001 in worklog");
    let pos_second = worklog_contents
        .find("WRK-002")
        .expect("Expected WRK-002 in worklog");
    assert!(
        pos_first < pos_second,
        "Expected WRK-001 (first archived) to appear before WRK-002 (second archived)"
    );
}

#[test]
fn archive_nonexistent_item_fails() {
    let dir = TempDir::new().unwrap();
    let backlog_path = dir.path().join("BACKLOG.yaml");
    let worklog_path = dir.path().join("_worklog/2026-02.md");

    let mut backlog = common::empty_backlog();
    backlog::save(&backlog_path, &backlog).unwrap();

    let result =
        backlog::archive_item(&mut backlog, "WRK-999", &backlog_path, &worklog_path);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

// =============================================================================
// Follow-up ingestion tests
// =============================================================================

#[test]
fn ingest_follow_ups_creates_new_items() {
    let mut backlog = common::empty_backlog();
    let follow_ups = vec![
        FollowUp {
            title: "Follow-up 1".to_string(),
            context: Some("From PRD".to_string()),
            suggested_size: Some(SizeLevel::Small),
            suggested_risk: Some(DimensionLevel::Low),
        },
        FollowUp {
            title: "Follow-up 2".to_string(),
            context: None,
            suggested_size: None,
            suggested_risk: None,
        },
    ];

    let created =
        backlog::ingest_follow_ups(&mut backlog, &follow_ups, "WRK-001/prd", "WRK");

    assert_eq!(created.len(), 2);
    assert_eq!(created[0].id, "WRK-001");
    assert_eq!(created[0].title, "Follow-up 1");
    assert_eq!(created[0].status, ItemStatus::New);
    assert_eq!(created[0].origin, Some("WRK-001/prd".to_string()));
    assert_eq!(created[0].size, Some(SizeLevel::Small));
    assert_eq!(created[0].risk, Some(DimensionLevel::Low));

    assert_eq!(created[1].id, "WRK-002");
    assert_eq!(created[1].title, "Follow-up 2");
    assert_eq!(created[1].origin, Some("WRK-001/prd".to_string()));
    assert_eq!(created[1].size, None);

    assert_eq!(backlog.items.len(), 2);
}

#[test]
fn ingest_follow_ups_continues_ids_from_existing_items() {
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-005", ItemStatus::New));

    let follow_ups = vec![FollowUp {
        title: "New follow-up".to_string(),
        context: None,
        suggested_size: None,
        suggested_risk: None,
    }];

    let created = backlog::ingest_follow_ups(&mut backlog, &follow_ups, "WRK-005/build", "WRK");

    assert_eq!(created[0].id, "WRK-006");
    assert_eq!(backlog.items.len(), 2);
}

#[test]
fn ingest_empty_follow_ups() {
    let mut backlog = common::empty_backlog();
    let created = backlog::ingest_follow_ups(&mut backlog, &[], "WRK-001/prd", "WRK");
    assert!(created.is_empty());
    assert!(backlog.items.is_empty());
}

// =============================================================================
// Integration / round-trip tests
// =============================================================================

#[test]
fn full_lifecycle_add_transition_advance_archive() {
    let dir = TempDir::new().unwrap();
    let backlog_path = dir.path().join("BACKLOG.yaml");
    let worklog_path = dir.path().join("_worklog/2026-02.md");

    let mut backlog = common::empty_backlog();

    // Add
    let item = backlog::add_item(&mut backlog, "Full lifecycle test", None, None, "WRK");
    assert_eq!(item.id, "WRK-001");

    // Transition through statuses
    let item = &mut backlog.items[0];
    backlog::transition_status(item, ItemStatus::Scoping).unwrap();
    backlog::transition_status(item, ItemStatus::Ready).unwrap();
    backlog::transition_status(item, ItemStatus::InProgress).unwrap();

    // Set phase manually (phases are now free-form strings)
    item.phase = Some("review".to_string());

    // Complete
    backlog::transition_status(item, ItemStatus::Done).unwrap();

    // Save and archive
    backlog::save(&backlog_path, &backlog).unwrap();
    backlog::archive_item(&mut backlog, "WRK-001", &backlog_path, &worklog_path).unwrap();

    let reloaded = backlog::load(&backlog_path, backlog_path.parent().unwrap()).unwrap();
    assert!(reloaded.items.is_empty());

    let worklog = fs::read_to_string(&worklog_path).unwrap();
    assert!(worklog.contains("WRK-001"));
}

#[test]
fn block_unblock_cycle_preserves_state() {
    let mut item = common::make_in_progress_item("WRK-001", "build");
    let original_phase = item.phase.clone();

    // Block
    backlog::transition_status(&mut item, ItemStatus::Blocked).unwrap();
    assert_eq!(item.blocked_from_status, Some(ItemStatus::InProgress));
    assert_eq!(item.phase, original_phase); // phase preserved

    // Add blocked metadata
    item.blocked_reason = Some("Need review".to_string());
    item.blocked_type = Some(BlockType::Decision);

    // Unblock
    backlog::transition_status(&mut item, ItemStatus::InProgress).unwrap();
    assert_eq!(item.status, ItemStatus::InProgress);
    assert_eq!(item.phase, original_phase); // phase still preserved
    assert_eq!(item.blocked_from_status, None);
    assert_eq!(item.blocked_reason, None);
    assert_eq!(item.blocked_type, None);
}

#[test]
fn transition_updates_timestamp() {
    let mut item = common::make_item("WRK-001", ItemStatus::New);
    let original_updated = item.updated.clone();

    // Small delay to ensure timestamp differs
    backlog::transition_status(&mut item, ItemStatus::Scoping).unwrap();
    assert_ne!(item.updated, original_updated);
}

// =============================================================================
// High-water mark (next_item_id) tests
// =============================================================================

#[test]
fn generate_id_empty_backlog_with_high_water_mark() {
    let mut backlog = common::empty_backlog();
    backlog.next_item_id = 5;
    let (id, suffix) = backlog::generate_next_id(&backlog, "WRK");
    assert_eq!(id, "WRK-006");
    assert_eq!(suffix, 6);
}

#[test]
fn generate_id_current_items_exceed_high_water_mark() {
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-010", ItemStatus::New));
    backlog.next_item_id = 3;
    let (id, suffix) = backlog::generate_next_id(&backlog, "WRK");
    assert_eq!(id, "WRK-011");
    assert_eq!(suffix, 11);
}

#[test]
fn generate_id_high_water_mark_exceeds_current_items() {
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-003", ItemStatus::New));
    backlog.next_item_id = 10;
    let (id, suffix) = backlog::generate_next_id(&backlog, "WRK");
    assert_eq!(id, "WRK-011");
    assert_eq!(suffix, 11);
}

#[test]
fn generate_id_sequential_with_high_water_mark_updates() {
    let mut backlog = common::empty_backlog();

    // First generation
    let (id1, suffix1) = backlog::generate_next_id(&backlog, "WRK");
    backlog.next_item_id = suffix1;
    backlog.items.push(common::make_item(&id1, ItemStatus::New));
    assert_eq!(id1, "WRK-001");
    assert_eq!(suffix1, 1);

    // Second generation
    let (id2, suffix2) = backlog::generate_next_id(&backlog, "WRK");
    backlog.next_item_id = suffix2;
    backlog.items.push(common::make_item(&id2, ItemStatus::New));
    assert_eq!(id2, "WRK-002");
    assert_eq!(suffix2, 2);

    // Third generation
    let (id3, suffix3) = backlog::generate_next_id(&backlog, "WRK");
    backlog.next_item_id = suffix3;
    backlog.items.push(common::make_item(&id3, ItemStatus::New));
    assert_eq!(id3, "WRK-003");
    assert_eq!(suffix3, 3);

    assert_eq!(backlog.next_item_id, 3);
}

#[test]
fn add_item_updates_next_item_id() {
    let mut backlog = common::empty_backlog();

    backlog::add_item(&mut backlog, "First", None, None, "WRK");
    assert_eq!(backlog.next_item_id, 1);

    backlog::add_item(&mut backlog, "Second", None, None, "WRK");
    assert_eq!(backlog.next_item_id, 2);
}

#[test]
fn ingest_follow_ups_updates_next_item_id_sequentially() {
    let mut backlog = common::empty_backlog();
    let follow_ups = vec![
        FollowUp {
            title: "FU 1".to_string(),
            context: None,
            suggested_size: None,
            suggested_risk: None,
        },
        FollowUp {
            title: "FU 2".to_string(),
            context: None,
            suggested_size: None,
            suggested_risk: None,
        },
        FollowUp {
            title: "FU 3".to_string(),
            context: None,
            suggested_size: None,
            suggested_risk: None,
        },
    ];

    let created = backlog::ingest_follow_ups(&mut backlog, &follow_ups, "WRK-000/prd", "WRK");

    assert_eq!(created[0].id, "WRK-001");
    assert_eq!(created[1].id, "WRK-002");
    assert_eq!(created[2].id, "WRK-003");
    assert_eq!(backlog.next_item_id, 3);
}

#[test]
fn no_id_reuse_after_archival() {
    let mut backlog = common::empty_backlog();

    // Add items WRK-001 through WRK-003
    backlog::add_item(&mut backlog, "Item 1", None, None, "WRK");
    backlog::add_item(&mut backlog, "Item 2", None, None, "WRK");
    backlog::add_item(&mut backlog, "Item 3", None, None, "WRK");
    assert_eq!(backlog.next_item_id, 3);

    // Simulate archival: clear all items but keep next_item_id
    backlog.items.clear();

    // Generate next ID — should be WRK-004, not WRK-001
    let (id, suffix) = backlog::generate_next_id(&backlog, "WRK");
    assert_eq!(id, "WRK-004");
    assert_eq!(suffix, 4);
}

#[test]
fn backward_compatible_yaml_load_without_next_item_id() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG.yaml");

    // Write YAML without next_item_id field
    fs::write(&path, "schema_version: 3\nitems: []\n").unwrap();

    // Load — next_item_id should default to 0
    let mut backlog = backlog::load(&path, path.parent().unwrap()).unwrap();
    assert_eq!(backlog.next_item_id, 0);

    // Generate an ID
    let (id, suffix) = backlog::generate_next_id(&backlog, "WRK");
    backlog.next_item_id = suffix;
    backlog.items.push(common::make_item(&id, ItemStatus::New));

    // Save
    backlog::save(&path, &backlog).unwrap();

    // Reload and verify next_item_id is persisted
    let reloaded = backlog::load(&path, path.parent().unwrap()).unwrap();
    assert_eq!(reloaded.next_item_id, suffix);

    // Verify raw YAML contains the field
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.contains("next_item_id"));
}

// =============================================================================
// load_inbox tests
// =============================================================================

#[test]
fn load_inbox_file_does_not_exist_returns_none() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG_INBOX.yaml");

    let result = backlog::load_inbox(&path).unwrap();
    assert!(result.is_none());
}

#[test]
fn load_inbox_valid_yaml_returns_items() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG_INBOX.yaml");

    fs::write(
        &path,
        "- title: Add feature X\n  description: Some details\n  size: small\n  risk: low\n- title: Fix bug Y\n",
    )
    .unwrap();

    let result = backlog::load_inbox(&path).unwrap().unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].title, "Add feature X");
    assert_eq!(result[0].description, Some("Some details".to_string()));
    assert_eq!(result[0].size, Some(SizeLevel::Small));
    assert_eq!(result[0].risk, Some(DimensionLevel::Low));
    assert_eq!(result[1].title, "Fix bug Y");
    assert_eq!(result[1].description, None);
    assert_eq!(result[1].size, None);
}

#[test]
fn load_inbox_empty_file_returns_empty_vec() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG_INBOX.yaml");

    fs::write(&path, "").unwrap();

    let result = backlog::load_inbox(&path).unwrap().unwrap();
    assert!(result.is_empty());
}

#[test]
fn load_inbox_whitespace_only_returns_empty_vec() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG_INBOX.yaml");

    fs::write(&path, "  \n\n  \n").unwrap();

    let result = backlog::load_inbox(&path).unwrap().unwrap();
    assert!(result.is_empty());
}

#[test]
fn load_inbox_malformed_yaml_returns_err() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG_INBOX.yaml");

    fs::write(&path, "{{not valid yaml").unwrap();

    let result = backlog::load_inbox(&path);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("Failed to parse inbox YAML"));
}

#[test]
fn load_inbox_all_optional_fields_populated() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG_INBOX.yaml");

    fs::write(
        &path,
        "- title: Full item\n  description: Detailed description\n  size: large\n  risk: high\n  impact: medium\n  pipeline_type: feature\n  dependencies:\n    - WRK-001\n    - WRK-002\n",
    )
    .unwrap();

    let result = backlog::load_inbox(&path).unwrap().unwrap();
    assert_eq!(result.len(), 1);
    let item = &result[0];
    assert_eq!(item.title, "Full item");
    assert_eq!(item.description, Some("Detailed description".to_string()));
    assert_eq!(item.size, Some(SizeLevel::Large));
    assert_eq!(item.risk, Some(DimensionLevel::High));
    assert_eq!(item.impact, Some(DimensionLevel::Medium));
    assert_eq!(item.pipeline_type, Some("feature".to_string()));
    assert_eq!(item.dependencies, vec!["WRK-001", "WRK-002"]);
}

#[test]
fn load_inbox_unknown_fields_silently_ignored() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG_INBOX.yaml");

    fs::write(
        &path,
        "- title: Item with extras\n  id: WRK-999\n  unknown_field: whatever\n  status: done\n",
    )
    .unwrap();

    let result = backlog::load_inbox(&path).unwrap().unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].title, "Item with extras");
}

#[test]
fn load_inbox_invalid_enum_value_returns_err() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG_INBOX.yaml");

    fs::write(&path, "- title: Bad size\n  size: mega\n").unwrap();

    let result = backlog::load_inbox(&path);
    assert!(result.is_err());
}

#[test]
fn load_inbox_yaml_mapping_instead_of_list_returns_err() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG_INBOX.yaml");

    // A common user mistake: writing a mapping instead of a list item
    fs::write(&path, "title: foo\ndescription: bar\n").unwrap();

    let result = backlog::load_inbox(&path);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("Failed to parse inbox YAML"));
}

#[test]
fn load_inbox_wrapped_items_key_returns_err() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG_INBOX.yaml");

    // Using an `items:` wrapper key is not supported — must be a bare sequence
    fs::write(
        &path,
        "items:\n  - title: Wrapped feature\n    size: small\n  - title: Another one\n",
    )
    .unwrap();

    let result = backlog::load_inbox(&path);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("Expected a bare YAML sequence"));
}

// =============================================================================
// ingest_inbox_items tests
// =============================================================================

#[test]
fn ingest_inbox_items_creates_backlog_items_with_correct_fields() {
    let mut backlog = common::empty_backlog();

    let inbox_items = vec![InboxItem {
        title: "New inbox feature".to_string(),
        description: Some("Details here".to_string()),
        size: Some(SizeLevel::Medium),
        risk: Some(DimensionLevel::Low),
        impact: Some(DimensionLevel::High),
        pipeline_type: Some("feature".to_string()),
        dependencies: vec!["WRK-001".to_string()],
    }];

    let created = backlog::ingest_inbox_items(&mut backlog, &inbox_items, "WRK");

    assert_eq!(created.len(), 1);
    let item = &created[0];
    assert_eq!(item.id, "WRK-001");
    assert_eq!(item.title, "New inbox feature");
    assert_eq!(item.status, ItemStatus::New);
    assert_eq!(item.origin, Some("inbox".to_string()));
    assert_eq!(item.description, None);
    assert_eq!(item.size, Some(SizeLevel::Medium));
    assert_eq!(item.risk, Some(DimensionLevel::Low));
    assert_eq!(item.impact, Some(DimensionLevel::High));
    assert_eq!(item.pipeline_type, Some("feature".to_string()));
    assert_eq!(item.dependencies, vec!["WRK-001"]);
    assert!(!item.created.is_empty());
    assert!(!item.updated.is_empty());

    assert_eq!(backlog.items.len(), 1);
}

#[test]
fn ingest_inbox_items_skips_empty_titles_ingests_valid() {
    let mut backlog = common::empty_backlog();

    let inbox_items = vec![
        InboxItem {
            title: "  ".to_string(),
            description: None,
            size: None,
            risk: None,
            impact: None,
            pipeline_type: None,
            dependencies: vec![],
        },
        InboxItem {
            title: "Valid item".to_string(),
            description: None,
            size: None,
            risk: None,
            impact: None,
            pipeline_type: None,
            dependencies: vec![],
        },
        InboxItem {
            title: "".to_string(),
            description: None,
            size: None,
            risk: None,
            impact: None,
            pipeline_type: None,
            dependencies: vec![],
        },
    ];

    let created = backlog::ingest_inbox_items(&mut backlog, &inbox_items, "WRK");

    // Only the valid item should be created
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].title, "Valid item");
    assert_eq!(created[0].id, "WRK-001");
    assert_eq!(backlog.items.len(), 1);
    // next_item_id should only be incremented once (for the valid item)
    assert_eq!(backlog.next_item_id, 1);
}

#[test]
fn ingest_inbox_items_sequential_ids() {
    let mut backlog = common::empty_backlog();

    let inbox_items = vec![
        InboxItem {
            title: "First".to_string(),
            description: None,
            size: None,
            risk: None,
            impact: None,
            pipeline_type: None,
            dependencies: vec![],
        },
        InboxItem {
            title: "Second".to_string(),
            description: None,
            size: None,
            risk: None,
            impact: None,
            pipeline_type: None,
            dependencies: vec![],
        },
        InboxItem {
            title: "Third".to_string(),
            description: None,
            size: None,
            risk: None,
            impact: None,
            pipeline_type: None,
            dependencies: vec![],
        },
    ];

    let created = backlog::ingest_inbox_items(&mut backlog, &inbox_items, "WRK");

    assert_eq!(created[0].id, "WRK-001");
    assert_eq!(created[1].id, "WRK-002");
    assert_eq!(created[2].id, "WRK-003");
    assert_eq!(backlog.next_item_id, 3);
}

#[test]
fn ingest_inbox_items_empty_slice_returns_empty_vec() {
    let mut backlog = common::empty_backlog();
    let original_items_len = backlog.items.len();
    let original_next_id = backlog.next_item_id;

    let created = backlog::ingest_inbox_items(&mut backlog, &[], "WRK");

    assert!(created.is_empty());
    assert_eq!(backlog.items.len(), original_items_len);
    assert_eq!(backlog.next_item_id, original_next_id);
}

// =============================================================================
// clear_inbox tests
// =============================================================================

#[test]
fn clear_inbox_deletes_existing_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG_INBOX.yaml");

    fs::write(&path, "- title: To delete\n").unwrap();
    assert!(path.exists());

    backlog::clear_inbox(&path).unwrap();
    assert!(!path.exists());
}

#[test]
fn clear_inbox_nonexistent_file_returns_ok() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("BACKLOG_INBOX.yaml");

    assert!(!path.exists());
    let result = backlog::clear_inbox(&path);
    assert!(result.is_ok());
}

// =============================================================================
// prune_stale_dependencies tests
// =============================================================================

#[test]
fn prune_stale_dependencies_removes_dangling_refs() {
    let mut backlog = common::empty_backlog();

    let mut item = common::make_item("WRK-001", ItemStatus::Ready);
    item.dependencies = vec!["WRK-999".to_string(), "WRK-002".to_string()];
    backlog.items.push(item);

    backlog.items.push(common::make_item("WRK-002", ItemStatus::New));

    let pruned = backlog::prune_stale_dependencies(&mut backlog);

    assert_eq!(pruned, 1);
    assert_eq!(backlog.items[0].dependencies, vec!["WRK-002"]);
}

#[test]
fn prune_stale_dependencies_no_stale_returns_zero() {
    let mut backlog = common::empty_backlog();

    let mut item = common::make_item("WRK-001", ItemStatus::Ready);
    item.dependencies = vec!["WRK-002".to_string()];
    backlog.items.push(item);

    backlog.items.push(common::make_item("WRK-002", ItemStatus::New));

    let pruned = backlog::prune_stale_dependencies(&mut backlog);

    assert_eq!(pruned, 0);
    assert_eq!(backlog.items[0].dependencies, vec!["WRK-002"]);
}

#[test]
fn prune_stale_dependencies_empty_backlog_returns_zero() {
    let mut backlog = common::empty_backlog();
    let pruned = backlog::prune_stale_dependencies(&mut backlog);
    assert_eq!(pruned, 0);
}

#[test]
fn prune_stale_dependencies_multiple_items_multiple_stale() {
    let mut backlog = common::empty_backlog();

    let mut item_a = common::make_item("WRK-001", ItemStatus::Ready);
    item_a.dependencies = vec!["WRK-888".to_string()];
    backlog.items.push(item_a);

    let mut item_b = common::make_item("WRK-002", ItemStatus::Ready);
    item_b.dependencies = vec!["WRK-999".to_string(), "WRK-001".to_string()];
    backlog.items.push(item_b);

    let pruned = backlog::prune_stale_dependencies(&mut backlog);

    assert_eq!(pruned, 2);
    assert!(backlog.items[0].dependencies.is_empty());
    assert_eq!(backlog.items[1].dependencies, vec!["WRK-001"]);
}

// =============================================================================
// merge_item tests
// =============================================================================

#[test]
fn merge_item_basic() {
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-001", ItemStatus::New));
    backlog.items.push(common::make_item("WRK-002", ItemStatus::New));

    let result = backlog::merge_item(&mut backlog, "WRK-002", "WRK-001").unwrap();

    assert_eq!(result.source_id, "WRK-002");
    assert_eq!(result.target_id, "WRK-001");
    assert_eq!(backlog.items.len(), 1);
    assert_eq!(backlog.items[0].id, "WRK-001");

    // Target should have merge context appended
    let desc = backlog.items[0].description.as_ref().unwrap();
    assert!(desc.context.contains("[Merged from WRK-002]"));
    assert!(desc.context.contains("Test item WRK-002"));
}

#[test]
fn merge_item_dependency_union() {
    let mut backlog = common::empty_backlog();
    let mut target = common::make_item("WRK-001", ItemStatus::New);
    target.dependencies = vec!["WRK-010".to_string()];
    backlog.items.push(target);

    let mut source = common::make_item("WRK-002", ItemStatus::New);
    source.dependencies = vec!["WRK-010".to_string(), "WRK-020".to_string()];
    backlog.items.push(source);

    backlog::merge_item(&mut backlog, "WRK-002", "WRK-001").unwrap();

    // WRK-010 should not be duplicated, WRK-020 should be added
    let deps = &backlog.items[0].dependencies;
    assert_eq!(deps.len(), 2);
    assert!(deps.contains(&"WRK-010".to_string()));
    assert!(deps.contains(&"WRK-020".to_string()));
}

#[test]
fn merge_item_strips_source_from_dependency_lists() {
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-001", ItemStatus::New));
    backlog.items.push(common::make_item("WRK-002", ItemStatus::New));

    let mut dependent = common::make_item("WRK-003", ItemStatus::New);
    dependent.dependencies = vec!["WRK-002".to_string(), "WRK-001".to_string()];
    backlog.items.push(dependent);

    backlog::merge_item(&mut backlog, "WRK-002", "WRK-001").unwrap();

    // WRK-003 should no longer depend on WRK-002
    let item3 = backlog.items.iter().find(|i| i.id == "WRK-003").unwrap();
    assert_eq!(item3.dependencies, vec!["WRK-001"]);
}

#[test]
fn merge_item_self_merge_errors() {
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-001", ItemStatus::New));

    let result = backlog::merge_item(&mut backlog, "WRK-001", "WRK-001");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("into itself"));
    assert_eq!(backlog.items.len(), 1); // unchanged
}

#[test]
fn merge_item_source_not_found_errors() {
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-001", ItemStatus::New));

    let result = backlog::merge_item(&mut backlog, "WRK-999", "WRK-001");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Source item WRK-999 not found"));
}

#[test]
fn merge_item_target_not_found_errors() {
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-001", ItemStatus::New));

    let result = backlog::merge_item(&mut backlog, "WRK-001", "WRK-999");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Target item WRK-999 not found"));
}

#[test]
fn merge_item_appends_to_existing_description() {
    use orchestrate::types::StructuredDescription;

    let mut backlog = common::empty_backlog();
    let mut target = common::make_item("WRK-001", ItemStatus::New);
    target.description = Some(StructuredDescription {
        context: "Existing context".to_string(),
        problem: String::new(),
        solution: String::new(),
        impact: String::new(),
        sizing_rationale: String::new(),
    });
    backlog.items.push(target);
    backlog.items.push(common::make_item("WRK-002", ItemStatus::New));

    backlog::merge_item(&mut backlog, "WRK-002", "WRK-001").unwrap();

    let desc = backlog.items[0].description.as_ref().unwrap();
    assert!(desc.context.starts_with("Existing context\n"));
    assert!(desc.context.contains("[Merged from WRK-002]"));
}

#[test]
fn merge_item_no_self_ref_in_dependencies() {
    let mut backlog = common::empty_backlog();
    backlog.items.push(common::make_item("WRK-001", ItemStatus::New));

    let mut source = common::make_item("WRK-002", ItemStatus::New);
    // Source depends on target — this should NOT be added to target's deps
    source.dependencies = vec!["WRK-001".to_string()];
    backlog.items.push(source);

    backlog::merge_item(&mut backlog, "WRK-002", "WRK-001").unwrap();

    assert!(backlog.items[0].dependencies.is_empty());
}

// =============================================================================
// Pre-implementation verification: from_value equivalence (WRK-002)
// =============================================================================

#[test]
fn test_from_value_matches_from_str_for_backlog_file() {
    let yaml = r#"
schema_version: 3
future_field: true
next_item_id: 5
items:
  - id: WRK-001
    title: Full item with all fields
    status: in_progress
    phase: build
    size: medium
    complexity: high
    risk: low
    impact: high
    requires_human_review: true
    origin: "WRK-000/prd"
    blocked_from_status: null
    blocked_reason: null
    blocked_type: null
    tags:
      - auth
      - security
    dependencies:
      - WRK-002
    created: "2026-02-10T00:00:00+00:00"
    updated: "2026-02-10T00:00:00+00:00"
    pipeline_type: feature
    follow_ups:
      - title: Refactor auth module
        context: Identified during code review
        suggested_size: small
        suggested_risk: low
    updated_assessments:
      size: large
      complexity: medium

  - id: WRK-002
    title: Minimal item with only required fields
    status: new
    created: "2026-02-10T00:00:00+00:00"
    updated: "2026-02-10T00:00:00+00:00"

  - id: WRK-003
    title: Mixed item with some optional fields
    status: blocked
    blocked_from_status: scoping
    blocked_reason: Waiting for decision
    blocked_type: clarification
    size: large
    tags:
      - backend
    follow_ups:
      - "String-only follow-up title"
    created: "2026-02-10T00:00:00+00:00"
    updated: "2026-02-10T00:00:00+00:00"
"#;

    let direct: BacklogFile = serde_yaml_ng::from_str(yaml).unwrap();
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml).unwrap();
    let via_value: BacklogFile = serde_yaml_ng::from_value(value).unwrap();

    assert_eq!(direct, via_value);
}

#[test]
fn test_from_value_error_preserves_field_context() {
    let yaml = r#"
schema_version: 3
items:
  - id: WRK-001
    title: Bad status item
    status: invalid_status
    created: "2026-02-10T00:00:00+00:00"
    updated: "2026-02-10T00:00:00+00:00"
"#;

    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml).unwrap();
    let result = serde_yaml_ng::from_value::<BacklogFile>(value);

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("unknown variant"),
        "Expected error to contain 'unknown variant', got: {}",
        err_msg
    );
}
