mod common;

use std::collections::{HashMap, HashSet};

use phase_golem::config::{PhaseConfig, PhaseGolemConfig, PipelineConfig};
use phase_golem::coordinator::WorkSnapshot;
use phase_golem::preflight::run_preflight;
use task_golem::model::deps::evaluate_dependencies;
use task_golem::model::status::Status;

fn config() -> PhaseGolemConfig {
    PhaseGolemConfig {
        pipelines: HashMap::from([(
            "feature".to_string(),
            PipelineConfig {
                pre_phases: vec![],
                phases: vec![PhaseConfig::new("build", false)],
            },
        )]),
        ..PhaseGolemConfig::default()
    }
}

fn snapshot(items: Vec<phase_golem::pg_item::PgItem>) -> WorkSnapshot {
    let active = items.iter().map(|item| item.0.clone()).collect::<Vec<_>>();
    WorkSnapshot {
        items,
        archived_done_ids: HashSet::new(),
        dependency_evaluation: evaluate_dependencies(&active, &[]),
    }
}

#[test]
fn valid_native_snapshot_passes_preflight() {
    // Arrange
    let directory = tempfile::tempdir().expect("tempdir");
    common::setup_task_golem_store(directory.path());
    let item = common::make_pg_item(common::ID_1, Status::Todo);

    // Act
    let result = run_preflight(
        &config(),
        &snapshot(vec![item]),
        directory.path(),
        directory.path(),
    );

    // Assert
    assert!(result.is_ok(), "preflight should pass: {result:?}");
}

#[test]
fn tg_integrity_issue_is_reported_without_local_readiness_rules() {
    // Arrange
    let directory = tempfile::tempdir().expect("tempdir");
    common::setup_task_golem_store(directory.path());
    let mut item = common::make_pg_item(common::ID_1, Status::Todo);
    item.0.dependencies = vec![common::ID_2.to_string()];

    // Act
    let errors = run_preflight(
        &config(),
        &snapshot(vec![item]),
        directory.path(),
        directory.path(),
    )
    .expect_err("missing dependency must fail preflight");

    // Assert
    assert!(errors.iter().any(|error| {
        error.condition.contains(common::ID_1) && error.condition.contains(common::ID_2)
    }));
}

#[test]
fn claimed_item_phase_must_map_to_configured_phase() {
    // Arrange
    let directory = tempfile::tempdir().expect("tempdir");
    common::setup_task_golem_store(directory.path());
    let item = common::make_doing_pg_item(common::ID_1, "publish");

    // Act
    let errors = run_preflight(
        &config(),
        &snapshot(vec![item]),
        directory.path(),
        directory.path(),
    )
    .expect_err("unknown phase must fail");

    // Assert
    assert!(errors
        .iter()
        .any(|error| error.condition.contains("unknown phase")));
}
