mod common;

use phase_golem::filter::{apply_filters, format_filter_criteria, parse_filter};
use phase_golem::pg_item;
use phase_golem::types::{DimensionLevel, SizeLevel};
use task_golem::model::status::Status;

#[test]
fn status_filter_uses_native_tg_names() {
    let criterion = parse_filter("status=todo,blocked").expect("parse native statuses");
    assert_eq!(format_filter_criteria(&[criterion]), "status=todo,blocked");
    assert!(parse_filter("status=ready").is_err());
}

#[test]
fn filters_native_status_and_pg_metadata() {
    // Arrange
    let mut todo = common::make_pg_item(common::ID_1, Status::Todo);
    pg_item::set_impact(&mut todo.0, Some(&DimensionLevel::High));
    let blocked = common::make_blocked_pg_item(common::ID_2, Status::Doing);
    let criteria = [
        parse_filter("status=todo").expect("status filter"),
        parse_filter("impact=high").expect("impact filter"),
    ];

    // Act
    let filtered = apply_filters(&criteria, &[todo, blocked]);

    // Assert
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id(), common::ID_1);
}

#[test]
fn repeated_scalar_filter_is_rejected() {
    let filters = [
        parse_filter("status=todo").expect("first filter"),
        parse_filter("status=doing").expect("second filter"),
    ];
    assert!(phase_golem::filter::validate_filter_criteria(&filters).is_err());
}

#[test]
fn supported_filter_fields_parse_and_round_trip() {
    // Arrange
    let cases = [
        ("STATUS=DOING", "status=doing"),
        ("impact=high", "impact=high"),
        ("size=large", "size=large"),
        ("risk=medium", "risk=medium"),
        ("complexity=low", "complexity=low"),
        ("tag=KeepCase", "tag=KeepCase"),
        ("pipeline_type=Feature", "pipeline_type=Feature"),
    ];

    // Act and assert
    for (raw, expected) in cases {
        assert_eq!(
            parse_filter(raw).expect("parse filter").to_string(),
            expected
        );
    }
}

#[test]
fn malformed_and_duplicate_filter_values_are_rejected() {
    for raw in [
        "",
        "status",
        "=todo",
        "status=",
        "unknown=value",
        "size=huge",
        "status=todo,,done",
        "status=todo,todo",
        "status=todo,TODO",
    ] {
        assert!(parse_filter(raw).is_err(), "{raw} should be rejected");
    }
}

#[test]
fn comma_values_are_or_and_separate_fields_are_and() {
    // Arrange
    let mut first = common::make_pg_item(common::ID_1, Status::Todo);
    pg_item::set_size(&mut first.0, Some(&SizeLevel::Small));
    pg_item::set_risk(&mut first.0, Some(&DimensionLevel::High));
    pg_item::set_pipeline_type(&mut first.0, Some("feature"));
    first.0.tags = vec!["backend".to_string()];

    let mut second = common::make_pg_item(common::ID_2, Status::Blocked);
    pg_item::set_size(&mut second.0, Some(&SizeLevel::Medium));
    pg_item::set_risk(&mut second.0, Some(&DimensionLevel::Low));
    pg_item::set_pipeline_type(&mut second.0, Some("feature"));
    second.0.tags = vec!["frontend".to_string()];

    let criteria = [
        parse_filter("status=todo,blocked").expect("status filter"),
        parse_filter("size=small,medium").expect("size filter"),
        parse_filter("pipeline_type=feature").expect("pipeline filter"),
        parse_filter("tag=backend,frontend").expect("tag filter"),
    ];

    // Act
    let filtered = apply_filters(&criteria, &[first, second]);

    // Assert
    assert_eq!(filtered.len(), 2);
}

#[test]
fn absent_metadata_and_case_mismatched_tags_do_not_match() {
    // Arrange
    let mut item = common::make_pg_item(common::ID_1, Status::Todo);
    item.0.tags = vec!["Backend".to_string()];

    // Act
    let missing_impact = apply_filters(
        &[parse_filter("impact=high").expect("impact filter")],
        std::slice::from_ref(&item),
    );
    let mismatched_tag =
        apply_filters(&[parse_filter("tag=backend").expect("tag filter")], &[item]);

    // Assert
    assert!(missing_impact.is_empty());
    assert!(mismatched_tag.is_empty());
}
