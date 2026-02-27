mod common;

use std::path::Path;

use phase_golem::config::{PhaseConfig, PhaseGolemConfig, PipelineConfig, StalenessAction};
use phase_golem::pg_item::{self, PgItem};
use phase_golem::preflight::{run_preflight, PreflightError};
use phase_golem::types::{ItemStatus, PhasePool};

// --- Test project root with .task-golem/ directory ---

/// Returns a stable project root path with a `.task-golem/` directory.
///
/// Uses `/tmp/pg-preflight-test` and creates `.task-golem/` on first call.
/// All preflight tests share this root since they only read (never write) from it.
fn test_project_root() -> &'static Path {
    use std::sync::Once;
    static INIT: Once = Once::new();
    static DIR: &str = "/tmp/pg-preflight-test";
    INIT.call_once(|| {
        std::fs::create_dir_all(format!("{}/.task-golem", DIR))
            .expect("Failed to create .task-golem dir for preflight tests");
    });
    Path::new(DIR)
}

// --- Test helpers ---

fn make_feature_item(id: &str, status: ItemStatus) -> PgItem {
    let mut pg = common::make_pg_item(id, status);
    pg_item::set_pipeline_type(&mut pg.0, Some("feature"));
    pg
}

/// Build a default feature pipeline with empty workflow lists.
///
/// This mirrors the structure of `default_feature_pipeline()` but with no workflow
/// file references, so the workflow probe phase does not require real files on disk.
fn feature_pipeline_no_workflows() -> PipelineConfig {
    PipelineConfig {
        pre_phases: vec![PhaseConfig::new("research", false)],
        phases: vec![
            PhaseConfig::new("prd", false),
            PhaseConfig::new("tech-research", false),
            PhaseConfig::new("design", false),
            PhaseConfig::new("spec", false),
            PhaseConfig::new("build", true),
            PhaseConfig::new("review", false),
        ],
    }
}

fn default_config() -> PhaseGolemConfig {
    let mut config = PhaseGolemConfig::default();
    config
        .pipelines
        .insert("feature".to_string(), feature_pipeline_no_workflows());
    config
}

// --- .task-golem/ directory existence check ---

#[test]
fn preflight_fails_when_task_golem_dir_missing() {
    let dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    // Do NOT create .task-golem/ — that's the point of the test

    let config = default_config();
    let items: Vec<PgItem> = vec![];

    let result = run_preflight(&config, &items, dir.path(), dir.path());

    let errors = result.expect_err("Should fail when .task-golem/ is missing");
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0].condition.contains(".task-golem/"),
        "Error should mention .task-golem/ directory: {:?}",
        errors[0].condition
    );
    assert!(
        errors[0].suggested_fix.contains("tg init"),
        "Fix should suggest `tg init`: {:?}",
        errors[0].suggested_fix
    );
}

#[test]
fn preflight_passes_when_task_golem_dir_exists() {
    let dir = tempfile::TempDir::new().expect("Failed to create temp dir");
    std::fs::create_dir_all(dir.path().join(".task-golem"))
        .expect("Failed to create .task-golem dir");

    let config = default_config();
    let items: Vec<PgItem> = vec![];

    let result = run_preflight(&config, &items, dir.path(), dir.path());

    assert!(result.is_ok(), "Should pass when .task-golem/ exists");
}

// --- Structural validation tests ---

#[test]
fn preflight_valid_config_passes() {
    let config = default_config();
    let items: Vec<PgItem> = vec![];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

#[test]
fn preflight_no_main_phases_fails() {
    let mut config = default_config();
    config.pipelines.insert(
        "empty".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![],
        },
    );

    let items: Vec<PgItem> = vec![];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("no main phases")));
}

#[test]
fn preflight_duplicate_phase_names_fails() {
    let mut config = default_config();
    config.pipelines.insert(
        "dup".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![
                PhaseConfig {
                    workflows: vec!["workflow1.md".to_string()],
                    ..PhaseConfig::new("build", false)
                },
                PhaseConfig {
                    workflows: vec!["workflow2.md".to_string()],
                    ..PhaseConfig::new("build", false)
                },
            ],
        },
    );

    let items: Vec<PgItem> = vec![];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("Duplicate phase name")));
}

#[test]
fn preflight_destructive_pre_phase_fails() {
    let mut config = default_config();
    config.pipelines.insert(
        "bad".to_string(),
        PipelineConfig {
            pre_phases: vec![PhaseConfig {
                workflows: vec!["workflow.md".to_string()],
                ..PhaseConfig::new("research", true)
            }],
            phases: vec![PhaseConfig {
                workflows: vec!["workflow.md".to_string()],
                ..PhaseConfig::new("build", false)
            }],
        },
    );

    let items: Vec<PgItem> = vec![];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("cannot be destructive")));
}

#[test]
fn preflight_max_wip_zero_fails() {
    let mut config = default_config();
    config.execution.max_wip = 0;

    let items: Vec<PgItem> = vec![];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.condition.contains("max_wip")));
}

#[test]
fn preflight_max_concurrent_zero_fails() {
    let mut config = default_config();
    config.execution.max_concurrent = 0;

    let items: Vec<PgItem> = vec![];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("max_concurrent")));
}

#[test]
fn preflight_staleness_block_with_max_wip_gt_1_fails() {
    let mut config = default_config();
    config.execution.max_wip = 2;
    config.pipelines.insert(
        "stale".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![PhaseConfig {
                workflows: vec!["workflow.md".to_string()],
                staleness: StalenessAction::Block,
                ..PhaseConfig::new("build", true)
            }],
        },
    );

    let items: Vec<PgItem> = vec![];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("staleness") && e.condition.contains("block")));
}

// --- Error format tests ---

#[test]
fn preflight_errors_contain_config_location() {
    let mut config = default_config();
    config.execution.max_wip = 0;

    let items: Vec<PgItem> = vec![];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    let error = &errors[0];
    assert!(error.config_location.contains("phase-golem.toml"));
    assert!(!error.suggested_fix.is_empty());
}

#[test]
fn preflight_error_display_format() {
    let error = PreflightError {
        condition: "max_wip must be >= 1".to_string(),
        config_location: "phase-golem.toml → execution.max_wip".to_string(),
        suggested_fix: "Set max_wip to at least 1".to_string(),
    };

    let display = format!("{}", error);
    assert!(display.contains("Preflight error:"));
    assert!(display.contains("Config:"));
    assert!(display.contains("Fix:"));
}

// --- Workflow probe tests ---

#[test]
fn preflight_workflow_files_exist_passes() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join(".task-golem")).unwrap();

    // Create a minimal config with workflow files that we'll create on disk
    let mut config = PhaseGolemConfig::default();
    let workflow_path = "workflows/build.md";
    std::fs::create_dir_all(root.join("workflows")).unwrap();
    std::fs::write(root.join(workflow_path), "# Build workflow\n").unwrap();

    config.pipelines.insert(
        "test".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![PhaseConfig {
                workflows: vec![workflow_path.to_string()],
                ..PhaseConfig::new("build", false)
            }],
        },
    );

    let items: Vec<PgItem> = vec![];
    let result = run_preflight(&config, &items, root, root);

    assert!(result.is_ok());
}

#[test]
fn preflight_missing_workflow_files_fails() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join(".task-golem")).unwrap();

    let mut config = PhaseGolemConfig::default();
    config.pipelines.insert(
        "test".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![PhaseConfig {
                workflows: vec!["workflows/nonexistent.md".to_string()],
                ..PhaseConfig::new("build", false)
            }],
        },
    );

    let items: Vec<PgItem> = vec![];
    let result = run_preflight(&config, &items, root, root);

    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("Workflow file not found")));
}

// --- Item validation tests ---

#[test]
fn preflight_valid_in_progress_item_passes() {
    let config = default_config();
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    pg_item::set_phase(&mut item.0, Some("prd"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));
    pg_item::set_pipeline_type(&mut item.0, Some("feature"));

    let items = vec![item];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

#[test]
fn preflight_invalid_pipeline_type_fails() {
    let config = default_config();
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    pg_item::set_phase(&mut item.0, Some("prd"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));
    pg_item::set_pipeline_type(&mut item.0, Some("nonexistent"));

    let items = vec![item];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("unknown pipeline type")));
}

#[test]
fn preflight_invalid_phase_name_fails() {
    let config = default_config();
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    pg_item::set_phase(&mut item.0, Some("nonexistent-phase"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));
    pg_item::set_pipeline_type(&mut item.0, Some("feature"));

    let items = vec![item];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.condition.contains("unknown phase")));
}

#[test]
fn preflight_mismatched_phase_pool_fails() {
    let config = default_config();
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    pg_item::set_phase(&mut item.0, Some("research")); // research is in pre_phases
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main)); // But pool says main
    pg_item::set_pipeline_type(&mut item.0, Some("feature"));

    let items = vec![item];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.condition.contains("phase_pool")));
}

#[test]
fn preflight_skips_new_and_done_items() {
    let config = default_config();
    // These items have invalid pipeline_type but should be skipped
    let mut new_item = make_feature_item("WRK-001", ItemStatus::New);
    pg_item::set_pipeline_type(&mut new_item.0, Some("nonexistent"));

    let mut done_item = make_feature_item("WRK-002", ItemStatus::Done);
    pg_item::set_pipeline_type(&mut done_item.0, Some("nonexistent"));

    let items = vec![new_item, done_item];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

#[test]
fn preflight_validates_scoping_items() {
    let config = default_config();
    let mut item = make_feature_item("WRK-001", ItemStatus::Scoping);
    pg_item::set_phase(&mut item.0, Some("research"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Pre));
    pg_item::set_pipeline_type(&mut item.0, Some("feature"));

    let items = vec![item];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

#[test]
fn preflight_item_with_default_pipeline_type_passes() {
    let config = default_config();
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    pg_item::set_phase(&mut item.0, Some("prd"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));
    pg_item::set_pipeline_type(&mut item.0, None); // Should default to "feature"

    let items = vec![item];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

// --- Duplicate ID validation tests ---

#[test]
fn preflight_empty_backlog_no_duplicate_errors() {
    let config = default_config();
    let items: Vec<PgItem> = vec![];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

#[test]
fn preflight_single_item_no_duplicate_errors() {
    let config = default_config();
    let item = make_feature_item("WRK-001", ItemStatus::New);

    let items = vec![item];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

#[test]
fn preflight_unique_ids_no_duplicate_errors() {
    let config = default_config();
    let item_a = make_feature_item("WRK-001", ItemStatus::New);
    let item_b = make_feature_item("WRK-002", ItemStatus::Ready);
    let item_c = make_feature_item("WRK-003", ItemStatus::Done);

    let items = vec![item_a, item_b, item_c];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

#[test]
fn preflight_duplicate_id_pair_fails() {
    let config = default_config();
    let item_a = make_feature_item("WRK-001", ItemStatus::New);
    let item_b = make_feature_item("WRK-001", ItemStatus::Done);

    let items = vec![item_a, item_b];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    let dup_errors: Vec<_> = errors
        .iter()
        .filter(|e| e.condition.contains("Duplicate item ID"))
        .collect();
    assert_eq!(dup_errors.len(), 1);
    assert!(dup_errors[0].condition.contains("WRK-001"));
    assert!(dup_errors[0].condition.contains("[0, 1]"));
    assert_eq!(dup_errors[0].config_location, "BACKLOG.yaml → items");
    assert!(dup_errors[0]
        .suggested_fix
        .contains("Remove or rename the duplicate item"));
}

#[test]
fn preflight_multiple_distinct_duplicate_ids_fails() {
    let config = default_config();
    let item_a = make_feature_item("WRK-002", ItemStatus::New);
    let item_b = make_feature_item("WRK-001", ItemStatus::Ready);
    let item_c = make_feature_item("WRK-002", ItemStatus::Done);
    let item_d = make_feature_item("WRK-001", ItemStatus::New);

    let items = vec![item_a, item_b, item_c, item_d];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    let dup_errors: Vec<_> = errors
        .iter()
        .filter(|e| e.condition.contains("Duplicate item ID"))
        .collect();
    assert_eq!(dup_errors.len(), 2);
    // Errors should be ordered by first occurrence index
    // WRK-002 appears first at index 0, WRK-001 appears first at index 1
    assert!(dup_errors[0].condition.contains("WRK-002"));
    assert!(dup_errors[1].condition.contains("WRK-001"));
}

#[test]
fn preflight_three_way_duplicate_id_fails() {
    let config = default_config();
    let item_a = make_feature_item("WRK-001", ItemStatus::New);
    let item_b = make_feature_item("WRK-002", ItemStatus::Ready);
    let item_c = make_feature_item("WRK-001", ItemStatus::Done);
    let item_d = make_feature_item("WRK-003", ItemStatus::New);
    let item_e = make_feature_item("WRK-001", ItemStatus::InProgress);

    let items = vec![item_a, item_b, item_c, item_d, item_e];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    let dup_errors: Vec<_> = errors
        .iter()
        .filter(|e| e.condition.contains("Duplicate item ID"))
        .collect();
    assert_eq!(dup_errors.len(), 1);
    assert!(dup_errors[0].condition.contains("WRK-001"));
    assert!(dup_errors[0].condition.contains("[0, 2, 4]"));
}

#[test]
fn preflight_case_sensitive_ids_not_duplicates() {
    let config = default_config();
    let item_a = make_feature_item("WRK-001", ItemStatus::New);
    let item_b = make_feature_item("wrk-001", ItemStatus::Ready);

    let items = vec![item_a, item_b];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

// --- Dependency graph validation: dangling references ---

#[test]
fn preflight_dangling_dependency_fails() {
    let config = default_config();
    let item = pg_item::new_from_parts(
        "WRK-001".to_string(),
        "Test item WRK-001".to_string(),
        ItemStatus::Ready,
        vec!["WRK-999".to_string()],
        vec![],
    );

    let items = vec![item];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("WRK-999") && e.condition.contains("does not exist")));
}

#[test]
fn preflight_multiple_dangling_references() {
    let config = default_config();
    let item_a = pg_item::new_from_parts(
        "WRK-001".to_string(),
        "Test item WRK-001".to_string(),
        ItemStatus::Ready,
        vec!["WRK-888".to_string()],
        vec![],
    );

    let item_b = pg_item::new_from_parts(
        "WRK-002".to_string(),
        "Test item WRK-002".to_string(),
        ItemStatus::Ready,
        vec!["WRK-999".to_string()],
        vec![],
    );

    let items = vec![item_a, item_b];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    let dangling_errors: Vec<_> = errors
        .iter()
        .filter(|e| e.condition.contains("does not exist"))
        .collect();
    assert_eq!(dangling_errors.len(), 2);
}

#[test]
fn preflight_valid_dependencies_passes() {
    let config = default_config();
    let item_a = make_feature_item("WRK-001", ItemStatus::Done);
    let item_b = pg_item::new_from_parts(
        "WRK-002".to_string(),
        "Test item WRK-002".to_string(),
        ItemStatus::Ready,
        vec!["WRK-001".to_string()],
        vec![],
    );

    let items = vec![item_a, item_b];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

// --- Dependency graph validation: cycle detection ---

#[test]
fn preflight_self_dependency_fails() {
    let config = default_config();
    let item = pg_item::new_from_parts(
        "WRK-001".to_string(),
        "Test item WRK-001".to_string(),
        ItemStatus::Ready,
        vec!["WRK-001".to_string()],
        vec![],
    );

    let items = vec![item];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    let cycle_errors: Vec<_> = errors
        .iter()
        .filter(|e| e.condition.contains("Circular dependency"))
        .collect();
    assert_eq!(cycle_errors.len(), 1);
    assert!(cycle_errors[0].condition.contains("WRK-001 → WRK-001"));
}

#[test]
fn preflight_two_node_cycle_fails() {
    let config = default_config();
    let item_a = pg_item::new_from_parts(
        "WRK-001".to_string(),
        "Test item WRK-001".to_string(),
        ItemStatus::Ready,
        vec!["WRK-002".to_string()],
        vec![],
    );

    let item_b = pg_item::new_from_parts(
        "WRK-002".to_string(),
        "Test item WRK-002".to_string(),
        ItemStatus::Ready,
        vec!["WRK-001".to_string()],
        vec![],
    );

    let items = vec![item_a, item_b];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    let cycle_errors: Vec<_> = errors
        .iter()
        .filter(|e| e.condition.contains("Circular dependency"))
        .collect();
    assert_eq!(cycle_errors.len(), 1);
    // Check that the cycle path format contains " → "
    assert!(cycle_errors.iter().any(|e| {
        let cond = &e.condition;
        cond.contains("WRK-001 → WRK-002") || cond.contains("WRK-002 → WRK-001")
    }));
}

#[test]
fn preflight_three_node_cycle_fails() {
    let config = default_config();
    let item_a = pg_item::new_from_parts(
        "WRK-001".to_string(),
        "Test item WRK-001".to_string(),
        ItemStatus::Ready,
        vec!["WRK-002".to_string()],
        vec![],
    );

    let item_b = pg_item::new_from_parts(
        "WRK-002".to_string(),
        "Test item WRK-002".to_string(),
        ItemStatus::Ready,
        vec!["WRK-003".to_string()],
        vec![],
    );

    let item_c = pg_item::new_from_parts(
        "WRK-003".to_string(),
        "Test item WRK-003".to_string(),
        ItemStatus::Ready,
        vec!["WRK-001".to_string()],
        vec![],
    );

    let items = vec![item_a, item_b, item_c];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    let cycle_errors: Vec<_> = errors
        .iter()
        .filter(|e| e.condition.contains("Circular dependency"))
        .collect();
    assert_eq!(cycle_errors.len(), 1);
    // Check that the full path is present in " → " format
    let cycle_cond = &cycle_errors[0].condition;
    assert!(cycle_cond.contains(" → "));
    // The cycle should contain all three items
    assert!(cycle_cond.contains("WRK-001"));
    assert!(cycle_cond.contains("WRK-002"));
    assert!(cycle_cond.contains("WRK-003"));
}

#[test]
fn preflight_multiple_independent_cycles() {
    let config = default_config();
    // Cycle 1: A <-> B
    let item_a = pg_item::new_from_parts(
        "WRK-001".to_string(),
        "Test item WRK-001".to_string(),
        ItemStatus::Ready,
        vec!["WRK-002".to_string()],
        vec![],
    );

    let item_b = pg_item::new_from_parts(
        "WRK-002".to_string(),
        "Test item WRK-002".to_string(),
        ItemStatus::Ready,
        vec!["WRK-001".to_string()],
        vec![],
    );

    // Cycle 2: C <-> D
    let item_c = pg_item::new_from_parts(
        "WRK-003".to_string(),
        "Test item WRK-003".to_string(),
        ItemStatus::Ready,
        vec!["WRK-004".to_string()],
        vec![],
    );

    let item_d = pg_item::new_from_parts(
        "WRK-004".to_string(),
        "Test item WRK-004".to_string(),
        ItemStatus::Ready,
        vec!["WRK-003".to_string()],
        vec![],
    );

    let items = vec![item_a, item_b, item_c, item_d];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    let cycle_errors: Vec<_> = errors
        .iter()
        .filter(|e| e.condition.contains("Circular dependency"))
        .collect();
    assert_eq!(cycle_errors.len(), 2);
    // Both cycle paths should be present
    let all_conditions: String = cycle_errors
        .iter()
        .map(|e| e.condition.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(all_conditions.contains("WRK-001") && all_conditions.contains("WRK-002"));
    assert!(all_conditions.contains("WRK-003") && all_conditions.contains("WRK-004"));
}

#[test]
fn preflight_cycle_with_blocked_item_detected() {
    let config = default_config();
    let item_a = pg_item::new_from_parts(
        "WRK-001".to_string(),
        "Test item WRK-001".to_string(),
        ItemStatus::Ready,
        vec!["WRK-002".to_string()],
        vec![],
    );

    let mut item_b = common::make_blocked_pg_item("WRK-002", ItemStatus::Ready);
    item_b.0.dependencies = vec!["WRK-001".to_string()];

    let items = vec![item_a, item_b];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("Circular dependency")));
}

#[test]
fn preflight_done_items_excluded_from_cycle_detection() {
    let config = default_config();
    // A depends on B (Done), B depends on A — but B is Done so no cycle
    let item_a = pg_item::new_from_parts(
        "WRK-001".to_string(),
        "Test item WRK-001".to_string(),
        ItemStatus::Ready,
        vec!["WRK-002".to_string()],
        vec![],
    );

    let item_b = pg_item::new_from_parts(
        "WRK-002".to_string(),
        "Test item WRK-002".to_string(),
        ItemStatus::Done,
        vec!["WRK-001".to_string()],
        vec![],
    );

    let items = vec![item_a, item_b];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

#[test]
fn preflight_diamond_dag_no_false_positive() {
    let config = default_config();
    // Diamond: A→B, A→C, B→D, C→D (not a cycle)
    let item_a = pg_item::new_from_parts(
        "WRK-001".to_string(),
        "Test item WRK-001".to_string(),
        ItemStatus::Ready,
        vec!["WRK-002".to_string(), "WRK-003".to_string()],
        vec![],
    );

    let item_b = pg_item::new_from_parts(
        "WRK-002".to_string(),
        "Test item WRK-002".to_string(),
        ItemStatus::Ready,
        vec!["WRK-004".to_string()],
        vec![],
    );

    let item_c = pg_item::new_from_parts(
        "WRK-003".to_string(),
        "Test item WRK-003".to_string(),
        ItemStatus::Ready,
        vec!["WRK-004".to_string()],
        vec![],
    );

    let item_d = make_feature_item("WRK-004", ItemStatus::Ready);

    let items = vec![item_a, item_b, item_c, item_d];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

#[test]
fn preflight_transitive_chain_no_cycle() {
    let config = default_config();
    // C→B→A (valid DAG chain)
    let item_a = make_feature_item("WRK-001", ItemStatus::Ready);

    let item_b = pg_item::new_from_parts(
        "WRK-002".to_string(),
        "Test item WRK-002".to_string(),
        ItemStatus::Ready,
        vec!["WRK-001".to_string()],
        vec![],
    );

    let item_c = pg_item::new_from_parts(
        "WRK-003".to_string(),
        "Test item WRK-003".to_string(),
        ItemStatus::Ready,
        vec!["WRK-002".to_string()],
        vec![],
    );

    let items = vec![item_a, item_b, item_c];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

#[test]
fn preflight_no_dependencies_passes() {
    let config = default_config();
    let item_a = make_feature_item("WRK-001", ItemStatus::Ready);
    let item_b = make_feature_item("WRK-002", ItemStatus::Ready);

    let items = vec![item_a, item_b];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    assert!(result.is_ok());
}

// --- Phase 3 gating tests ---

#[test]
fn preflight_phase3_skipped_when_phase1_fails() {
    // Config with a structurally broken pipeline (no main phases)
    let mut config = default_config();
    config.pipelines.insert(
        "broken".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![],
        },
    );

    // InProgress item referencing a pipeline that doesn't exist in the config —
    // would trigger Phase 3 "unknown pipeline type" error if Phase 3 ran,
    // but Phase 1 should gate it
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    pg_item::set_pipeline_type(&mut item.0, Some("nonexistent"));
    pg_item::set_phase(&mut item.0, Some("build"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));

    let items = vec![item];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    // Phase 1 ran and found structural errors
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("no main phases")));
    // Phase 3 was skipped — no item validation errors
    assert!(!errors
        .iter()
        .any(|e| e.condition.contains("unknown pipeline type")
            || e.condition.contains("unknown phase")));
}

#[test]
fn preflight_phase3_runs_when_phase1_passes_but_phase2_fails() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join(".task-golem")).unwrap();

    // Structurally valid config with a workflow file that doesn't exist on disk
    let mut config = PhaseGolemConfig::default();
    config.pipelines.insert(
        "feature".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![PhaseConfig {
                workflows: vec!["workflows/nonexistent.md".to_string()],
                ..PhaseConfig::new("build", false)
            }],
        },
    );

    // InProgress item with an invalid phase — Phase 3 will report "unknown phase"
    // if it runs, proving the gate did not suppress it
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    pg_item::set_pipeline_type(&mut item.0, Some("feature"));
    pg_item::set_phase(&mut item.0, Some("nonexistent-phase"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));

    let items = vec![item];

    let result = run_preflight(&config, &items, root, root);

    let errors = result.unwrap_err();
    // Phase 2 ran and found missing workflow file
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("Workflow file not found")));
    // Phase 3 ran (because Phase 1 passed) and caught the invalid phase reference
    assert!(errors.iter().any(|e| e.condition.contains("unknown phase")));
}

#[test]
fn preflight_phase4_and_phase5_run_when_phase1_fails() {
    // Config with a structurally broken pipeline (no main phases)
    let mut config = default_config();
    config.pipelines.insert(
        "broken".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![],
        },
    );

    // Two items with the same ID to trigger Phase 4 duplicate detection
    let item_a = make_feature_item("WRK-DUP", ItemStatus::New);
    let item_b = make_feature_item("WRK-DUP", ItemStatus::New);

    let items = vec![item_a, item_b];

    let result = run_preflight(
        &config,
        &items,
        test_project_root(),
        test_project_root(),
    );

    let errors = result.unwrap_err();
    // Phase 1 ran and found structural errors
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("no main phases")));
    // Phase 4 ran despite Phase 1 failure
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("Duplicate item ID")));
}

// --- config_base vs project_root tests ---

#[test]
fn preflight_config_base_differs_from_project_root() {
    let dir = tempfile::tempdir().unwrap();
    let project_root = dir.path();
    std::fs::create_dir_all(project_root.join(".task-golem")).unwrap();

    // Create a subdirectory to serve as config_base
    let config_base = project_root.join("subdir");
    std::fs::create_dir_all(&config_base).unwrap();

    // Place the workflow file relative to config_base (not project_root)
    let workflow_path = "workflows/build.md";
    std::fs::create_dir_all(config_base.join("workflows")).unwrap();
    std::fs::write(config_base.join(workflow_path), "# Build workflow\n").unwrap();

    let mut config = PhaseGolemConfig::default();
    config.pipelines.insert(
        "test".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![PhaseConfig {
                workflows: vec![workflow_path.to_string()],
                ..PhaseConfig::new("build", false)
            }],
        },
    );

    let items: Vec<PgItem> = vec![];

    // The workflow file exists under config_base but NOT under project_root directly,
    // so this should pass because probe_workflows resolves relative to config_base.
    let result = run_preflight(&config, &items, project_root, &config_base);
    assert!(result.is_ok());

    // Verify it would fail if we passed project_root as config_base instead,
    // since the file does not exist at project_root/workflows/build.md.
    let result = run_preflight(&config, &items, project_root, project_root);
    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("Workflow file not found")));
}
