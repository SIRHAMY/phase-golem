use phase_golem::types::{
    parse_item_status, DimensionLevel, FollowUp, ItemUpdate, PhasePool, PhaseResult, ResultCode,
    SchedulerAction, SizeLevel, StructuredDescription, UpdatedAssessments,
};
use task_golem::model::status::Status;

#[test]
fn native_status_parser_accepts_only_tg_statuses() {
    for (input, expected) in [
        ("todo", Status::Todo),
        ("doing", Status::Doing),
        ("blocked", Status::Blocked),
        ("done", Status::Done),
    ] {
        assert_eq!(parse_item_status(input), Ok(expected));
    }
    assert!(parse_item_status("ready").is_err());
}

#[test]
fn lifecycle_updates_carry_native_status_and_diagnostics() {
    assert_eq!(
        ItemUpdate::TransitionStatus(Status::Done),
        ItemUpdate::TransitionStatus(Status::Done)
    );
    assert_eq!(
        ItemUpdate::SetBlocked("executor failed".to_string()),
        ItemUpdate::SetBlocked("executor failed".to_string())
    );
}

#[test]
fn scheduler_actions_distinguish_claim_gate_and_block() {
    let actions = [
        SchedulerAction::Claim("a".to_string()),
        SchedulerAction::HumanGate("b".to_string()),
        SchedulerAction::Block {
            item_id: "c".to_string(),
            reason: "no phase".to_string(),
        },
    ];
    assert!(matches!(actions[0], SchedulerAction::Claim(_)));
    assert!(matches!(actions[1], SchedulerAction::HumanGate(_)));
    assert!(matches!(actions[2], SchedulerAction::Block { .. }));
}

#[test]
fn follow_up_accepts_plain_string_and_structured_shape() {
    let plain: FollowUp = serde_json::from_str("\"Add coverage\"").expect("plain follow-up");
    let structured: FollowUp = serde_json::from_str(
        r#"{"title":"Add docs","suggested_size":"small","context":"public API"}"#,
    )
    .expect("structured follow-up");
    assert_eq!(plain.title, "Add coverage");
    assert_eq!(structured.suggested_size, Some(SizeLevel::Small));
}

#[test]
fn structured_description_reports_empty_content() {
    assert!(StructuredDescription::default().is_empty());
    assert!(!StructuredDescription {
        context: "context".to_string(),
        ..StructuredDescription::default()
    }
    .is_empty());
}

#[test]
fn phase_result_serialization_round_trips_agent_contract() {
    let result = PhaseResult {
        item_id: "018f2b1c-4d5e-7abc-8123-456789abcdef".to_string(),
        phase: "build".to_string(),
        result: ResultCode::PhaseComplete,
        summary: "Implemented native lifecycle".to_string(),
        context: Some("All checks passed".to_string()),
        updated_assessments: Some(UpdatedAssessments {
            size: Some(SizeLevel::Medium),
            complexity: Some(DimensionLevel::High),
            risk: Some(DimensionLevel::Low),
            impact: Some(DimensionLevel::High),
        }),
        follow_ups: vec![FollowUp {
            title: "Add operator documentation".to_string(),
            context: Some("Document claim behavior".to_string()),
            suggested_size: Some(SizeLevel::Small),
            suggested_risk: Some(DimensionLevel::Low),
        }],
        based_on_commit: Some("abc123".to_string()),
        pipeline_type: Some("feature".to_string()),
        commit_summary: Some("Use TG-native lifecycle".to_string()),
        duplicates: vec!["018f2b1c-4d5e-7abc-9234-56789abcdef0".to_string()],
        description: Some(StructuredDescription {
            context: "Legacy state diverged".to_string(),
            problem: "Two lifecycle authorities".to_string(),
            solution: "Use TG status".to_string(),
            impact: "Consistent scheduling".to_string(),
            sizing_rationale: "Cross-cutting migration".to_string(),
        }),
    };

    let json = serde_json::to_string(&result).expect("serialize phase result");
    let decoded = serde_json::from_str::<PhaseResult>(&json).expect("deserialize phase result");

    assert!(json.contains(r#""result":"phase_complete""#));
    assert_eq!(decoded, result);
}

#[test]
fn item_update_serialization_round_trips_all_contract_variants() {
    let updates = vec![
        ItemUpdate::TransitionStatus(Status::Doing),
        ItemUpdate::SetPhase("build".to_string()),
        ItemUpdate::SetPhasePool(PhasePool::Main),
        ItemUpdate::ClearPhase,
        ItemUpdate::SetBlocked("executor failed".to_string()),
        ItemUpdate::Unblock,
        ItemUpdate::UpdateAssessments(UpdatedAssessments {
            size: Some(SizeLevel::Large),
            complexity: Some(DimensionLevel::Medium),
            risk: Some(DimensionLevel::High),
            impact: Some(DimensionLevel::Low),
        }),
        ItemUpdate::SetPipelineType("feature".to_string()),
        ItemUpdate::SetLastPhaseCommit("abc123".to_string()),
        ItemUpdate::SetDescription(StructuredDescription {
            context: "Context".to_string(),
            ..StructuredDescription::default()
        }),
    ];

    for update in updates {
        let json = serde_json::to_string(&update).expect("serialize item update");
        let decoded = serde_json::from_str::<ItemUpdate>(&json).expect("deserialize item update");
        assert_eq!(decoded, update, "item update JSON: {json}");
    }
}
