mod common;

use std::fs;

use tempfile::TempDir;

use phase_golem::config::default_feature_pipeline;
use phase_golem::migration::{migrate_v1_to_v2, migrate_v2_to_v3, parse_description};
use phase_golem::types::{ItemStatus, PhasePool, StructuredDescription};

// --- Full v1 fixture migration ---

#[test]
fn migrate_v1_full_fixture() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    // Copy v1 fixture to temp dir so migration can write back
    let v1_fixture = common::fixtures_dir().join("backlog_v1_full.yaml");
    fs::copy(&v1_fixture, &target).unwrap();

    let result = migrate_v1_to_v2(&target, &default_feature_pipeline());
    assert!(result.is_ok(), "Migration failed: {:?}", result);

    let backlog = result.unwrap();
    assert_eq!(backlog.schema_version, 2);
    assert_eq!(backlog.items.len(), 5);

    // WRK-001: in_progress + build -> InProgress + "build"
    let item1 = backlog.items.iter().find(|i| i.id == "WRK-001").unwrap();
    assert_eq!(item1.status, ItemStatus::InProgress);
    assert_eq!(item1.phase.as_deref(), Some("build"));
    assert_eq!(item1.phase_pool, Some(PhasePool::Main));
    assert_eq!(item1.pipeline_type.as_deref(), Some("feature"));

    // WRK-002: done + review -> Done + "review"
    let item2 = backlog.items.iter().find(|i| i.id == "WRK-002").unwrap();
    assert_eq!(item2.status, ItemStatus::Done);
    assert_eq!(item2.phase.as_deref(), Some("review"));
    assert_eq!(item2.pipeline_type.as_deref(), Some("feature"));

    // WRK-003: blocked (from researching) -> Blocked (blocked_from_status: Scoping)
    let item3 = backlog.items.iter().find(|i| i.id == "WRK-003").unwrap();
    assert_eq!(item3.status, ItemStatus::Blocked);
    assert_eq!(item3.blocked_from_status, Some(ItemStatus::Scoping));
    assert!(item3.blocked_reason.is_some());

    // WRK-004: researching + research -> Scoping + phase cleared (scheduler re-assigns)
    let item4 = backlog.items.iter().find(|i| i.id == "WRK-004").unwrap();
    assert_eq!(item4.status, ItemStatus::Scoping);
    assert_eq!(
        item4.phase, None,
        "Researching items should have phase cleared"
    );
    assert_eq!(item4.phase_pool, None);
    assert_eq!(item4.pipeline_type.as_deref(), Some("feature"));

    // WRK-005: scoped -> Ready
    let item5 = backlog.items.iter().find(|i| i.id == "WRK-005").unwrap();
    assert_eq!(item5.status, ItemStatus::Ready);
    assert_eq!(item5.pipeline_type.as_deref(), Some("feature"));
}

// --- Persisted file is valid v2 ---

#[test]
fn migrate_v1_persisted_file_is_valid_v2() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    let v1_fixture = common::fixtures_dir().join("backlog_v1_full.yaml");
    fs::copy(&v1_fixture, &target).unwrap();

    migrate_v1_to_v2(&target, &default_feature_pipeline()).unwrap();

    // Re-read the persisted file and verify it's valid v2
    let contents = fs::read_to_string(&target).unwrap();
    assert!(
        contents.contains("schema_version: 2"),
        "Persisted file should have schema_version 2"
    );

    // Parse again — should succeed as v2 without re-migration
    let result = migrate_v1_to_v2(&target, &default_feature_pipeline());
    assert!(
        result.is_ok(),
        "Re-parsing v2 file should succeed: {:?}",
        result
    );
    assert_eq!(result.unwrap().schema_version, 2);
}

// --- Empty backlog migration ---

#[test]
fn migrate_v1_empty_backlog() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    fs::write(&target, "schema_version: 1\nitems: []\n").unwrap();

    let result = migrate_v1_to_v2(&target, &default_feature_pipeline());
    assert!(
        result.is_ok(),
        "Empty backlog migration failed: {:?}",
        result
    );

    let backlog = result.unwrap();
    assert_eq!(backlog.schema_version, 2);
    assert!(backlog.items.is_empty());
}

// --- Idempotency: v2 input is a no-op ---

#[test]
fn migrate_v2_input_is_noop() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    let v2_fixture = common::fixtures_dir().join("backlog_v2_full.yaml");
    let original_contents = fs::read_to_string(&v2_fixture).unwrap();
    fs::write(&target, &original_contents).unwrap();

    let result = migrate_v1_to_v2(&target, &default_feature_pipeline());
    assert!(result.is_ok(), "V2 input should pass through: {:?}", result);

    let backlog = result.unwrap();
    assert_eq!(backlog.schema_version, 2);
}

// --- Blocked item with blocked_from_status mapping ---

#[test]
fn migrate_v1_blocked_item_maps_blocked_from_status() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    fs::write(
        &target,
        r#"schema_version: 1
items:
  - id: WRK-001
    title: Blocked from scoped
    status: blocked
    blocked_from_status: scoped
    blocked_reason: Waiting for decision
    blocked_type: decision
    requires_human_review: false
    created: "2026-01-01T00:00:00+00:00"
    updated: "2026-01-01T00:00:00+00:00"
"#,
    )
    .unwrap();

    let result = migrate_v1_to_v2(&target, &default_feature_pipeline());
    assert!(result.is_ok(), "Migration failed: {:?}", result);

    let backlog = result.unwrap();
    let item = &backlog.items[0];
    assert_eq!(item.status, ItemStatus::Blocked);
    // scoped -> Ready in v2
    assert_eq!(item.blocked_from_status, Some(ItemStatus::Ready));
    assert_eq!(item.blocked_reason.as_deref(), Some("Waiting for decision"));
}

// --- New status item passes through ---

#[test]
fn migrate_v1_new_status_maps_to_new() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    fs::write(
        &target,
        r#"schema_version: 1
items:
  - id: WRK-001
    title: Fresh item
    status: new
    requires_human_review: false
    created: "2026-01-01T00:00:00+00:00"
    updated: "2026-01-01T00:00:00+00:00"
"#,
    )
    .unwrap();

    let result = migrate_v1_to_v2(&target, &default_feature_pipeline());
    assert!(result.is_ok(), "Migration failed: {:?}", result);

    let backlog = result.unwrap();
    let item = &backlog.items[0];
    assert_eq!(item.status, ItemStatus::New);
    assert_eq!(item.phase, None);
    assert_eq!(item.pipeline_type.as_deref(), Some("feature"));
}

// --- Missing schema_version defaults to v1 ---

#[test]
fn migrate_missing_schema_version_treated_as_v1() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    fs::write(
        &target,
        r#"items:
  - id: WRK-001
    title: No version field
    status: new
    requires_human_review: false
    created: "2026-01-01T00:00:00+00:00"
    updated: "2026-01-01T00:00:00+00:00"
"#,
    )
    .unwrap();

    let result = migrate_v1_to_v2(&target, &default_feature_pipeline());
    assert!(
        result.is_ok(),
        "Migration without schema_version failed: {:?}",
        result
    );

    let backlog = result.unwrap();
    assert_eq!(backlog.schema_version, 2);
    assert_eq!(backlog.items[0].status, ItemStatus::New);
}

// --- All v1 pipeline_type set to "feature" ---

#[test]
fn migrate_v1_sets_pipeline_type_feature_on_all_items() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    let v1_fixture = common::fixtures_dir().join("backlog_v1_full.yaml");
    fs::copy(&v1_fixture, &target).unwrap();

    let backlog = migrate_v1_to_v2(&target, &default_feature_pipeline()).unwrap();

    for item in &backlog.items {
        assert_eq!(
            item.pipeline_type.as_deref(),
            Some("feature"),
            "Item {} should have pipeline_type 'feature'",
            item.id
        );
    }
}

// --- Invalid YAML fails gracefully ---

#[test]
fn migrate_invalid_yaml_returns_error() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    fs::write(&target, "not valid yaml {{{{").unwrap();

    let result = migrate_v1_to_v2(&target, &default_feature_pipeline());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("parse"));
}

// --- Missing file fails gracefully ---

#[test]
fn migrate_missing_file_returns_error() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("nonexistent.yaml");

    let result = migrate_v1_to_v2(&target, &default_feature_pipeline());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Failed to read"));
}

// --- Phase validation: invalid phase cleared ---

#[test]
fn migrate_v1_invalid_phase_cleared_by_validation() {
    use phase_golem::config::{PhaseConfig, PipelineConfig};

    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    // Create a V1 YAML with an item having phase: prd and status: in_progress
    fs::write(
        &target,
        r#"schema_version: 1
items:
  - id: WRK-001
    title: Item with invalid phase
    status: in_progress
    phase: prd
    requires_human_review: false
    created: "2026-01-01T00:00:00+00:00"
    updated: "2026-01-01T00:00:00+00:00"
"#,
    )
    .unwrap();

    // Build a custom pipeline that excludes "prd" from its phases
    let custom_pipeline = PipelineConfig {
        pre_phases: vec![PhaseConfig::new("research", false)],
        phases: vec![
            PhaseConfig::new("design", false),
            PhaseConfig::new("build", true),
            PhaseConfig::new("review", false),
        ],
    };

    let result = migrate_v1_to_v2(&target, &custom_pipeline);
    assert!(result.is_ok(), "Migration failed: {:?}", result);

    let backlog = result.unwrap();
    let item = &backlog.items[0];
    assert_eq!(item.phase, None, "Invalid phase should be cleared");
    assert_eq!(
        item.phase_pool, None,
        "phase_pool should be cleared when phase is invalid"
    );
}

// --- Phase validation: None phase skips validation ---

#[test]
fn migrate_v1_none_phase_skips_validation() {
    use phase_golem::config::{PhaseConfig, PipelineConfig};

    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    // Create a V1 YAML with an item that has no phase set
    fs::write(
        &target,
        r#"schema_version: 1
items:
  - id: WRK-001
    title: Item without phase
    status: new
    requires_human_review: false
    created: "2026-01-01T00:00:00+00:00"
    updated: "2026-01-01T00:00:00+00:00"
"#,
    )
    .unwrap();

    // Custom pipeline with only one phase
    let custom_pipeline = PipelineConfig {
        pre_phases: Vec::new(),
        phases: vec![PhaseConfig::new("build", true)],
    };

    let result = migrate_v1_to_v2(&target, &custom_pipeline);
    assert!(result.is_ok(), "Migration failed: {:?}", result);

    let backlog = result.unwrap();
    let item = &backlog.items[0];
    assert_eq!(item.phase, None, "Phase should remain None");
}

// --- parse_description tests ---

#[test]
fn parse_description_convention_formatted() {
    let text = "\
Context: The coordinator provides backlog snapshots via handle_get_snapshot(),\n\
which does a full .clone() of all backlog items.\n\
Problem: With large backlogs (1000+ items), each clone is a significant\n\
allocation that creates per-iteration memory pressure.\n\
Solution: Investigate using Arc-wrapped immutable data structures so snapshots\n\
can be shared cheaply without deep cloning.\n\
Impact: Reduces memory allocation churn proportional to backlog size.\n\
Sizing rationale: Medium size and complexity because it touches the\n\
coordinator-scheduler boundary.";

    let result = parse_description(text);

    assert_eq!(
        result.context,
        "The coordinator provides backlog snapshots via handle_get_snapshot(),\n\
         which does a full .clone() of all backlog items."
    );
    assert_eq!(
        result.problem,
        "With large backlogs (1000+ items), each clone is a significant\n\
         allocation that creates per-iteration memory pressure."
    );
    assert_eq!(
        result.solution,
        "Investigate using Arc-wrapped immutable data structures so snapshots\n\
         can be shared cheaply without deep cloning."
    );
    assert_eq!(
        result.impact,
        "Reduces memory allocation churn proportional to backlog size."
    );
    assert_eq!(
        result.sizing_rationale,
        "Medium size and complexity because it touches the\n\
         coordinator-scheduler boundary."
    );
}

#[test]
fn parse_description_convention_with_colons_in_content() {
    let text = "\
Context: Items have a description field (Option<String>) that accepts any content.\n\
Problem: Without a structured type, agents produce inconsistent descriptions: some are good, some are not.\n\
Solution: Replace description: Option<String> with a structured type.\n\
Impact: Consistent, high-quality descriptions.\n\
Sizing rationale: Medium size/complexity because it changes the BacklogItem type.";

    let result = parse_description(text);

    assert_eq!(
        result.context,
        "Items have a description field (Option<String>) that accepts any content."
    );
    assert!(result
        .problem
        .contains("inconsistent descriptions: some are good"));
    assert!(result
        .solution
        .contains("Replace description: Option<String>"));
    assert_eq!(result.impact, "Consistent, high-quality descriptions.");
    assert!(result.sizing_rationale.contains("Medium size/complexity"));
}

#[test]
fn parse_description_freeform_no_headers() {
    let text = "Two improvements to orchestrator terminal output during phase transitions: \
(1) Print a one-liner with the item title when starting work on a phase. \
(2) Add a bold visual separator between phases.";

    let result = parse_description(text);

    assert_eq!(result.context, text.trim());
    assert_eq!(result.problem, "");
    assert_eq!(result.solution, "");
    assert_eq!(result.impact, "");
    assert_eq!(result.sizing_rationale, "");
}

#[test]
fn parse_description_freeform_multiline_no_headers() {
    let text = "\
The init command should create an empty BACKLOG_INBOX.yaml alongside BACKLOG.yaml\n\
so users know the file exists and can see the expected format. Include a commented-out\n\
example item showing the available fields.";

    let result = parse_description(text);

    assert_eq!(result.context, text.trim());
    assert_eq!(result.problem, "");
    assert_eq!(result.solution, "");
    assert_eq!(result.impact, "");
    assert_eq!(result.sizing_rationale, "");
}

#[test]
fn parse_description_empty_string() {
    let result = parse_description("");

    assert_eq!(result.context, "");
    assert_eq!(result.problem, "");
    assert_eq!(result.solution, "");
    assert_eq!(result.impact, "");
    assert_eq!(result.sizing_rationale, "");
}

#[test]
fn parse_description_whitespace_only() {
    let result = parse_description("   \n  \n   ");

    assert_eq!(result.context, "");
    assert_eq!(result.problem, "");
    assert_eq!(result.solution, "");
    assert_eq!(result.impact, "");
    assert_eq!(result.sizing_rationale, "");
}

#[test]
fn parse_description_partial_headers() {
    let text = "\
Context: Some context here.\n\
Solution: A proposed solution.";

    let result = parse_description(text);

    assert_eq!(result.context, "Some context here.");
    assert_eq!(result.problem, "");
    assert_eq!(result.solution, "A proposed solution.");
    assert_eq!(result.impact, "");
    assert_eq!(result.sizing_rationale, "");
}

#[test]
fn parse_description_duplicate_headers_later_wins() {
    let text = "\
Context: First context.\n\
Problem: A problem.\n\
Context: Second context replaces first.";

    let result = parse_description(text);

    assert_eq!(result.context, "Second context replaces first.");
    assert_eq!(result.problem, "A problem.");
}

#[test]
fn parse_description_header_with_content_on_same_line() {
    let text = "Context: some text here";

    let result = parse_description(text);

    assert_eq!(result.context, "some text here");
    assert_eq!(result.problem, "");
}

#[test]
fn parse_description_case_insensitive_headers() {
    let text = "\
CONTEXT: Upper case context.\n\
problem: lower case problem.\n\
SOLUTION: Upper case solution.\n\
Impact: Mixed case impact.\n\
SIZING RATIONALE: Upper sizing rationale.";

    let result = parse_description(text);

    assert_eq!(result.context, "Upper case context.");
    assert_eq!(result.problem, "lower case problem.");
    assert_eq!(result.solution, "Upper case solution.");
    assert_eq!(result.impact, "Mixed case impact.");
    assert_eq!(result.sizing_rationale, "Upper sizing rationale.");
}

#[test]
fn parse_description_roundtrip_serde() {
    let desc = StructuredDescription {
        context: "Some context".to_string(),
        problem: "A problem".to_string(),
        solution: "The solution".to_string(),
        impact: "High impact".to_string(),
        sizing_rationale: "Small because minimal changes".to_string(),
    };

    let yaml = serde_yaml_ng::to_string(&desc).expect("Failed to serialize");
    let deserialized: StructuredDescription =
        serde_yaml_ng::from_str(&yaml).expect("Failed to deserialize");

    assert_eq!(desc, deserialized);
}

#[test]
fn parse_description_pre_header_text_lands_in_context() {
    let text = "\
This item is about improving perf.\n\
Problem: Latency is too high.\n\
Solution: Add caching.";

    let result = parse_description(text);

    assert_eq!(result.context, "This item is about improving perf.");
    assert_eq!(result.problem, "Latency is too high.");
    assert_eq!(result.solution, "Add caching.");
    assert_eq!(result.impact, "");
    assert_eq!(result.sizing_rationale, "");
}

// =============================================================================
// V2 → V3 migration tests
// =============================================================================

#[test]
fn migrate_v2_full_fixture() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    let v2_fixture = common::fixtures_dir().join("backlog_v2_full.yaml");
    fs::copy(&v2_fixture, &target).unwrap();

    let result = migrate_v2_to_v3(&target);
    assert!(result.is_ok(), "Migration failed: {:?}", result);

    let backlog = result.unwrap();
    assert_eq!(backlog.schema_version, 3);
    assert_eq!(backlog.items.len(), 3);

    // Items without descriptions should have description: None
    assert_eq!(backlog.items[0].description, None);
    assert_eq!(backlog.items[1].description, None);
    assert_eq!(backlog.items[2].description, None);

    // Other fields should be preserved
    assert_eq!(backlog.items[0].id, "WRK-001");
    assert_eq!(backlog.items[0].status, ItemStatus::InProgress);
    assert_eq!(backlog.items[0].phase, Some("build".to_string()));
    assert_eq!(backlog.items[2].status, ItemStatus::Blocked);
}

#[test]
fn migrate_v2_with_structured_descriptions() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    let v2_fixture = common::fixtures_dir().join("backlog_v2_with_descriptions.yaml");
    fs::copy(&v2_fixture, &target).unwrap();

    let result = migrate_v2_to_v3(&target);
    assert!(result.is_ok(), "Migration failed: {:?}", result);

    let backlog = result.unwrap();
    assert_eq!(backlog.schema_version, 3);
    assert_eq!(backlog.items.len(), 3);

    // WRK-001 has convention-formatted description
    let desc1 = backlog.items[0]
        .description
        .as_ref()
        .expect("WRK-001 should have description");
    assert_eq!(desc1.context, "Users need to log in.");
    assert_eq!(desc1.problem, "No auth exists.");
    assert_eq!(desc1.solution, "Add JWT-based auth.");
    assert_eq!(desc1.impact, "Enables user-specific features.");
    assert_eq!(
        desc1.sizing_rationale,
        "Medium because it touches multiple layers."
    );

    // WRK-002 has freeform description (no headers) — lands in context
    let desc2 = backlog.items[1]
        .description
        .as_ref()
        .expect("WRK-002 should have description");
    assert_eq!(
        desc2.context,
        "Just a simple typo fix, no structured headers"
    );
    assert_eq!(desc2.problem, "");
    assert_eq!(desc2.solution, "");

    // WRK-003 has no description
    assert_eq!(backlog.items[2].description, None);
}

#[test]
fn migrate_v2_persisted_file_is_valid_v3() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    let v2_fixture = common::fixtures_dir().join("backlog_v2_full.yaml");
    fs::copy(&v2_fixture, &target).unwrap();

    migrate_v2_to_v3(&target).unwrap();

    // Re-read the persisted file and verify it's valid v3
    let contents = fs::read_to_string(&target).unwrap();
    assert!(
        contents.contains("schema_version: 3"),
        "Persisted file should have schema_version 3"
    );

    // Should be loadable as current schema
    let reloaded = phase_golem::backlog::load(&target, target.parent().unwrap()).unwrap();
    assert_eq!(reloaded.schema_version, 3);
}

#[test]
fn migrate_v2_empty_backlog() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    fs::write(&target, "schema_version: 2\nitems: []\n").unwrap();

    let result = migrate_v2_to_v3(&target);
    assert!(
        result.is_ok(),
        "Empty backlog migration failed: {:?}",
        result
    );

    let backlog = result.unwrap();
    assert_eq!(backlog.schema_version, 3);
    assert!(backlog.items.is_empty());
}

#[test]
fn migrate_v2_rejects_wrong_version() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    fs::write(&target, "schema_version: 1\nitems: []\n").unwrap();

    let result = migrate_v2_to_v3(&target);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("expected schema_version 2"));
}

#[test]
fn migrate_v2_invalid_yaml_returns_error() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    // Use truly invalid YAML that can't be parsed at all
    fs::write(&target, "\t\x00invalid").unwrap();

    let result = migrate_v2_to_v3(&target);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("parse") || err.contains("expected schema_version 2"),
        "Expected parse-related error, got: {}",
        err
    );
}

#[test]
fn migrate_v2_missing_file_returns_error() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("nonexistent.yaml");

    let result = migrate_v2_to_v3(&target);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Failed to read"));
}

#[test]
fn migrate_v2_preserves_next_item_id() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    fs::write(&target, "schema_version: 2\nitems: []\nnext_item_id: 42\n").unwrap();

    let result = migrate_v2_to_v3(&target);
    assert!(result.is_ok());

    let backlog = result.unwrap();
    assert_eq!(backlog.next_item_id, 42);
}

#[test]
fn migrate_chain_v1_to_v3_via_load() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("BACKLOG.yaml");

    let v1_fixture = common::fixtures_dir().join("backlog_v1_full.yaml");
    fs::copy(&v1_fixture, &target).unwrap();

    // Create a minimal phase-golem.toml so load() can find the feature pipeline
    let config_path = dir.path().join("phase-golem.toml");
    fs::write(&config_path, "").unwrap();

    // load() should chain v1→v2→v3 automatically
    let backlog = phase_golem::backlog::load(&target, dir.path()).unwrap();

    assert_eq!(backlog.schema_version, 3);
    assert_eq!(backlog.items.len(), 5);

    // Verify file on disk is now v3
    let raw = fs::read_to_string(&target).unwrap();
    assert!(raw.contains("schema_version: 3"));
}
