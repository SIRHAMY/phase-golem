mod common;

use phase_golem::config::{GuardrailsConfig, PhaseConfig, PipelineConfig};
use phase_golem::executor::{passes_guardrails, resolve_transition, validate_result_identity};
use phase_golem::pg_item::{self, X_PG_TEMPLATE_NODE_KEY};
use phase_golem::types::{
    DimensionLevel, ItemUpdate, PhasePool, PhaseResult, ResultCode, SizeLevel,
};
use task_golem::model::status::Status;

fn pipeline() -> PipelineConfig {
    PipelineConfig {
        pre_phases: vec![PhaseConfig::new("research", false)],
        phases: vec![
            PhaseConfig::new("build", true),
            PhaseConfig::new("review", false),
        ],
    }
}

fn result(item_id: &str, phase: &str, code: ResultCode) -> PhaseResult {
    PhaseResult {
        item_id: item_id.to_string(),
        phase: phase.to_string(),
        result: code,
        summary: "completed".to_string(),
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

#[test]
fn materialized_phase_completion_finishes_its_independent_task() {
    // Arrange
    let mut item = common::make_doing_pg_item(common::ID_1, "build");
    item.0.extensions.insert(
        X_PG_TEMPLATE_NODE_KEY.to_string(),
        serde_json::json!("build"),
    );

    // Act
    let updates = resolve_transition(
        &item,
        &result(common::ID_1, "build", ResultCode::PhaseComplete),
        &pipeline(),
        &GuardrailsConfig::default(),
    );

    // Assert
    assert_eq!(updates, vec![ItemUpdate::TransitionStatus(Status::Done)]);
}

#[test]
fn crud_item_continues_existing_phase_progression_while_doing() {
    // Arrange
    let item = common::make_doing_pg_item(common::ID_1, "build");

    // Act
    let updates = resolve_transition(
        &item,
        &result(common::ID_1, "build", ResultCode::PhaseComplete),
        &pipeline(),
        &GuardrailsConfig::default(),
    );

    // Assert
    assert_eq!(updates, vec![ItemUpdate::SetPhase("review".to_string())]);
}

#[test]
fn final_pre_phase_routes_to_first_main_phase_without_parallel_status() {
    // Arrange
    let mut item = common::make_doing_pg_item(common::ID_1, "research");
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Pre));

    // Act
    let updates = resolve_transition(
        &item,
        &result(common::ID_1, "research", ResultCode::PhaseComplete),
        &pipeline(),
        &GuardrailsConfig::default(),
    );

    // Assert
    assert_eq!(
        updates,
        vec![
            ItemUpdate::SetPhase("build".to_string()),
            ItemUpdate::SetPhasePool(PhasePool::Main),
        ]
    );
    assert_eq!(item.status(), Status::Doing);
}

#[test]
fn failed_and_blocked_results_produce_diagnostic_blocks() {
    let failed = resolve_transition(
        &common::make_doing_pg_item(common::ID_1, "build"),
        &result(common::ID_1, "build", ResultCode::Failed),
        &pipeline(),
        &GuardrailsConfig::default(),
    );
    let mut blocked_result = result(common::ID_1, "build", ResultCode::Blocked);
    blocked_result.context = Some("approval service unavailable".to_string());
    let blocked = resolve_transition(
        &common::make_doing_pg_item(common::ID_1, "build"),
        &blocked_result,
        &pipeline(),
        &GuardrailsConfig::default(),
    );
    assert!(matches!(&failed[0], ItemUpdate::SetBlocked(reason) if reason.contains("failed")));
    assert_eq!(
        blocked,
        vec![ItemUpdate::SetBlocked(
            "approval service unavailable".to_string()
        )]
    );
}

#[test]
fn guardrails_still_use_pg_assessment_metadata() {
    let mut item = common::make_doing_pg_item(common::ID_1, "build");
    pg_item::set_size(&mut item.0, Some(&SizeLevel::Large));
    pg_item::set_risk(&mut item.0, Some(&DimensionLevel::High));
    assert!(!passes_guardrails(&item, &GuardrailsConfig::default()));
}

#[test]
fn result_identity_requires_exact_uuid_and_phase() {
    let result = result(common::ID_1, "build", ResultCode::PhaseComplete);
    assert!(validate_result_identity(&result, common::ID_1, "build").is_ok());
    assert!(validate_result_identity(&result, common::ID_2, "build").is_err());
    assert!(validate_result_identity(&result, common::ID_1, "review").is_err());
}
