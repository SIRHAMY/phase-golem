mod common;

use phase_golem::filter::{
    apply_filters, format_filter_criteria, parse_filter, validate_filter_criteria, FilterField,
    FilterValue,
};
use phase_golem::types::{BacklogFile, DimensionLevel, ItemStatus, SizeLevel};

use common::make_item;

fn make_backlog(items: Vec<phase_golem::types::BacklogItem>) -> BacklogFile {
    BacklogFile {
        items,
        schema_version: 2,
        next_item_id: 0,
    }
}

// --- Parse valid filters for all 7 field types ---

#[test]
fn parse_filter_status() {
    let f = parse_filter("status=ready").unwrap();
    assert_eq!(f.field, FilterField::Status);
    assert_eq!(f.values, vec![FilterValue::Status(ItemStatus::Ready)]);
}

#[test]
fn parse_filter_impact() {
    let f = parse_filter("impact=high").unwrap();
    assert_eq!(f.field, FilterField::Impact);
    assert_eq!(f.values, vec![FilterValue::Dimension(DimensionLevel::High)]);
}

#[test]
fn parse_filter_size() {
    let f = parse_filter("size=small").unwrap();
    assert_eq!(f.field, FilterField::Size);
    assert_eq!(f.values, vec![FilterValue::Size(SizeLevel::Small)]);
}

#[test]
fn parse_filter_risk() {
    let f = parse_filter("risk=low").unwrap();
    assert_eq!(f.field, FilterField::Risk);
    assert_eq!(f.values, vec![FilterValue::Dimension(DimensionLevel::Low)]);
}

#[test]
fn parse_filter_complexity() {
    let f = parse_filter("complexity=medium").unwrap();
    assert_eq!(f.field, FilterField::Complexity);
    assert_eq!(f.values, vec![FilterValue::Dimension(DimensionLevel::Medium)]);
}

#[test]
fn parse_filter_tag() {
    let f = parse_filter("tag=v1").unwrap();
    assert_eq!(f.field, FilterField::Tag);
    assert_eq!(f.values, vec![FilterValue::Tag("v1".to_string())]);
}

#[test]
fn parse_filter_pipeline_type() {
    let f = parse_filter("pipeline_type=feature").unwrap();
    assert_eq!(f.field, FilterField::PipelineType);
    assert_eq!(f.values, vec![FilterValue::PipelineType("feature".to_string())]);
}

// --- Invalid field name ---

#[test]
fn parse_filter_invalid_field() {
    let err = parse_filter("foo=bar").unwrap_err();
    assert!(err.contains("Unknown filter field: foo"));
    assert!(err.contains("Supported: status, impact, size, risk, complexity, tag, pipeline_type"));
}

// --- Invalid enum values ---

#[test]
fn parse_filter_invalid_size_value() {
    let err = parse_filter("size=gigantic").unwrap_err();
    assert!(err.contains("Invalid value 'gigantic' for field 'size'"));
    assert!(err.contains("Valid values: small, medium, large"));
}

#[test]
fn parse_filter_invalid_impact_value() {
    let err = parse_filter("impact=extreme").unwrap_err();
    assert!(err.contains("Invalid value 'extreme' for field 'impact'"));
    assert!(err.contains("Valid values: low, medium, high"));
}

#[test]
fn parse_filter_invalid_status_value() {
    let err = parse_filter("status=archived").unwrap_err();
    assert!(err.contains("Invalid value 'archived' for field 'status'"));
}

// --- Malformed syntax ---

#[test]
fn parse_filter_no_equals() {
    let err = parse_filter("impact-high").unwrap_err();
    assert!(err.contains("Filter must be in format KEY=VALUE"));
    assert!(err.contains("impact-high"));
}

#[test]
fn parse_filter_empty_string() {
    let err = parse_filter("").unwrap_err();
    assert!(err.contains("Filter must be in format KEY=VALUE"));
}

#[test]
fn parse_filter_whitespace_only() {
    let err = parse_filter("   ").unwrap_err();
    assert!(err.contains("Filter must be in format KEY=VALUE"));
}

#[test]
fn parse_filter_multiple_equals_splits_on_first() {
    // "key=val=ue" should split on first = → field="key", value="val=ue"
    // "key" is not a valid field, so it'll error on unknown field
    let err = parse_filter("key=val=ue").unwrap_err();
    assert!(err.contains("Unknown filter field: key"));
}

#[test]
fn parse_filter_equals_but_empty_value() {
    let err = parse_filter("impact=").unwrap_err();
    assert!(err.contains("Filter must be in format KEY=VALUE"));
}

#[test]
fn parse_filter_equals_but_empty_field() {
    let err = parse_filter("=high").unwrap_err();
    assert!(err.contains("Filter must be in format KEY=VALUE"));
}

// --- Case-insensitive parsing for enum fields ---

#[test]
fn parse_filter_case_insensitive_impact() {
    let f = parse_filter("impact=HIGH").unwrap();
    assert_eq!(f.values, vec![FilterValue::Dimension(DimensionLevel::High)]);
}

#[test]
fn parse_filter_case_insensitive_status_in_progress() {
    let f = parse_filter("status=IN_PROGRESS").unwrap();
    assert_eq!(f.values, vec![FilterValue::Status(ItemStatus::InProgress)]);
}

#[test]
fn parse_filter_case_insensitive_field_name() {
    let f = parse_filter("IMPACT=high").unwrap();
    assert_eq!(f.field, FilterField::Impact);
}

#[test]
fn parse_filter_case_insensitive_size() {
    let f = parse_filter("SIZE=LARGE").unwrap();
    assert_eq!(f.field, FilterField::Size);
    assert_eq!(f.values, vec![FilterValue::Size(SizeLevel::Large)]);
}

// --- Case-sensitive matching for tag and pipeline_type ---

#[test]
fn parse_filter_tag_preserves_case() {
    let f = parse_filter("tag=V1").unwrap();
    assert_eq!(f.values, vec![FilterValue::Tag("V1".to_string())]);
}

#[test]
fn parse_filter_pipeline_type_preserves_case() {
    let f = parse_filter("pipeline_type=Feature").unwrap();
    assert_eq!(f.values, vec![FilterValue::PipelineType("Feature".to_string())]);
}

// --- status=in_progress parses and matches ItemStatus::InProgress ---

#[test]
fn parse_and_match_status_in_progress() {
    let f = parse_filter("status=in_progress").unwrap();
    assert_eq!(f.values, vec![FilterValue::Status(ItemStatus::InProgress)]);

    let mut item = make_item("WRK-001", ItemStatus::InProgress);
    item.phase = Some("build".to_string());

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f.clone()], &snapshot);
    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].id, "WRK-001");
}

// --- Tag filtering: empty tags never match ---

#[test]
fn tag_filter_empty_tags_never_match() {
    let f = parse_filter("tag=v1").unwrap();
    let item = make_item("WRK-001", ItemStatus::Ready);
    // item.tags is empty by default from make_item

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f.clone()], &snapshot);
    assert!(filtered.items.is_empty());
}

// --- Tag filtering: case-sensitive ---

#[test]
fn tag_filter_case_sensitive() {
    let f = parse_filter("tag=v1").unwrap();

    let mut item = make_item("WRK-001", ItemStatus::Ready);
    item.tags = vec!["V1".to_string()];

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f.clone()], &snapshot);
    assert!(filtered.items.is_empty(), "tag=v1 should NOT match tag V1");
}

#[test]
fn tag_filter_exact_match() {
    let f = parse_filter("tag=v1").unwrap();

    let mut item = make_item("WRK-001", ItemStatus::Ready);
    item.tags = vec!["v1".to_string(), "other".to_string()];

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f.clone()], &snapshot);
    assert_eq!(filtered.items.len(), 1);
}

// --- Option::None fields never match ---

#[test]
fn none_impact_never_matches() {
    let f = parse_filter("impact=high").unwrap();
    let item = make_item("WRK-001", ItemStatus::Ready);
    // item.impact is None by default

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f.clone()], &snapshot);
    assert!(filtered.items.is_empty());
}

#[test]
fn none_size_never_matches() {
    let f = parse_filter("size=medium").unwrap();
    let item = make_item("WRK-001", ItemStatus::Ready);

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f.clone()], &snapshot);
    assert!(filtered.items.is_empty());
}

#[test]
fn none_risk_never_matches() {
    let f = parse_filter("risk=low").unwrap();
    let item = make_item("WRK-001", ItemStatus::Ready);

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f.clone()], &snapshot);
    assert!(filtered.items.is_empty());
}

#[test]
fn none_complexity_never_matches() {
    let f = parse_filter("complexity=high").unwrap();
    let item = make_item("WRK-001", ItemStatus::Ready);

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f.clone()], &snapshot);
    assert!(filtered.items.is_empty());
}

#[test]
fn none_pipeline_type_never_matches() {
    let f = parse_filter("pipeline_type=feature").unwrap();
    let item = make_item("WRK-001", ItemStatus::Ready);

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f.clone()], &snapshot);
    assert!(filtered.items.is_empty());
}

// --- apply_filter returns correct subset ---

#[test]
fn apply_filter_returns_matching_subset() {
    let f = parse_filter("impact=high").unwrap();

    let mut item1 = make_item("WRK-001", ItemStatus::Ready);
    item1.impact = Some(DimensionLevel::High);

    let mut item2 = make_item("WRK-002", ItemStatus::Ready);
    item2.impact = Some(DimensionLevel::Low);

    let mut item3 = make_item("WRK-003", ItemStatus::InProgress);
    item3.impact = Some(DimensionLevel::High);

    let snapshot = make_backlog(vec![item1, item2, item3]);
    let filtered = apply_filters(&[f.clone()], &snapshot);

    assert_eq!(filtered.items.len(), 2);
    assert_eq!(filtered.items[0].id, "WRK-001");
    assert_eq!(filtered.items[1].id, "WRK-003");
}

// --- apply_filter on empty snapshot ---

#[test]
fn apply_filter_empty_snapshot_returns_empty() {
    let f = parse_filter("impact=high").unwrap();
    let snapshot = make_backlog(vec![]);
    let filtered = apply_filters(&[f.clone()], &snapshot);
    assert!(filtered.items.is_empty());
    assert_eq!(filtered.schema_version, 2);
}

// --- apply_filter preserves schema_version ---

#[test]
fn apply_filter_preserves_schema_version() {
    let f = parse_filter("status=ready").unwrap();
    let backlog = BacklogFile {
        items: vec![make_item("WRK-001", ItemStatus::Ready)],
        schema_version: 2,
        next_item_id: 0,
    };
    let filtered = apply_filters(&[f.clone()], &backlog);
    assert_eq!(filtered.schema_version, 2);
}

// --- Display impl for FilterCriterion ---

#[test]
fn filter_criterion_display() {
    let f = parse_filter("impact=high").unwrap();
    assert_eq!(f.to_string(), "impact=high");

    let f = parse_filter("tag=v1").unwrap();
    assert_eq!(f.to_string(), "tag=v1");

    let f = parse_filter("status=in_progress").unwrap();
    assert_eq!(f.to_string(), "status=in_progress");
}

#[test]
fn filter_criterion_display_roundtrip() {
    let filters = vec![
        "status=new",
        "status=in_progress",
        "impact=high",
        "size=small",
        "risk=low",
        "complexity=medium",
        "tag=v1",
        "pipeline_type=feature",
    ];
    for raw in filters {
        let parsed = parse_filter(raw).unwrap();
        let displayed = parsed.to_string();
        let reparsed = parse_filter(&displayed).unwrap();
        assert_eq!(parsed, reparsed, "Round-trip failed for '{}'", raw);
    }
}

// --- Pipeline type case-sensitive matching ---

#[test]
fn pipeline_type_case_sensitive_matching() {
    let f = parse_filter("pipeline_type=feature").unwrap();

    let mut item = make_item("WRK-001", ItemStatus::Ready);
    item.pipeline_type = Some("Feature".to_string());

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f.clone()], &snapshot);
    assert!(
        filtered.items.is_empty(),
        "pipeline_type=feature should NOT match Feature"
    );
}

#[test]
fn pipeline_type_exact_match() {
    let f = parse_filter("pipeline_type=feature").unwrap();

    let mut item = make_item("WRK-001", ItemStatus::Ready);
    item.pipeline_type = Some("feature".to_string());

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f.clone()], &snapshot);
    assert_eq!(filtered.items.len(), 1);
}

// --- Status filter matching ---

#[test]
fn status_filter_matches_correctly() {
    let f = parse_filter("status=blocked").unwrap();

    let item1 = make_item("WRK-001", ItemStatus::Blocked);
    let item2 = make_item("WRK-002", ItemStatus::Ready);
    let item3 = make_item("WRK-003", ItemStatus::Blocked);

    let snapshot = make_backlog(vec![item1, item2, item3]);
    let filtered = apply_filters(&[f.clone()], &snapshot);

    assert_eq!(filtered.items.len(), 2);
    assert_eq!(filtered.items[0].id, "WRK-001");
    assert_eq!(filtered.items[1].id, "WRK-003");
}

// --- apply_filter preserves next_item_id ---

#[test]
fn apply_filter_preserves_next_item_id() {
    let f = parse_filter("status=ready").unwrap();
    let backlog = BacklogFile {
        items: vec![make_item("WRK-001", ItemStatus::Ready)],
        schema_version: 3,
        next_item_id: 42,
    };
    let filtered = apply_filters(&[f.clone()], &backlog);
    assert_eq!(filtered.next_item_id, 42);
}

// --- Multiple equals in tag value ---

#[test]
fn parse_filter_tag_with_equals_in_value() {
    // "tag=key=value" should parse with field=tag, value="key=value"
    let f = parse_filter("tag=key=value").unwrap();
    assert_eq!(f.field, FilterField::Tag);
    assert_eq!(f.values, vec![FilterValue::Tag("key=value".to_string())]);
}

// --- validate_filter_criteria tests ---

#[test]
fn validate_empty_slice_returns_ok() {
    assert!(validate_filter_criteria(&[]).is_ok());
}

#[test]
fn validate_single_criterion_returns_ok() {
    let c = parse_filter("impact=high").unwrap();
    assert!(validate_filter_criteria(&[c]).is_ok());
}

#[test]
fn validate_two_different_scalar_fields_returns_ok() {
    let c1 = parse_filter("impact=high").unwrap();
    let c2 = parse_filter("size=small").unwrap();
    assert!(validate_filter_criteria(&[c1, c2]).is_ok());
}

#[test]
fn validate_duplicate_scalar_field_returns_err() {
    let c1 = parse_filter("impact=high").unwrap();
    let c2 = parse_filter("impact=low").unwrap();
    let err = validate_filter_criteria(&[c1, c2]).unwrap_err();
    assert!(err.contains("Field 'impact' specified multiple times"));
    assert!(err.contains("--only impact=value1,value2"));
}

#[test]
fn validate_identical_scalar_field_value_returns_err() {
    let c1 = parse_filter("impact=high").unwrap();
    let c2 = parse_filter("impact=high").unwrap();
    let err = validate_filter_criteria(&[c1, c2]).unwrap_err();
    assert!(err.contains("Field 'impact' specified multiple times"));
}

#[test]
fn validate_two_different_tag_values_returns_ok() {
    let c1 = parse_filter("tag=backend").unwrap();
    let c2 = parse_filter("tag=sprint-1").unwrap();
    assert!(validate_filter_criteria(&[c1, c2]).is_ok());
}

#[test]
fn validate_identical_tag_values_returns_err() {
    let c1 = parse_filter("tag=backend").unwrap();
    let c2 = parse_filter("tag=backend").unwrap();
    let err = validate_filter_criteria(&[c1, c2]).unwrap_err();
    assert!(err.contains("Duplicate filter: tag=backend specified multiple times"));
}

#[test]
fn validate_mixed_scalar_and_tag_returns_ok() {
    let c1 = parse_filter("impact=high").unwrap();
    let c2 = parse_filter("tag=backend").unwrap();
    let c3 = parse_filter("size=small").unwrap();
    assert!(validate_filter_criteria(&[c1, c2, c3]).is_ok());
}

#[test]
fn validate_non_adjacent_duplicate_scalar_detected() {
    let c1 = parse_filter("impact=high").unwrap();
    let c2 = parse_filter("tag=backend").unwrap();
    let c3 = parse_filter("impact=low").unwrap();
    let err = validate_filter_criteria(&[c1, c2, c3]).unwrap_err();
    assert!(err.contains("Field 'impact' specified multiple times"));
}

// --- apply_filters tests ---

#[test]
fn apply_filters_two_criteria_and() {
    let c1 = parse_filter("impact=high").unwrap();
    let c2 = parse_filter("size=small").unwrap();

    let mut item1 = make_item("WRK-001", ItemStatus::Ready);
    item1.impact = Some(DimensionLevel::High);
    item1.size = Some(SizeLevel::Small);

    let mut item2 = make_item("WRK-002", ItemStatus::Ready);
    item2.impact = Some(DimensionLevel::High);
    item2.size = Some(SizeLevel::Large);

    let mut item3 = make_item("WRK-003", ItemStatus::Ready);
    item3.impact = Some(DimensionLevel::Low);
    item3.size = Some(SizeLevel::Small);

    let snapshot = make_backlog(vec![item1, item2, item3]);
    let filtered = apply_filters(&[c1, c2], &snapshot);

    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].id, "WRK-001");
}

#[test]
fn apply_filters_item_matching_one_criterion_excluded() {
    let c1 = parse_filter("impact=high").unwrap();
    let c2 = parse_filter("risk=low").unwrap();

    let mut item = make_item("WRK-001", ItemStatus::Ready);
    item.impact = Some(DimensionLevel::High);
    item.risk = Some(DimensionLevel::Medium);

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[c1, c2], &snapshot);

    assert!(filtered.items.is_empty());
}

#[test]
fn apply_filters_none_optional_field_excluded_by_and() {
    let c1 = parse_filter("impact=high").unwrap();
    let c2 = parse_filter("size=small").unwrap();

    let mut item = make_item("WRK-001", ItemStatus::Ready);
    item.impact = Some(DimensionLevel::High);
    // size is None

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[c1, c2], &snapshot);

    assert!(filtered.items.is_empty());
}

#[test]
fn apply_filters_multi_tag_and() {
    let c1 = parse_filter("tag=backend").unwrap();
    let c2 = parse_filter("tag=sprint-1").unwrap();

    let mut item1 = make_item("WRK-001", ItemStatus::Ready);
    item1.tags = vec!["backend".to_string(), "sprint-1".to_string()];

    let mut item2 = make_item("WRK-002", ItemStatus::Ready);
    item2.tags = vec!["backend".to_string()];

    let mut item3 = make_item("WRK-003", ItemStatus::Ready);
    item3.tags = vec!["sprint-1".to_string()];

    let snapshot = make_backlog(vec![item1, item2, item3]);
    let filtered = apply_filters(&[c1, c2], &snapshot);

    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].id, "WRK-001");
}

#[test]
fn apply_filters_empty_criteria_returns_all() {
    let item1 = make_item("WRK-001", ItemStatus::Ready);
    let item2 = make_item("WRK-002", ItemStatus::InProgress);

    let snapshot = make_backlog(vec![item1, item2]);
    let filtered = apply_filters(&[], &snapshot);

    assert_eq!(filtered.items.len(), 2);
}

#[test]
fn apply_filters_single_criterion_returns_matching() {
    let c = parse_filter("impact=high").unwrap();

    let mut item1 = make_item("WRK-001", ItemStatus::Ready);
    item1.impact = Some(DimensionLevel::High);

    let mut item2 = make_item("WRK-002", ItemStatus::Ready);
    item2.impact = Some(DimensionLevel::Low);

    let snapshot = make_backlog(vec![item1, item2]);
    let filtered = apply_filters(&[c], &snapshot);

    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].id, "WRK-001");
}

// --- format_filter_criteria tests ---

#[test]
fn format_filter_criteria_empty_slice() {
    assert_eq!(format_filter_criteria(&[]), "");
}

#[test]
fn format_filter_criteria_single() {
    let c = parse_filter("impact=high").unwrap();
    assert_eq!(format_filter_criteria(&[c]), "impact=high");
}

#[test]
fn format_filter_criteria_two() {
    let c1 = parse_filter("impact=high").unwrap();
    let c2 = parse_filter("size=small").unwrap();
    assert_eq!(
        format_filter_criteria(&[c1, c2]),
        "impact=high AND size=small"
    );
}

#[test]
fn format_filter_criteria_three() {
    let c1 = parse_filter("impact=high").unwrap();
    let c2 = parse_filter("size=small").unwrap();
    let c3 = parse_filter("status=ready").unwrap();
    assert_eq!(
        format_filter_criteria(&[c1, c2, c3]),
        "impact=high AND size=small AND status=ready"
    );
}

#[test]
fn apply_filters_three_heterogeneous_criteria() {
    let c1 = parse_filter("status=ready").unwrap();
    let c2 = parse_filter("impact=high").unwrap();
    let c3 = parse_filter("tag=backend").unwrap();

    let mut item1 = make_item("WRK-001", ItemStatus::Ready);
    item1.impact = Some(DimensionLevel::High);
    item1.tags = vec!["backend".to_string()];

    let mut item2 = make_item("WRK-002", ItemStatus::Ready);
    item2.impact = Some(DimensionLevel::High);
    // no tag

    let mut item3 = make_item("WRK-003", ItemStatus::InProgress);
    item3.impact = Some(DimensionLevel::High);
    item3.tags = vec!["backend".to_string()];

    let mut item4 = make_item("WRK-004", ItemStatus::Ready);
    item4.impact = Some(DimensionLevel::Low);
    item4.tags = vec!["backend".to_string()];

    let snapshot = make_backlog(vec![item1, item2, item3, item4]);
    let filtered = apply_filters(&[c1, c2, c3], &snapshot);

    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].id, "WRK-001");
}

// --- Multi-value parsing (happy path) ---

#[test]
fn parse_filter_multi_value_impact() {
    let f = parse_filter("impact=high,medium").unwrap();
    assert_eq!(f.field, FilterField::Impact);
    assert_eq!(
        f.values,
        vec![
            FilterValue::Dimension(DimensionLevel::High),
            FilterValue::Dimension(DimensionLevel::Medium),
        ]
    );
}

#[test]
fn parse_filter_multi_value_status() {
    let f = parse_filter("status=ready,blocked").unwrap();
    assert_eq!(f.field, FilterField::Status);
    assert_eq!(
        f.values,
        vec![
            FilterValue::Status(ItemStatus::Ready),
            FilterValue::Status(ItemStatus::Blocked),
        ]
    );
}

#[test]
fn parse_filter_multi_value_tag() {
    let f = parse_filter("tag=a,b").unwrap();
    assert_eq!(f.field, FilterField::Tag);
    assert_eq!(
        f.values,
        vec![
            FilterValue::Tag("a".to_string()),
            FilterValue::Tag("b".to_string()),
        ]
    );
}

#[test]
fn parse_filter_multi_value_pipeline_type() {
    let f = parse_filter("pipeline_type=feature,bugfix").unwrap();
    assert_eq!(f.field, FilterField::PipelineType);
    assert_eq!(
        f.values,
        vec![
            FilterValue::PipelineType("feature".to_string()),
            FilterValue::PipelineType("bugfix".to_string()),
        ]
    );
}

// --- Empty token rejection ---

#[test]
fn parse_filter_empty_token_middle() {
    let err = parse_filter("impact=high,,low").unwrap_err();
    assert!(err.contains("Empty value in comma-separated list"));
}

#[test]
fn parse_filter_empty_token_leading() {
    let err = parse_filter("impact=,high").unwrap_err();
    assert!(err.contains("Empty value in comma-separated list"));
}

#[test]
fn parse_filter_empty_token_trailing() {
    let err = parse_filter("impact=high,").unwrap_err();
    assert!(err.contains("Empty value in comma-separated list"));
}

#[test]
fn parse_filter_comma_only() {
    let err = parse_filter("impact=,").unwrap_err();
    assert!(err.contains("Empty value in comma-separated list"));
}

// --- Within-list duplicate rejection ---

#[test]
fn parse_filter_duplicate_value_rejected() {
    let err = parse_filter("impact=high,high").unwrap_err();
    assert!(err.contains("Duplicate value 'high' in comma-separated list for field 'impact'"));
}

#[test]
fn parse_filter_duplicate_case_insensitive_enum_rejected() {
    // "HIGH" and "high" parse to the same DimensionLevel::High
    let err = parse_filter("impact=high,HIGH").unwrap_err();
    assert!(err.contains("Duplicate value"));
}

#[test]
fn parse_filter_tag_duplicate_case_sensitive_same_rejected() {
    let err = parse_filter("tag=a,a").unwrap_err();
    assert!(err.contains("Duplicate value 'a' in comma-separated list for field 'tag'"));
}

#[test]
fn parse_filter_tag_different_case_accepted() {
    // Tags are case-sensitive: "a" and "A" are different values
    let f = parse_filter("tag=a,A").unwrap();
    assert_eq!(
        f.values,
        vec![
            FilterValue::Tag("a".to_string()),
            FilterValue::Tag("A".to_string()),
        ]
    );
}

// --- Multi-value OR matching ---

#[test]
fn multi_value_or_matches_first_value() {
    let f = parse_filter("impact=high,medium").unwrap();

    let mut item = make_item("WRK-001", ItemStatus::Ready);
    item.impact = Some(DimensionLevel::High);

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f], &snapshot);
    assert_eq!(filtered.items.len(), 1);
}

#[test]
fn multi_value_or_matches_second_value() {
    let f = parse_filter("impact=high,medium").unwrap();

    let mut item = make_item("WRK-001", ItemStatus::Ready);
    item.impact = Some(DimensionLevel::Medium);

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f], &snapshot);
    assert_eq!(filtered.items.len(), 1);
}

#[test]
fn multi_value_or_no_match() {
    let f = parse_filter("impact=high,medium").unwrap();

    let item = make_item("WRK-001", ItemStatus::Ready);
    // impact is None

    let snapshot = make_backlog(vec![item]);
    let filtered = apply_filters(&[f], &snapshot);
    assert!(filtered.items.is_empty());
}

#[test]
fn multi_value_or_composes_with_cross_field_and() {
    let c1 = parse_filter("impact=high,medium").unwrap();
    let c2 = parse_filter("size=small").unwrap();

    let mut item1 = make_item("WRK-001", ItemStatus::Ready);
    item1.impact = Some(DimensionLevel::High);
    item1.size = Some(SizeLevel::Small);

    let mut item2 = make_item("WRK-002", ItemStatus::Ready);
    item2.impact = Some(DimensionLevel::Medium);
    item2.size = Some(SizeLevel::Large);

    let mut item3 = make_item("WRK-003", ItemStatus::Ready);
    item3.impact = Some(DimensionLevel::Low);
    item3.size = Some(SizeLevel::Small);

    let snapshot = make_backlog(vec![item1, item2, item3]);
    let filtered = apply_filters(&[c1, c2], &snapshot);

    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.items[0].id, "WRK-001");
}

#[test]
fn multi_value_or_size_matching() {
    let f = parse_filter("size=small,medium").unwrap();

    let mut item1 = make_item("WRK-001", ItemStatus::Ready);
    item1.size = Some(SizeLevel::Small);

    let mut item2 = make_item("WRK-002", ItemStatus::Ready);
    item2.size = Some(SizeLevel::Large);

    let mut item3 = make_item("WRK-003", ItemStatus::Ready);
    item3.size = Some(SizeLevel::Medium);

    let snapshot = make_backlog(vec![item1, item2, item3]);
    let filtered = apply_filters(&[f], &snapshot);

    assert_eq!(filtered.items.len(), 2);
    assert_eq!(filtered.items[0].id, "WRK-001");
    assert_eq!(filtered.items[1].id, "WRK-003");
}

#[test]
fn multi_value_or_pipeline_type_matching() {
    let f = parse_filter("pipeline_type=feature,bugfix").unwrap();

    let mut item1 = make_item("WRK-001", ItemStatus::Ready);
    item1.pipeline_type = Some("feature".to_string());

    let mut item2 = make_item("WRK-002", ItemStatus::Ready);
    item2.pipeline_type = Some("release".to_string());

    let mut item3 = make_item("WRK-003", ItemStatus::Ready);
    item3.pipeline_type = Some("bugfix".to_string());

    let snapshot = make_backlog(vec![item1, item2, item3]);
    let filtered = apply_filters(&[f], &snapshot);

    assert_eq!(filtered.items.len(), 2);
    assert_eq!(filtered.items[0].id, "WRK-001");
    assert_eq!(filtered.items[1].id, "WRK-003");
}

// --- Multi-value display ---

#[test]
fn multi_value_display() {
    let f = parse_filter("impact=high,medium").unwrap();
    assert_eq!(f.to_string(), "impact=high,medium");
}

#[test]
fn format_filter_criteria_multi_value_and_single() {
    let c1 = parse_filter("impact=high,medium").unwrap();
    let c2 = parse_filter("size=small").unwrap();
    assert_eq!(
        format_filter_criteria(&[c1, c2]),
        "impact=high,medium AND size=small"
    );
}

// --- Tag OR + AND composition ---

#[test]
fn tag_or_matches_either() {
    let f = parse_filter("tag=a,b").unwrap();

    let mut item1 = make_item("WRK-001", ItemStatus::Ready);
    item1.tags = vec!["a".to_string()];

    let mut item2 = make_item("WRK-002", ItemStatus::Ready);
    item2.tags = vec!["b".to_string()];

    let mut item3 = make_item("WRK-003", ItemStatus::Ready);
    item3.tags = vec!["c".to_string()];

    let snapshot = make_backlog(vec![item1, item2, item3]);
    let filtered = apply_filters(&[f], &snapshot);

    assert_eq!(filtered.items.len(), 2);
    assert_eq!(filtered.items[0].id, "WRK-001");
    assert_eq!(filtered.items[1].id, "WRK-002");
}

#[test]
fn tag_or_and_composition() {
    // (a or b) AND c
    let c1 = parse_filter("tag=a,b").unwrap();
    let c2 = parse_filter("tag=c").unwrap();

    let mut item1 = make_item("WRK-001", ItemStatus::Ready);
    item1.tags = vec!["a".to_string(), "c".to_string()];

    let mut item2 = make_item("WRK-002", ItemStatus::Ready);
    item2.tags = vec!["b".to_string(), "c".to_string()];

    let mut item3 = make_item("WRK-003", ItemStatus::Ready);
    item3.tags = vec!["a".to_string(), "b".to_string()];

    let mut item4 = make_item("WRK-004", ItemStatus::Ready);
    item4.tags = vec!["c".to_string()];

    let snapshot = make_backlog(vec![item1, item2, item3, item4]);
    let filtered = apply_filters(&[c1, c2], &snapshot);

    assert_eq!(filtered.items.len(), 2);
    assert_eq!(filtered.items[0].id, "WRK-001");
    assert_eq!(filtered.items[1].id, "WRK-002");
}

// --- Whitespace trimming ---

#[test]
fn parse_filter_whitespace_after_comma_trimmed() {
    let f = parse_filter("impact=high, medium").unwrap();
    assert_eq!(
        f.values,
        vec![
            FilterValue::Dimension(DimensionLevel::High),
            FilterValue::Dimension(DimensionLevel::Medium),
        ]
    );
}

// --- Multi-value roundtrip ---

#[test]
fn multi_value_roundtrip() {
    let original = parse_filter("impact=high,medium").unwrap();
    let displayed = original.to_string();
    let reparsed = parse_filter(&displayed).unwrap();
    assert_eq!(original, reparsed);
}

// --- Invalid value within comma list ---

#[test]
fn parse_filter_invalid_value_in_comma_list() {
    let err = parse_filter("size=small,huge").unwrap_err();
    assert!(err.contains("Invalid value 'huge' for field 'size'"));
}

// --- Fail-fast ordering ---

#[test]
fn parse_filter_fail_fast_first_invalid_token() {
    let err = parse_filter("impact=high,huge,medium").unwrap_err();
    assert!(err.contains("huge"));
}

// --- Cross-flag duplicate validation with multi-value criteria ---

#[test]
fn validate_cross_flag_duplicate_with_multi_value() {
    let c1 = parse_filter("impact=high,medium").unwrap();
    let c2 = parse_filter("impact=low").unwrap();
    let err = validate_filter_criteria(&[c1, c2]).unwrap_err();
    assert!(err.contains("Field 'impact' specified multiple times"));
}

// --- Identical multi-value tag criteria across flags ---

#[test]
fn validate_identical_multi_value_tag_rejected() {
    let c1 = parse_filter("tag=a,b").unwrap();
    let c2 = parse_filter("tag=a,b").unwrap();
    let err = validate_filter_criteria(&[c1, c2]).unwrap_err();
    assert!(err.contains("Duplicate filter: tag=a,b specified multiple times"));
}

// --- Tag with equals + commas interaction ---

#[test]
fn parse_filter_tag_equals_and_commas() {
    let f = parse_filter("tag=key=val1,key=val2").unwrap();
    assert_eq!(
        f.values,
        vec![
            FilterValue::Tag("key=val1".to_string()),
            FilterValue::Tag("key=val2".to_string()),
        ]
    );
}

// --- Updated validation error message ---

#[test]
fn validate_duplicate_scalar_error_mentions_separate_flags() {
    let c1 = parse_filter("impact=high").unwrap();
    let c2 = parse_filter("impact=low").unwrap();
    let err = validate_filter_criteria(&[c1, c2]).unwrap_err();
    assert!(err.contains("in separate --only flags"));
}
