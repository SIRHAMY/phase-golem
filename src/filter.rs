use std::collections::HashSet;

use crate::pg_item::PgItem;
use crate::types::{
    parse_dimension_level, parse_item_status, parse_size_level, DimensionLevel, ItemStatus,
    SizeLevel,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FilterField {
    Status,
    Impact,
    Size,
    Risk,
    Complexity,
    Tag,
    PipelineType,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FilterValue {
    Status(ItemStatus),
    Dimension(DimensionLevel),
    Size(SizeLevel),
    Tag(String),
    PipelineType(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FilterCriterion {
    pub field: FilterField,
    pub values: Vec<FilterValue>,
}

impl std::fmt::Display for FilterField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            FilterField::Status => "status",
            FilterField::Impact => "impact",
            FilterField::Size => "size",
            FilterField::Risk => "risk",
            FilterField::Complexity => "complexity",
            FilterField::Tag => "tag",
            FilterField::PipelineType => "pipeline_type",
        };
        write!(f, "{}", name)
    }
}

impl std::fmt::Display for FilterValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterValue::Status(s) => match s {
                ItemStatus::New => write!(f, "new"),
                ItemStatus::Scoping => write!(f, "scoping"),
                ItemStatus::Ready => write!(f, "ready"),
                ItemStatus::InProgress => write!(f, "in_progress"),
                ItemStatus::Done => write!(f, "done"),
                ItemStatus::Blocked => write!(f, "blocked"),
            },
            FilterValue::Dimension(d) => write!(f, "{}", d),
            FilterValue::Size(s) => write!(f, "{}", s),
            FilterValue::Tag(t) => write!(f, "{}", t),
            FilterValue::PipelineType(p) => write!(f, "{}", p),
        }
    }
}

impl std::fmt::Display for FilterCriterion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let values_str: Vec<String> = self.values.iter().map(|v| v.to_string()).collect();
        write!(f, "{}={}", self.field, values_str.join(","))
    }
}

fn parse_single_value(field: &FilterField, token: &str) -> Result<FilterValue, String> {
    match field {
        FilterField::Status => {
            let status = parse_item_status(token).map_err(|_| {
                format!(
                    "Invalid value '{}' for field 'status'. Valid values: new, scoping, ready, in_progress, done, blocked",
                    token
                )
            })?;
            Ok(FilterValue::Status(status))
        }
        FilterField::Impact => {
            let level = parse_dimension_level(token).map_err(|_| {
                format!(
                    "Invalid value '{}' for field 'impact'. Valid values: low, medium, high",
                    token
                )
            })?;
            Ok(FilterValue::Dimension(level))
        }
        FilterField::Size => {
            let size = parse_size_level(token).map_err(|_| {
                format!(
                    "Invalid value '{}' for field 'size'. Valid values: small, medium, large",
                    token
                )
            })?;
            Ok(FilterValue::Size(size))
        }
        FilterField::Risk => {
            let level = parse_dimension_level(token).map_err(|_| {
                format!(
                    "Invalid value '{}' for field 'risk'. Valid values: low, medium, high",
                    token
                )
            })?;
            Ok(FilterValue::Dimension(level))
        }
        FilterField::Complexity => {
            let level = parse_dimension_level(token).map_err(|_| {
                format!(
                    "Invalid value '{}' for field 'complexity'. Valid values: low, medium, high",
                    token
                )
            })?;
            Ok(FilterValue::Dimension(level))
        }
        FilterField::Tag => Ok(FilterValue::Tag(token.to_string())),
        FilterField::PipelineType => Ok(FilterValue::PipelineType(token.to_string())),
    }
}

pub fn parse_filter(raw: &str) -> Result<FilterCriterion, String> {
    let Some((field_str, value_str)) = raw.split_once('=') else {
        return Err(format!("Filter must be in format KEY=VALUE, got: {}", raw));
    };

    let field_str = field_str.trim();
    let value_str = value_str.trim();

    if field_str.is_empty() || value_str.is_empty() {
        return Err(format!("Filter must be in format KEY=VALUE, got: {}", raw));
    }

    let field = match field_str.to_lowercase().as_str() {
        "status" => FilterField::Status,
        "impact" => FilterField::Impact,
        "size" => FilterField::Size,
        "risk" => FilterField::Risk,
        "complexity" => FilterField::Complexity,
        "tag" => FilterField::Tag,
        "pipeline_type" => FilterField::PipelineType,
        _ => {
            return Err(format!(
                "Unknown filter field: {}. Supported: status, impact, size, risk, complexity, tag, pipeline_type",
                field_str
            ));
        }
    };

    let tokens: Vec<&str> = value_str.split(',').collect();
    let mut parsed: Vec<(String, FilterValue)> = Vec::with_capacity(tokens.len());

    for token in &tokens {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return Err(format!(
                "Empty value in comma-separated list for field '{}'. Each value must be non-empty.",
                field
            ));
        }
        let value = parse_single_value(&field, trimmed)?;
        parsed.push((trimmed.to_string(), value));
    }

    let mut seen = HashSet::new();
    for (raw_token, value) in &parsed {
        if !seen.insert(value) {
            return Err(format!(
                "Duplicate value '{}' in comma-separated list for field '{}'",
                raw_token, field
            ));
        }
    }

    let values: Vec<FilterValue> = parsed.into_iter().map(|(_, v)| v).collect();

    Ok(FilterCriterion { field, values })
}

fn matches_single_value(field: &FilterField, value: &FilterValue, item: &PgItem) -> bool {
    match (field, value) {
        (FilterField::Status, FilterValue::Status(target)) => item.pg_status() == *target,
        (FilterField::Impact, FilterValue::Dimension(target)) => {
            item.impact().as_ref() == Some(target)
        }
        (FilterField::Size, FilterValue::Size(target)) => item.size().as_ref() == Some(target),
        (FilterField::Risk, FilterValue::Dimension(target)) => item.risk().as_ref() == Some(target),
        (FilterField::Complexity, FilterValue::Dimension(target)) => {
            item.complexity().as_ref() == Some(target)
        }
        (FilterField::Tag, FilterValue::Tag(target)) => item.tags().contains(target),
        (FilterField::PipelineType, FilterValue::PipelineType(target)) => {
            item.pipeline_type().as_deref() == Some(target.as_str())
        }
        // Mismatched field/value combinations should never occur with parse_filter,
        // but return false for safety.
        _ => false,
    }
}

/// OR logic: item matches if ANY value in the criterion matches.
pub fn matches_item(criterion: &FilterCriterion, item: &PgItem) -> bool {
    criterion
        .values
        .iter()
        .any(|v| matches_single_value(&criterion.field, v, item))
}

pub fn validate_filter_criteria(criteria: &[FilterCriterion]) -> Result<(), String> {
    let mut seen_scalar_fields = HashSet::new();
    let mut seen_tag_criteria = HashSet::new();

    for criterion in criteria {
        if criterion.field == FilterField::Tag {
            if !seen_tag_criteria.insert(criterion) {
                return Err(format!(
                    "Duplicate filter: {} specified multiple times",
                    criterion
                ));
            }
        } else if !seen_scalar_fields.insert(&criterion.field) {
            return Err(format!(
                "Field '{}' specified multiple times in separate --only flags. Combine values in a single flag: --only {}=value1,value2",
                criterion.field, criterion.field
            ));
        }
    }

    Ok(())
}

pub fn apply_filters(criteria: &[FilterCriterion], items: &[PgItem]) -> Vec<PgItem> {
    items
        .iter()
        .filter(|item| criteria.iter().all(|c| matches_item(c, item)))
        .cloned()
        .collect()
}

pub fn format_filter_criteria(criteria: &[FilterCriterion]) -> String {
    criteria
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(" AND ")
}
