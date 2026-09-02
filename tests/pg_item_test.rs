mod common;

use phase_golem::pg_item::{
    self, PgItem, X_PG_BLOCKED_TYPE, X_PG_COMPLEXITY, X_PG_DESCRIPTION, X_PG_HUMAN_DECISION,
    X_PG_IMPACT, X_PG_OWNER, X_PG_PHASE_POOL, X_PG_RISK, X_PG_SIZE, X_PG_TEMPLATE_NODE_KEY,
};
use phase_golem::types::{
    BlockType, DimensionLevel, ItemUpdate, PhasePool, SizeLevel, StructuredDescription,
    UpdatedAssessments,
};
use task_golem::model::item::Item;
use task_golem::model::status::Status;

#[test]
fn new_pg_item_uses_native_status_and_pg_ownership_metadata() {
    let item = common::make_pg_item(common::ID_1, Status::Todo);
    assert_eq!(item.status(), Status::Todo);
    assert!(item.is_pg_owned());
    assert_eq!(item.0.extensions[X_PG_OWNER], "phase-golem");
    let removed_lifecycle_key = ["x-pg", "status"].join("-");
    assert!(!item.0.extensions.contains_key(&removed_lifecycle_key));
}

#[test]
fn materialized_gate_and_node_metadata_are_typed() {
    let mut item = common::make_pg_item(common::ID_1, Status::Todo);
    item.0
        .extensions
        .insert(X_PG_HUMAN_DECISION.to_string(), serde_json::json!(true));
    item.0.extensions.insert(
        X_PG_TEMPLATE_NODE_KEY.to_string(),
        serde_json::json!("review"),
    );
    assert!(item.is_human_gate());
    assert_eq!(item.template_node_key().as_deref(), Some("review"));
}

#[test]
fn metadata_updates_do_not_change_native_lifecycle_fields() {
    let mut item = common::make_doing_pg_item(common::ID_1, "build");
    let claimed_by = item.0.claimed_by.clone();
    pg_item::apply_metadata_update(
        &mut item.0,
        ItemUpdate::UpdateAssessments(UpdatedAssessments {
            size: Some(SizeLevel::Medium),
            complexity: Some(DimensionLevel::Low),
            risk: None,
            impact: None,
        }),
    );
    pg_item::apply_metadata_update(&mut item.0, ItemUpdate::SetPhasePool(PhasePool::Main));
    assert_eq!(PgItem(item.0.clone()).status(), Status::Doing);
    assert_eq!(item.0.claimed_by, claimed_by);
    assert_eq!(PgItem(item.0).size(), Some(SizeLevel::Medium));
}

#[test]
fn string_metadata_setters_round_trip_and_clear() {
    type Setter = fn(&mut Item, Option<&str>);
    type Getter = fn(&PgItem) -> Option<String>;

    for (setter, getter, value) in [
        (
            pg_item::set_phase as Setter,
            PgItem::phase as Getter,
            "build",
        ),
        (pg_item::set_pipeline_type, PgItem::pipeline_type, "feature"),
        (pg_item::set_origin, PgItem::origin, "manual"),
        (
            pg_item::set_unblock_context,
            PgItem::unblock_context,
            "operator approved",
        ),
        (
            pg_item::set_last_phase_commit,
            PgItem::last_phase_commit,
            "abc123",
        ),
    ] {
        let mut item = common::make_pg_item(common::ID_1, Status::Todo);

        setter(&mut item.0, Some(value));
        assert_eq!(getter(&item).as_deref(), Some(value));

        setter(&mut item.0, None);
        assert_eq!(getter(&item), None);
    }
}

#[test]
fn dimension_metadata_setters_round_trip_and_clear() {
    type Setter = fn(&mut Item, Option<&DimensionLevel>);
    type Getter = fn(&PgItem) -> Option<DimensionLevel>;

    for (setter, getter, value) in [
        (
            pg_item::set_complexity as Setter,
            PgItem::complexity as Getter,
            DimensionLevel::Low,
        ),
        (pg_item::set_risk, PgItem::risk, DimensionLevel::Medium),
        (pg_item::set_impact, PgItem::impact, DimensionLevel::High),
    ] {
        let mut item = common::make_pg_item(common::ID_1, Status::Todo);

        setter(&mut item.0, Some(&value));
        assert_eq!(getter(&item), Some(value));

        setter(&mut item.0, None);
        assert_eq!(getter(&item), None);
    }
}

#[test]
fn enum_and_description_metadata_round_trip_and_clear() {
    let mut item = common::make_pg_item(common::ID_1, Status::Todo);
    let description = StructuredDescription {
        context: "User cannot publish".to_string(),
        problem: "Missing permission check".to_string(),
        solution: "Enforce publish permission".to_string(),
        impact: "Protects private drafts".to_string(),
        sizing_rationale: "One boundary change".to_string(),
    };

    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Pre));
    pg_item::set_size(&mut item.0, Some(&SizeLevel::Large));
    pg_item::set_blocked_type(&mut item.0, Some(&BlockType::Decision));
    pg_item::set_structured_description(&mut item.0, Some(&description));

    assert_eq!(item.phase_pool(), Some(PhasePool::Pre));
    assert_eq!(item.size(), Some(SizeLevel::Large));
    assert_eq!(item.blocked_type(), Some(BlockType::Decision));
    assert_eq!(item.structured_description(), Some(description));
    assert_eq!(item.0.description.as_deref(), Some("User cannot publish"));

    pg_item::set_phase_pool(&mut item.0, None);
    pg_item::set_size(&mut item.0, None);
    pg_item::set_blocked_type(&mut item.0, None);
    pg_item::set_structured_description(&mut item.0, None);

    assert_eq!(item.phase_pool(), None);
    assert_eq!(item.size(), None);
    assert_eq!(item.blocked_type(), None);
    assert_eq!(item.structured_description(), None);
    assert_eq!(item.0.description, None);
}

#[test]
fn invalid_persisted_metadata_is_treated_as_absent() {
    let mut item = common::make_pg_item(common::ID_1, Status::Todo);
    for key in [X_PG_COMPLEXITY, X_PG_RISK, X_PG_IMPACT] {
        item.0
            .extensions
            .insert(key.to_string(), serde_json::json!("invalid"));
    }
    item.0
        .extensions
        .insert(X_PG_PHASE_POOL.to_string(), serde_json::json!("invalid"));
    item.0
        .extensions
        .insert(X_PG_SIZE.to_string(), serde_json::json!("invalid"));
    item.0
        .extensions
        .insert(X_PG_BLOCKED_TYPE.to_string(), serde_json::json!("invalid"));
    item.0
        .extensions
        .insert(X_PG_DESCRIPTION.to_string(), serde_json::json!("invalid"));

    assert_eq!(item.phase_pool(), None);
    assert_eq!(item.size(), None);
    assert_eq!(item.complexity(), None);
    assert_eq!(item.risk(), None);
    assert_eq!(item.impact(), None);
    assert_eq!(item.blocked_type(), None);
    assert_eq!(item.structured_description(), None);
}

#[test]
#[should_panic(expected = "lifecycle updates must be applied by the coordinator")]
fn metadata_boundary_rejects_lifecycle_mutation() {
    let mut item = common::make_pg_item(common::ID_1, Status::Todo);
    pg_item::apply_metadata_update(&mut item.0, ItemUpdate::TransitionStatus(Status::Doing));
}
