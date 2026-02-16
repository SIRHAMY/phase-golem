use orchestrate::types::*;

// --- Default impl verification ---

#[test]
fn test_itemstatus_default() {
    assert_eq!(ItemStatus::default(), ItemStatus::New);
}

#[test]
fn test_backlogitem_default() {
    let item = BacklogItem::default();
    assert_eq!(item.id, "");
    assert_eq!(item.title, "");
    assert_eq!(item.status, ItemStatus::New);
    assert_eq!(item.phase, None);
    assert_eq!(item.size, None);
    assert_eq!(item.complexity, None);
    assert_eq!(item.risk, None);
    assert_eq!(item.impact, None);
    assert_eq!(item.requires_human_review, false);
    assert_eq!(item.origin, None);
    assert_eq!(item.blocked_from_status, None);
    assert_eq!(item.blocked_reason, None);
    assert_eq!(item.blocked_type, None);
    assert_eq!(item.unblock_context, None);
    assert_eq!(item.tags, Vec::<String>::new());
    assert_eq!(item.dependencies, Vec::<String>::new());
    assert_eq!(item.created, "");
    assert_eq!(item.updated, "");
    assert_eq!(item.pipeline_type, None);
    assert_eq!(item.description, None);
    assert_eq!(item.phase_pool, None);
    assert_eq!(item.last_phase_commit, None);
}

// --- YAML serialization round-trips ---

#[test]
fn yaml_round_trip_item_status_all_variants() {
    let variants = vec![
        ItemStatus::New,
        ItemStatus::Scoping,
        ItemStatus::Ready,
        ItemStatus::InProgress,
        ItemStatus::Done,
        ItemStatus::Blocked,
    ];
    for status in variants {
        let yaml = serde_yaml_ng::to_string(&status).unwrap();
        let deserialized: ItemStatus = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(status, deserialized);
    }
}

#[test]
fn yaml_round_trip_result_code_all_variants() {
    let variants = vec![
        ResultCode::SubphaseComplete,
        ResultCode::PhaseComplete,
        ResultCode::Failed,
        ResultCode::Blocked,
    ];
    for code in variants {
        let yaml = serde_yaml_ng::to_string(&code).unwrap();
        let deserialized: ResultCode = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(code, deserialized);
    }
}

#[test]
fn yaml_round_trip_block_type_all_variants() {
    let variants = vec![BlockType::Clarification, BlockType::Decision];
    for bt in variants {
        let yaml = serde_yaml_ng::to_string(&bt).unwrap();
        let deserialized: BlockType = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(bt, deserialized);
    }
}

#[test]
fn yaml_round_trip_size_level_all_variants() {
    let variants = vec![SizeLevel::Small, SizeLevel::Medium, SizeLevel::Large];
    for sl in variants {
        let yaml = serde_yaml_ng::to_string(&sl).unwrap();
        let deserialized: SizeLevel = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(sl, deserialized);
    }
}

#[test]
fn yaml_round_trip_dimension_level_all_variants() {
    let variants = vec![
        DimensionLevel::Low,
        DimensionLevel::Medium,
        DimensionLevel::High,
    ];
    for dl in variants {
        let yaml = serde_yaml_ng::to_string(&dl).unwrap();
        let deserialized: DimensionLevel = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(dl, deserialized);
    }
}

#[test]
fn yaml_round_trip_backlog_item_full() {
    let item = BacklogItem {
        id: "WRK-001".to_string(),
        title: "Add dark mode".to_string(),
        status: ItemStatus::InProgress,
        phase: Some("build".to_string()),
        size: Some(SizeLevel::Medium),
        complexity: Some(DimensionLevel::Medium),
        risk: Some(DimensionLevel::Low),
        impact: Some(DimensionLevel::High),
        requires_human_review: true,
        origin: Some("WRK-000/build".to_string()),
        blocked_from_status: None,
        blocked_reason: None,
        blocked_type: None,
        unblock_context: None,
        tags: vec!["ui".to_string(), "feature".to_string()],
        dependencies: vec!["WRK-002".to_string()],
        created: "2026-02-10T10:00:00Z".to_string(),
        updated: "2026-02-11T14:30:00Z".to_string(),
        pipeline_type: None,
        description: None,
        phase_pool: None,
        last_phase_commit: None,
    };

    let yaml = serde_yaml_ng::to_string(&item).unwrap();
    let deserialized: BacklogItem = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(item, deserialized);
}

#[test]
fn yaml_round_trip_backlog_item_minimal() {
    let item = BacklogItem {
        id: "WRK-002".to_string(),
        title: "Fix typo".to_string(),
        created: "2026-02-10T10:00:00Z".to_string(),
        updated: "2026-02-10T10:00:00Z".to_string(),
        ..Default::default()
    };

    let yaml = serde_yaml_ng::to_string(&item).unwrap();
    let deserialized: BacklogItem = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(item, deserialized);
}

#[test]
fn yaml_round_trip_backlog_item_blocked() {
    let item = BacklogItem {
        id: "WRK-003".to_string(),
        title: "Blocked item".to_string(),
        status: ItemStatus::Blocked,
        phase: Some("design".to_string()),
        size: Some(SizeLevel::Large),
        complexity: Some(DimensionLevel::High),
        risk: Some(DimensionLevel::High),
        impact: Some(DimensionLevel::Medium),
        requires_human_review: true,
        origin: None,
        blocked_from_status: Some(ItemStatus::InProgress),
        blocked_reason: Some("Need decision on API design".to_string()),
        blocked_type: Some(BlockType::Decision),
        unblock_context: None,
        tags: vec![],
        dependencies: vec![],
        created: "2026-02-10T10:00:00Z".to_string(),
        updated: "2026-02-11T14:30:00Z".to_string(),
        pipeline_type: None,
        description: None,
        phase_pool: None,
        last_phase_commit: None,
    };

    let yaml = serde_yaml_ng::to_string(&item).unwrap();
    let deserialized: BacklogItem = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(item, deserialized);
}

#[test]
fn yaml_round_trip_backlog_file() {
    let backlog = BacklogFile {
        schema_version: 3,
        items: vec![
            BacklogItem {
                id: "WRK-001".to_string(),
                title: "First item".to_string(),
                status: ItemStatus::Ready,
                phase: Some("prd".to_string()),
                size: Some(SizeLevel::Small),
                complexity: Some(DimensionLevel::Low),
                risk: Some(DimensionLevel::Low),
                impact: Some(DimensionLevel::Medium),
                created: "2026-02-10T10:00:00Z".to_string(),
                updated: "2026-02-10T10:00:00Z".to_string(),
                ..Default::default()
            },
            BacklogItem {
                id: "WRK-002".to_string(),
                title: "Second item".to_string(),
                created: "2026-02-11T10:00:00Z".to_string(),
                updated: "2026-02-11T10:00:00Z".to_string(),
                ..Default::default()
            },
        ],
        next_item_id: 0,
    };

    let yaml = serde_yaml_ng::to_string(&backlog).unwrap();
    let deserialized: BacklogFile = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(backlog, deserialized);
}

// --- JSON serialization round-trips ---

#[test]
fn json_round_trip_phase_result_full() {
    let result = PhaseResult {
        item_id: "WRK-001".to_string(),
        phase: "build".to_string(),
        result: ResultCode::PhaseComplete,
        summary: "Built all components successfully".to_string(),
        context: Some("All tests pass".to_string()),
        updated_assessments: Some(UpdatedAssessments {
            size: Some(SizeLevel::Medium),
            complexity: Some(DimensionLevel::Medium),
            risk: None,
            impact: Some(DimensionLevel::High),
        }),
        follow_ups: vec![
            FollowUp {
                title: "Add integration tests".to_string(),
                context: Some("Unit tests pass but integration coverage is low".to_string()),
                suggested_size: Some(SizeLevel::Small),
                suggested_risk: Some(DimensionLevel::Low),
            },
            FollowUp {
                title: "Refactor error handling".to_string(),
                context: None,
                suggested_size: None,
                suggested_risk: None,
            },
        ],
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    };

    let json = serde_json::to_string_pretty(&result).unwrap();
    let deserialized: PhaseResult = serde_json::from_str(&json).unwrap();
    assert_eq!(result, deserialized);
}

#[test]
fn json_round_trip_phase_result_minimal() {
    let result = PhaseResult {
        item_id: "WRK-002".to_string(),
        phase: "prd".to_string(),
        result: ResultCode::Failed,
        summary: "Could not generate PRD".to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: vec![],
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    };

    let json = serde_json::to_string(&result).unwrap();
    let deserialized: PhaseResult = serde_json::from_str(&json).unwrap();
    assert_eq!(result, deserialized);
}

#[test]
fn json_round_trip_phase_result_blocked() {
    let result = PhaseResult {
        item_id: "WRK-003".to_string(),
        phase: "design".to_string(),
        result: ResultCode::Blocked,
        summary: "Need clarification on auth approach".to_string(),
        context: Some("OAuth vs JWT decision needed".to_string()),
        updated_assessments: None,
        follow_ups: vec![],
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    };

    let json = serde_json::to_string(&result).unwrap();
    let deserialized: PhaseResult = serde_json::from_str(&json).unwrap();
    assert_eq!(result, deserialized);
}

#[test]
fn json_round_trip_phase_result_subphase_complete() {
    let result = PhaseResult {
        item_id: "WRK-001".to_string(),
        phase: "build".to_string(),
        result: ResultCode::SubphaseComplete,
        summary: "Phase 1 of 3 complete".to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: vec![],
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    };

    let json = serde_json::to_string(&result).unwrap();
    let deserialized: PhaseResult = serde_json::from_str(&json).unwrap();
    assert_eq!(result, deserialized);
}

// --- Status transition validation ---

#[test]
fn valid_transitions_pre_workflow() {
    assert!(ItemStatus::New.is_valid_transition(&ItemStatus::Scoping));
    assert!(ItemStatus::Scoping.is_valid_transition(&ItemStatus::Ready));
}

#[test]
fn valid_transitions_workflow() {
    assert!(ItemStatus::Ready.is_valid_transition(&ItemStatus::InProgress));
    assert!(ItemStatus::InProgress.is_valid_transition(&ItemStatus::Done));
}

#[test]
fn valid_transitions_to_blocked() {
    assert!(ItemStatus::New.is_valid_transition(&ItemStatus::Blocked));
    assert!(ItemStatus::Scoping.is_valid_transition(&ItemStatus::Blocked));
    assert!(ItemStatus::Ready.is_valid_transition(&ItemStatus::Blocked));
    assert!(ItemStatus::InProgress.is_valid_transition(&ItemStatus::Blocked));
}

#[test]
fn valid_transitions_unblock() {
    assert!(ItemStatus::Blocked.is_valid_transition(&ItemStatus::New));
    assert!(ItemStatus::Blocked.is_valid_transition(&ItemStatus::Scoping));
    assert!(ItemStatus::Blocked.is_valid_transition(&ItemStatus::Ready));
    assert!(ItemStatus::Blocked.is_valid_transition(&ItemStatus::InProgress));
}

#[test]
fn invalid_transitions() {
    // Can't skip pre-workflow stages
    assert!(!ItemStatus::New.is_valid_transition(&ItemStatus::Ready));
    assert!(!ItemStatus::New.is_valid_transition(&ItemStatus::InProgress));
    assert!(!ItemStatus::New.is_valid_transition(&ItemStatus::Done));

    // Can't go backward
    assert!(!ItemStatus::Scoping.is_valid_transition(&ItemStatus::New));
    assert!(!ItemStatus::Ready.is_valid_transition(&ItemStatus::Scoping));
    assert!(!ItemStatus::InProgress.is_valid_transition(&ItemStatus::Ready));
    assert!(!ItemStatus::Done.is_valid_transition(&ItemStatus::InProgress));

    // Done is terminal (except for blocking, which is already excluded)
    assert!(!ItemStatus::Done.is_valid_transition(&ItemStatus::New));
    assert!(!ItemStatus::Done.is_valid_transition(&ItemStatus::Blocked));

    // Can't go from blocked to done
    assert!(!ItemStatus::Blocked.is_valid_transition(&ItemStatus::Done));

    // Identity transitions are invalid
    assert!(!ItemStatus::New.is_valid_transition(&ItemStatus::New));
    assert!(!ItemStatus::Blocked.is_valid_transition(&ItemStatus::Blocked));
}

// --- Serde rename verification ---

#[test]
fn yaml_enum_uses_snake_case() {
    let yaml = serde_yaml_ng::to_string(&ItemStatus::InProgress).unwrap();
    assert_eq!(yaml.trim(), "in_progress");

    let yaml = serde_yaml_ng::to_string(&ResultCode::SubphaseComplete).unwrap();
    assert_eq!(yaml.trim(), "subphase_complete");

    let yaml = serde_yaml_ng::to_string(&ResultCode::PhaseComplete).unwrap();
    assert_eq!(yaml.trim(), "phase_complete");
}

#[test]
fn optional_fields_omitted_when_none() {
    let item = BacklogItem {
        id: "WRK-001".to_string(),
        title: "Test".to_string(),
        created: "2026-02-10T10:00:00Z".to_string(),
        updated: "2026-02-10T10:00:00Z".to_string(),
        ..Default::default()
    };

    let yaml = serde_yaml_ng::to_string(&item).unwrap();
    assert!(!yaml.contains("phase:"));
    assert!(!yaml.contains("size:"));
    assert!(!yaml.contains("origin:"));
    assert!(!yaml.contains("blocked_from_status:"));
    assert!(!yaml.contains("tags:"));
    assert!(!yaml.contains("dependencies:"));
}

#[test]
fn deserialize_backlog_item_with_missing_optional_fields() {
    let yaml = r#"
id: WRK-001
title: Minimal item
status: new
created: "2026-02-10T10:00:00Z"
updated: "2026-02-10T10:00:00Z"
"#;

    let item: BacklogItem = serde_yaml_ng::from_str(yaml).unwrap();
    assert_eq!(item.id, "WRK-001");
    assert_eq!(item.status, ItemStatus::New);
    assert_eq!(item.phase, None);
    assert_eq!(item.size, None);
    assert!(!item.requires_human_review);
    assert!(item.tags.is_empty());
    assert!(item.dependencies.is_empty());
    assert_eq!(item.pipeline_type, None);
    assert_eq!(item.description, None);
    assert_eq!(item.phase_pool, None);
    assert_eq!(item.last_phase_commit, None);
}

// --- New v2 type serialization round-trips ---

#[test]
fn yaml_round_trip_phase_pool_all_variants() {
    let variants = vec![PhasePool::Pre, PhasePool::Main];
    for pool in variants {
        let yaml = serde_yaml_ng::to_string(&pool).unwrap();
        let deserialized: PhasePool = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(pool, deserialized);
    }
}

#[test]
fn yaml_round_trip_item_update_variants() {
    let variants: Vec<ItemUpdate> = vec![
        ItemUpdate::TransitionStatus(ItemStatus::Ready),
        ItemUpdate::SetPhase("build".to_string()),
        ItemUpdate::ClearPhase,
        ItemUpdate::SetBlocked("guardrails: risk too high".to_string()),
        ItemUpdate::Unblock,
        ItemUpdate::UpdateAssessments(UpdatedAssessments {
            size: Some(SizeLevel::Large),
            complexity: None,
            risk: Some(DimensionLevel::High),
            impact: None,
        }),
        ItemUpdate::SetPipelineType("feature".to_string()),
        ItemUpdate::SetLastPhaseCommit("abc123def456".to_string()),
        ItemUpdate::SetDescription(StructuredDescription {
            context: "Add dark mode toggle".to_string(),
            problem: String::new(),
            solution: String::new(),
            impact: String::new(),
            sizing_rationale: String::new(),
        }),
    ];
    for update in variants {
        let yaml = serde_yaml_ng::to_string(&update).unwrap();
        let deserialized: ItemUpdate = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(update, deserialized);
    }
}

#[test]
fn yaml_round_trip_scheduler_action_variants() {
    let variants = vec![
        SchedulerAction::Triage("WRK-001".to_string()),
        SchedulerAction::Promote("WRK-002".to_string()),
        SchedulerAction::RunPhase {
            item_id: "WRK-003".to_string(),
            phase: "build".to_string(),
            phase_pool: PhasePool::Main,
            is_destructive: true,
        },
    ];
    for action in variants {
        let yaml = serde_yaml_ng::to_string(&action).unwrap();
        let deserialized: SchedulerAction = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(action, deserialized);
    }
}

#[test]
fn yaml_round_trip_phase_execution_result_variants() {
    let variants = vec![
        PhaseExecutionResult::Success(PhaseResult {
            item_id: "WRK-001".to_string(),
            phase: "build".to_string(),
            result: ResultCode::PhaseComplete,
            summary: "Build complete".to_string(),
            context: None,
            updated_assessments: None,
            follow_ups: vec![],
            based_on_commit: None,
            pipeline_type: None,
            commit_summary: None,
            duplicates: Vec::new(),
        }),
        PhaseExecutionResult::SubphaseComplete(PhaseResult {
            item_id: "WRK-001".to_string(),
            phase: "build".to_string(),
            result: ResultCode::SubphaseComplete,
            summary: "Phase 1 done".to_string(),
            context: None,
            updated_assessments: None,
            follow_ups: vec![],
            based_on_commit: None,
            pipeline_type: None,
            commit_summary: None,
            duplicates: Vec::new(),
        }),
        PhaseExecutionResult::Failed("Something went wrong".to_string()),
        PhaseExecutionResult::Blocked("Needs human review".to_string()),
        PhaseExecutionResult::Cancelled,
    ];
    for result in variants {
        let yaml = serde_yaml_ng::to_string(&result).unwrap();
        let deserialized: PhaseExecutionResult = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(result, deserialized);
    }
}

#[test]
fn yaml_round_trip_backlog_file_with_all_fields() {
    let backlog = BacklogFile {
        items: vec![BacklogItem {
            id: "WRK-001".to_string(),
            title: "Test item".to_string(),
            status: ItemStatus::InProgress,
            phase: Some("build".to_string()),
            size: Some(SizeLevel::Medium),
            created: "2026-02-10T10:00:00Z".to_string(),
            updated: "2026-02-10T10:00:00Z".to_string(),
            pipeline_type: Some("feature".to_string()),
            description: Some(StructuredDescription {
                context: "Add dark mode".to_string(),
                problem: String::new(),
                solution: String::new(),
                impact: String::new(),
                sizing_rationale: String::new(),
            }),
            phase_pool: Some(PhasePool::Main),
            last_phase_commit: Some("abc123".to_string()),
            ..Default::default()
        }],
        schema_version: 3,
        next_item_id: 5,
    };

    let yaml = serde_yaml_ng::to_string(&backlog).unwrap();
    let deserialized: BacklogFile = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(backlog, deserialized);
}

#[test]
fn yaml_round_trip_backlog_item_with_new_fields() {
    let item = BacklogItem {
        id: "WRK-005".to_string(),
        title: "Item with v2 fields".to_string(),
        status: ItemStatus::InProgress,
        phase: Some("build".to_string()),
        size: Some(SizeLevel::Medium),
        complexity: Some(DimensionLevel::Medium),
        risk: Some(DimensionLevel::Low),
        impact: Some(DimensionLevel::High),
        requires_human_review: false,
        origin: None,
        blocked_from_status: None,
        blocked_reason: None,
        blocked_type: None,
        unblock_context: None,
        tags: vec![],
        dependencies: vec![],
        created: "2026-02-10T10:00:00Z".to_string(),
        updated: "2026-02-11T14:30:00Z".to_string(),
        pipeline_type: Some("feature".to_string()),
        description: Some(StructuredDescription {
            context: "Implement user auth flow".to_string(),
            problem: String::new(),
            solution: String::new(),
            impact: String::new(),
            sizing_rationale: String::new(),
        }),
        phase_pool: Some(PhasePool::Main),
        last_phase_commit: Some("deadbeef12345678".to_string()),
    };

    let yaml = serde_yaml_ng::to_string(&item).unwrap();
    let deserialized: BacklogItem = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(item, deserialized);
}

#[test]
fn yaml_round_trip_backlog_item_without_new_optional_fields() {
    let item = BacklogItem {
        id: "WRK-006".to_string(),
        title: "Item without v2 fields".to_string(),
        created: "2026-02-10T10:00:00Z".to_string(),
        updated: "2026-02-10T10:00:00Z".to_string(),
        ..Default::default()
    };

    let yaml = serde_yaml_ng::to_string(&item).unwrap();
    let deserialized: BacklogItem = serde_yaml_ng::from_str(&yaml).unwrap();
    assert_eq!(item, deserialized);

    // New optional fields should not appear in serialized output
    assert!(!yaml.contains("pipeline_type:"));
    assert!(!yaml.contains("description:"));
    assert!(!yaml.contains("phase_pool:"));
    assert!(!yaml.contains("last_phase_commit:"));
}

#[test]
fn json_round_trip_phase_result_with_new_fields() {
    let result = PhaseResult {
        item_id: "WRK-001".to_string(),
        phase: "build".to_string(),
        result: ResultCode::PhaseComplete,
        summary: "Build complete".to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: vec![],
        based_on_commit: Some("abc123def456789012345678901234567890abcd".to_string()),
        pipeline_type: Some("feature".to_string()),
        commit_summary: None,
        duplicates: Vec::new(),
    };

    let json = serde_json::to_string_pretty(&result).unwrap();
    let deserialized: PhaseResult = serde_json::from_str(&json).unwrap();
    assert_eq!(result, deserialized);
}

#[test]
fn json_round_trip_phase_result_without_new_fields() {
    let result = PhaseResult {
        item_id: "WRK-002".to_string(),
        phase: "prd".to_string(),
        result: ResultCode::PhaseComplete,
        summary: "PRD complete".to_string(),
        context: None,
        updated_assessments: None,
        follow_ups: vec![],
        based_on_commit: None,
        pipeline_type: None,
        commit_summary: None,
        duplicates: Vec::new(),
    };

    let json = serde_json::to_string(&result).unwrap();
    let deserialized: PhaseResult = serde_json::from_str(&json).unwrap();
    assert_eq!(result, deserialized);

    // New optional fields should not appear when None
    assert!(!json.contains("based_on_commit"));
    assert!(!json.contains("pipeline_type"));
}

// --- FollowUp flexible deserialization ---

#[test]
fn json_follow_up_from_string() {
    let json = r#""Close WRK-020 as duplicate""#;
    let follow_up: FollowUp = serde_json::from_str(json).unwrap();
    assert_eq!(follow_up.title, "Close WRK-020 as duplicate");
    assert_eq!(follow_up.context, None);
    assert_eq!(follow_up.suggested_size, None);
    assert_eq!(follow_up.suggested_risk, None);
}

#[test]
fn json_follow_up_from_struct() {
    let json = r#"{"title": "Add tests", "context": "Coverage is low"}"#;
    let follow_up: FollowUp = serde_json::from_str(json).unwrap();
    assert_eq!(follow_up.title, "Add tests");
    assert_eq!(follow_up.context, Some("Coverage is low".to_string()));
    assert_eq!(follow_up.suggested_size, None);
    assert_eq!(follow_up.suggested_risk, None);
}

#[test]
fn json_follow_up_from_full_struct() {
    let json = r#"{
        "title": "Refactor module",
        "context": "Too complex",
        "suggested_size": "medium",
        "suggested_risk": "low"
    }"#;
    let follow_up: FollowUp = serde_json::from_str(json).unwrap();
    assert_eq!(follow_up.title, "Refactor module");
    assert_eq!(follow_up.context, Some("Too complex".to_string()));
    assert_eq!(follow_up.suggested_size, Some(SizeLevel::Medium));
    assert_eq!(follow_up.suggested_risk, Some(DimensionLevel::Low));
}

#[test]
fn json_phase_result_with_string_follow_ups() {
    let json = r#"{
        "item_id": "WRK-020",
        "phase": "triage",
        "result": "phase_complete",
        "summary": "Duplicate item",
        "follow_ups": [
            "Close WRK-020 as duplicate of WRK-017"
        ]
    }"#;
    let result: PhaseResult = serde_json::from_str(json).unwrap();
    assert_eq!(result.follow_ups.len(), 1);
    assert_eq!(
        result.follow_ups[0].title,
        "Close WRK-020 as duplicate of WRK-017"
    );
    assert_eq!(result.follow_ups[0].context, None);
}

#[test]
fn json_phase_result_with_mixed_follow_ups() {
    let json = r#"{
        "item_id": "WRK-005",
        "phase": "build",
        "result": "phase_complete",
        "summary": "Build done",
        "follow_ups": [
            "Simple string follow-up",
            {"title": "Structured follow-up", "context": "With context"}
        ]
    }"#;
    let result: PhaseResult = serde_json::from_str(json).unwrap();
    assert_eq!(result.follow_ups.len(), 2);
    assert_eq!(result.follow_ups[0].title, "Simple string follow-up");
    assert_eq!(result.follow_ups[0].context, None);
    assert_eq!(result.follow_ups[1].title, "Structured follow-up");
    assert_eq!(
        result.follow_ups[1].context,
        Some("With context".to_string())
    );
}
