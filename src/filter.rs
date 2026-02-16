use crate::types::{
    parse_dimension_level, parse_item_status, parse_size_level, BacklogFile, BacklogItem,
    DimensionLevel, ItemStatus, SizeLevel,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterField {
    Status,
    Impact,
    Size,
    Risk,
    Complexity,
    Tag,
    PipelineType,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterValue {
    Status(ItemStatus),
    Dimension(DimensionLevel),
    Size(SizeLevel),
    Tag(String),
    PipelineType(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterCriterion {
    pub field: FilterField,
    pub value: FilterValue,
}

impl std::fmt::Display for FilterCriterion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let field_name = match self.field {
            FilterField::Status => "status",
            FilterField::Impact => "impact",
            FilterField::Size => "size",
            FilterField::Risk => "risk",
            FilterField::Complexity => "complexity",
            FilterField::Tag => "tag",
            FilterField::PipelineType => "pipeline_type",
        };
        let value_str = match &self.value {
            FilterValue::Status(s) => match s {
                ItemStatus::New => "new".to_string(),
                ItemStatus::Scoping => "scoping".to_string(),
                ItemStatus::Ready => "ready".to_string(),
                ItemStatus::InProgress => "in_progress".to_string(),
                ItemStatus::Done => "done".to_string(),
                ItemStatus::Blocked => "blocked".to_string(),
            },
            FilterValue::Dimension(d) => d.to_string(),
            FilterValue::Size(s) => s.to_string(),
            FilterValue::Tag(t) => t.clone(),
            FilterValue::PipelineType(p) => p.clone(),
        };
        write!(f, "{}={}", field_name, value_str)
    }
}

pub fn parse_filter(raw: &str) -> Result<FilterCriterion, String> {
    let Some((field_str, value_str)) = raw.split_once('=') else {
        return Err(format!(
            "Filter must be in format KEY=VALUE, got: {}",
            raw
        ));
    };

    let field_str = field_str.trim();
    let value_str = value_str.trim();

    if field_str.is_empty() || value_str.is_empty() {
        return Err(format!(
            "Filter must be in format KEY=VALUE, got: {}",
            raw
        ));
    }

    let (field, value) = match field_str.to_lowercase().as_str() {
        "status" => {
            let status = parse_item_status(value_str).map_err(|_| {
                format!(
                    "Invalid value '{}' for field 'status'. Valid values: new, scoping, ready, in_progress, done, blocked",
                    value_str
                )
            })?;
            (FilterField::Status, FilterValue::Status(status))
        }
        "impact" => {
            let level = parse_dimension_level(value_str).map_err(|_| {
                format!(
                    "Invalid value '{}' for field 'impact'. Valid values: low, medium, high",
                    value_str
                )
            })?;
            (FilterField::Impact, FilterValue::Dimension(level))
        }
        "size" => {
            let size = parse_size_level(value_str).map_err(|_| {
                format!(
                    "Invalid value '{}' for field 'size'. Valid values: small, medium, large",
                    value_str
                )
            })?;
            (FilterField::Size, FilterValue::Size(size))
        }
        "risk" => {
            let level = parse_dimension_level(value_str).map_err(|_| {
                format!(
                    "Invalid value '{}' for field 'risk'. Valid values: low, medium, high",
                    value_str
                )
            })?;
            (FilterField::Risk, FilterValue::Dimension(level))
        }
        "complexity" => {
            let level = parse_dimension_level(value_str).map_err(|_| {
                format!(
                    "Invalid value '{}' for field 'complexity'. Valid values: low, medium, high",
                    value_str
                )
            })?;
            (FilterField::Complexity, FilterValue::Dimension(level))
        }
        "tag" => (
            FilterField::Tag,
            FilterValue::Tag(value_str.to_string()),
        ),
        "pipeline_type" => (
            FilterField::PipelineType,
            FilterValue::PipelineType(value_str.to_string()),
        ),
        _ => {
            return Err(format!(
                "Unknown filter field: {}. Supported: status, impact, size, risk, complexity, tag, pipeline_type",
                field_str
            ));
        }
    };

    Ok(FilterCriterion { field, value })
}

pub fn apply_filter(criterion: &FilterCriterion, backlog: &BacklogFile) -> BacklogFile {
    let items = backlog
        .items
        .iter()
        .filter(|item| matches_item(criterion, item))
        .cloned()
        .collect();

    BacklogFile {
        items,
        schema_version: backlog.schema_version,
        // next_item_id is carried forward for structural completeness only.
        // Filtered results are never persisted; the coordinator owns ID generation.
        next_item_id: backlog.next_item_id,
    }
}

pub fn matches_item(criterion: &FilterCriterion, item: &BacklogItem) -> bool {
    match (&criterion.field, &criterion.value) {
        (FilterField::Status, FilterValue::Status(target)) => item.status == *target,
        (FilterField::Impact, FilterValue::Dimension(target)) => {
            item.impact.as_ref() == Some(target)
        }
        (FilterField::Size, FilterValue::Size(target)) => item.size.as_ref() == Some(target),
        (FilterField::Risk, FilterValue::Dimension(target)) => {
            item.risk.as_ref() == Some(target)
        }
        (FilterField::Complexity, FilterValue::Dimension(target)) => {
            item.complexity.as_ref() == Some(target)
        }
        (FilterField::Tag, FilterValue::Tag(target)) => item.tags.contains(target),
        (FilterField::PipelineType, FilterValue::PipelineType(target)) => {
            item.pipeline_type.as_deref() == Some(target.as_str())
        }
        // Mismatched field/value combinations should never occur with parse_filter,
        // but return false for safety.
        _ => false,
    }
}
