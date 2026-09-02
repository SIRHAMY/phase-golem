mod common;

use std::collections::HashMap;

use phase_golem::config::{PhaseConfig, PipelineConfig};
use phase_golem::prompt::{build_backlog_summary, build_prompt, build_triage_prompt, PromptParams};
use task_golem::model::status::Status;

#[test]
fn backlog_summary_renders_native_status_names() {
    let items = [
        common::make_pg_item(common::ID_1, Status::Todo),
        common::make_doing_pg_item(common::ID_2, "build"),
    ];
    let summary = build_backlog_summary(&items, common::ID_1).expect("backlog summary");
    assert!(summary.contains(common::ID_2));
    assert!(summary.contains("[doing]"));
}

#[test]
fn phase_prompt_contains_exact_task_identity_and_output_path() {
    let directory = tempfile::tempdir().expect("tempdir");
    let item = common::make_doing_pg_item(common::ID_1, "build");
    let phase = PhaseConfig::new("build", false);
    let result_path = directory.path().join("result.json");
    let prompt = build_prompt(&PromptParams {
        phase: "build",
        phase_config: &phase,
        item: &item,
        result_path: &result_path,
        change_folder: directory.path(),
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: directory.path(),
    });
    assert!(prompt.contains(common::ID_1));
    assert!(prompt.contains(&result_path.display().to_string()));
}

#[test]
fn triage_prompt_describes_diagnostic_blocking_without_parallel_gate_state() {
    let directory = tempfile::tempdir().expect("tempdir");
    let item = common::make_pg_item(common::ID_1, Status::Doing);
    let pipelines = HashMap::from([(
        "feature".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![PhaseConfig::new("build", false)],
        },
    )]);
    let prompt = build_triage_prompt(
        &item,
        &directory.path().join("triage.json"),
        &pipelines,
        None,
    );
    let removed_gate_field = ["requires", "human", "review"].join("_");
    assert!(!prompt.contains(&removed_gate_field));
    assert!(prompt.contains("reported as blocked with a diagnostic"));
}
