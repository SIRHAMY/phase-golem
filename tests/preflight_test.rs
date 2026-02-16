mod common;

use std::path::Path;

use orchestrate::config::{OrchestrateConfig, PhaseConfig, PipelineConfig, StalenessAction};
use orchestrate::preflight::{run_preflight, PreflightError};
use orchestrate::types::{BacklogItem, ItemStatus, PhasePool};

// --- Test helpers ---

fn make_feature_item(id: &str, status: ItemStatus) -> BacklogItem {
    let mut item = common::make_item(id, status);
    item.pipeline_type = Some("feature".to_string());
    item
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

fn default_config() -> OrchestrateConfig {
    let mut config = OrchestrateConfig::default();
    config
        .pipelines
        .insert("feature".to_string(), feature_pipeline_no_workflows());
    config
}

// --- Structural validation tests ---

#[test]
fn preflight_valid_config_passes() {
    let config = default_config();
    let backlog = common::make_backlog(vec![]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

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

    let backlog = common::make_backlog(vec![]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

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

    let backlog = common::make_backlog(vec![]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

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

    let backlog = common::make_backlog(vec![]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("cannot be destructive")));
}

#[test]
fn preflight_max_wip_zero_fails() {
    let mut config = default_config();
    config.execution.max_wip = 0;

    let backlog = common::make_backlog(vec![]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.condition.contains("max_wip")));
}

#[test]
fn preflight_max_concurrent_zero_fails() {
    let mut config = default_config();
    config.execution.max_concurrent = 0;

    let backlog = common::make_backlog(vec![]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

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

    let backlog = common::make_backlog(vec![]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

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

    let backlog = common::make_backlog(vec![]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    let errors = result.unwrap_err();
    let error = &errors[0];
    assert!(error.config_location.contains("orchestrate.toml"));
    assert!(!error.suggested_fix.is_empty());
}

#[test]
fn preflight_error_display_format() {
    let error = PreflightError {
        condition: "max_wip must be >= 1".to_string(),
        config_location: "orchestrate.toml → execution.max_wip".to_string(),
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

    // Create a minimal config with workflow files that we'll create on disk
    let mut config = OrchestrateConfig::default();
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

    let backlog = common::make_backlog(vec![]);
    let result = run_preflight(&config, &backlog, root);

    assert!(result.is_ok());
}

#[test]
fn preflight_missing_workflow_files_fails() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let mut config = OrchestrateConfig::default();
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

    let backlog = common::make_backlog(vec![]);
    let result = run_preflight(&config, &backlog, root);

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
    item.phase = Some("prd".to_string());
    item.phase_pool = Some(PhasePool::Main);
    item.pipeline_type = Some("feature".to_string());

    let backlog = common::make_backlog(vec![item]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    assert!(result.is_ok());
}

#[test]
fn preflight_invalid_pipeline_type_fails() {
    let config = default_config();
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    item.phase = Some("prd".to_string());
    item.phase_pool = Some(PhasePool::Main);
    item.pipeline_type = Some("nonexistent".to_string());

    let backlog = common::make_backlog(vec![item]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("unknown pipeline type")));
}

#[test]
fn preflight_invalid_phase_name_fails() {
    let config = default_config();
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    item.phase = Some("nonexistent-phase".to_string());
    item.phase_pool = Some(PhasePool::Main);
    item.pipeline_type = Some("feature".to_string());

    let backlog = common::make_backlog(vec![item]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.condition.contains("unknown phase")));
}

#[test]
fn preflight_mismatched_phase_pool_fails() {
    let config = default_config();
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    item.phase = Some("research".to_string()); // research is in pre_phases
    item.phase_pool = Some(PhasePool::Main); // But pool says main
    item.pipeline_type = Some("feature".to_string());

    let backlog = common::make_backlog(vec![item]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.condition.contains("phase_pool")));
}

#[test]
fn preflight_skips_new_and_done_items() {
    let config = default_config();
    // These items have invalid pipeline_type but should be skipped
    let mut new_item = make_feature_item("WRK-001", ItemStatus::New);
    new_item.pipeline_type = Some("nonexistent".to_string());

    let mut done_item = make_feature_item("WRK-002", ItemStatus::Done);
    done_item.pipeline_type = Some("nonexistent".to_string());

    let backlog = common::make_backlog(vec![new_item, done_item]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    assert!(result.is_ok());
}

#[test]
fn preflight_validates_scoping_items() {
    let config = default_config();
    let mut item = make_feature_item("WRK-001", ItemStatus::Scoping);
    item.phase = Some("research".to_string());
    item.phase_pool = Some(PhasePool::Pre);
    item.pipeline_type = Some("feature".to_string());

    let backlog = common::make_backlog(vec![item]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    assert!(result.is_ok());
}

#[test]
fn preflight_item_with_default_pipeline_type_passes() {
    let config = default_config();
    let mut item = make_feature_item("WRK-001", ItemStatus::InProgress);
    item.phase = Some("prd".to_string());
    item.phase_pool = Some(PhasePool::Main);
    item.pipeline_type = None; // Should default to "feature"

    let backlog = common::make_backlog(vec![item]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    assert!(result.is_ok());
}

// --- Duplicate ID validation tests ---

#[test]
fn preflight_empty_backlog_no_duplicate_errors() {
    let config = default_config();
    let backlog = common::make_backlog(vec![]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    assert!(result.is_ok());
}

#[test]
fn preflight_single_item_no_duplicate_errors() {
    let config = default_config();
    let item = make_feature_item("WRK-001", ItemStatus::New);

    let backlog = common::make_backlog(vec![item]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    assert!(result.is_ok());
}

#[test]
fn preflight_unique_ids_no_duplicate_errors() {
    let config = default_config();
    let item_a = make_feature_item("WRK-001", ItemStatus::New);
    let item_b = make_feature_item("WRK-002", ItemStatus::Ready);
    let item_c = make_feature_item("WRK-003", ItemStatus::Done);

    let backlog = common::make_backlog(vec![item_a, item_b, item_c]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    assert!(result.is_ok());
}

#[test]
fn preflight_duplicate_id_pair_fails() {
    let config = default_config();
    let item_a = make_feature_item("WRK-001", ItemStatus::New);
    let item_b = make_feature_item("WRK-001", ItemStatus::Done);

    let backlog = common::make_backlog(vec![item_a, item_b]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

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

    let backlog = common::make_backlog(vec![item_a, item_b, item_c, item_d]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

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

    let backlog = common::make_backlog(vec![item_a, item_b, item_c, item_d, item_e]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

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

    let backlog = common::make_backlog(vec![item_a, item_b]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    assert!(result.is_ok());
}

// --- Dependency graph validation: dangling references ---

#[test]
fn preflight_dangling_dependency_fails() {
    let config = default_config();
    let mut item = make_feature_item("WRK-001", ItemStatus::Ready);
    item.dependencies = vec!["WRK-999".to_string()];

    let backlog = common::make_backlog(vec![item]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("WRK-999") && e.condition.contains("does not exist")));
}

#[test]
fn preflight_multiple_dangling_references() {
    let config = default_config();
    let mut item_a = make_feature_item("WRK-001", ItemStatus::Ready);
    item_a.dependencies = vec!["WRK-888".to_string()];

    let mut item_b = make_feature_item("WRK-002", ItemStatus::Ready);
    item_b.dependencies = vec!["WRK-999".to_string()];

    let backlog = common::make_backlog(vec![item_a, item_b]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

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
    let mut item_b = make_feature_item("WRK-002", ItemStatus::Ready);
    item_b.dependencies = vec!["WRK-001".to_string()];

    let backlog = common::make_backlog(vec![item_a, item_b]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    assert!(result.is_ok());
}

// --- Dependency graph validation: cycle detection ---

#[test]
fn preflight_self_dependency_fails() {
    let config = default_config();
    let mut item = make_feature_item("WRK-001", ItemStatus::Ready);
    item.dependencies = vec!["WRK-001".to_string()];

    let backlog = common::make_backlog(vec![item]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

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
    let mut item_a = make_feature_item("WRK-001", ItemStatus::Ready);
    item_a.dependencies = vec!["WRK-002".to_string()];

    let mut item_b = make_feature_item("WRK-002", ItemStatus::Ready);
    item_b.dependencies = vec!["WRK-001".to_string()];

    let backlog = common::make_backlog(vec![item_a, item_b]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

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
    let mut item_a = make_feature_item("WRK-001", ItemStatus::Ready);
    item_a.dependencies = vec!["WRK-002".to_string()];

    let mut item_b = make_feature_item("WRK-002", ItemStatus::Ready);
    item_b.dependencies = vec!["WRK-003".to_string()];

    let mut item_c = make_feature_item("WRK-003", ItemStatus::Ready);
    item_c.dependencies = vec!["WRK-001".to_string()];

    let backlog = common::make_backlog(vec![item_a, item_b, item_c]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

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
    let mut item_a = make_feature_item("WRK-001", ItemStatus::Ready);
    item_a.dependencies = vec!["WRK-002".to_string()];

    let mut item_b = make_feature_item("WRK-002", ItemStatus::Ready);
    item_b.dependencies = vec!["WRK-001".to_string()];

    // Cycle 2: C <-> D
    let mut item_c = make_feature_item("WRK-003", ItemStatus::Ready);
    item_c.dependencies = vec!["WRK-004".to_string()];

    let mut item_d = make_feature_item("WRK-004", ItemStatus::Ready);
    item_d.dependencies = vec!["WRK-003".to_string()];

    let backlog = common::make_backlog(vec![item_a, item_b, item_c, item_d]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

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
    let mut item_a = make_feature_item("WRK-001", ItemStatus::Ready);
    item_a.dependencies = vec!["WRK-002".to_string()];

    let mut item_b = make_feature_item("WRK-002", ItemStatus::Blocked);
    item_b.blocked_from_status = Some(ItemStatus::Ready);
    item_b.dependencies = vec!["WRK-001".to_string()];

    let backlog = common::make_backlog(vec![item_a, item_b]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.condition.contains("Circular dependency")));
}

#[test]
fn preflight_done_items_excluded_from_cycle_detection() {
    let config = default_config();
    // A depends on B (Done), B depends on A — but B is Done so no cycle
    let mut item_a = make_feature_item("WRK-001", ItemStatus::Ready);
    item_a.dependencies = vec!["WRK-002".to_string()];

    let mut item_b = make_feature_item("WRK-002", ItemStatus::Done);
    item_b.dependencies = vec!["WRK-001".to_string()];

    let backlog = common::make_backlog(vec![item_a, item_b]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    assert!(result.is_ok());
}

#[test]
fn preflight_diamond_dag_no_false_positive() {
    let config = default_config();
    // Diamond: A→B, A→C, B→D, C→D (not a cycle)
    let mut item_a = make_feature_item("WRK-001", ItemStatus::Ready);
    item_a.dependencies = vec!["WRK-002".to_string(), "WRK-003".to_string()];

    let mut item_b = make_feature_item("WRK-002", ItemStatus::Ready);
    item_b.dependencies = vec!["WRK-004".to_string()];

    let mut item_c = make_feature_item("WRK-003", ItemStatus::Ready);
    item_c.dependencies = vec!["WRK-004".to_string()];

    let item_d = make_feature_item("WRK-004", ItemStatus::Ready);

    let backlog = common::make_backlog(vec![item_a, item_b, item_c, item_d]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    assert!(result.is_ok());
}

#[test]
fn preflight_transitive_chain_no_cycle() {
    let config = default_config();
    // C→B→A (valid DAG chain)
    let item_a = make_feature_item("WRK-001", ItemStatus::Ready);

    let mut item_b = make_feature_item("WRK-002", ItemStatus::Ready);
    item_b.dependencies = vec!["WRK-001".to_string()];

    let mut item_c = make_feature_item("WRK-003", ItemStatus::Ready);
    item_c.dependencies = vec!["WRK-002".to_string()];

    let backlog = common::make_backlog(vec![item_a, item_b, item_c]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    assert!(result.is_ok());
}

#[test]
fn preflight_no_dependencies_passes() {
    let config = default_config();
    let item_a = make_feature_item("WRK-001", ItemStatus::Ready);
    let item_b = make_feature_item("WRK-002", ItemStatus::Ready);

    let backlog = common::make_backlog(vec![item_a, item_b]);

    let result = run_preflight(&config, &backlog, Path::new("/tmp/test"));

    assert!(result.is_ok());
}
