use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use task_golem::model::item::Item;
use task_golem::model::status::Status;

use crate::config::{PublicExecutionPolicy, VerificationPlan};
use crate::types::{
    BlockType, DimensionLevel, ItemUpdate, PhasePool, SizeLevel, StructuredDescription,
    UpdatedAssessments,
};

// --- Extension key constants ---

pub const X_PG_PHASE: &str = "x-pg-phase";
pub const X_PG_PHASE_POOL: &str = "x-pg-phase-pool";
pub const X_PG_SIZE: &str = "x-pg-size";
pub const X_PG_COMPLEXITY: &str = "x-pg-complexity";
pub const X_PG_RISK: &str = "x-pg-risk";
pub const X_PG_IMPACT: &str = "x-pg-impact";
pub const X_PG_PIPELINE_TYPE: &str = "x-pg-pipeline-type";
pub const X_PG_ORIGIN: &str = "x-pg-origin";
pub const X_PG_BLOCKED_TYPE: &str = "x-pg-blocked-type";
pub const X_PG_UNBLOCK_CONTEXT: &str = "x-pg-unblock-context";
pub const X_PG_LAST_PHASE_COMMIT: &str = "x-pg-last-phase-commit";
pub const X_PG_DESCRIPTION: &str = "x-pg-description";
pub const X_PG_OWNER: &str = "x-pg-owner";
pub const X_PG_HUMAN_DECISION: &str = "x-pg-human-decision";
pub const X_PG_TEMPLATE_NODE_KEY: &str = "x-pg-template-node-key";
pub const X_PG_EXECUTOR_PROFILE: &str = "x-pg-executor-profile";
pub const X_PG_EXECUTION_POLICY: &str = "x-pg-execution-policy";
pub const X_PG_VERIFICATION: &str = "x-pg-verification";

// --- PgItem newtype ---

/// Newtype wrapper over task-golem's `Item` with typed PG metadata access.
#[derive(Debug, Clone, PartialEq)]
pub struct PgItem(pub Item);

// --- Native field delegates ---

impl PgItem {
    pub fn id(&self) -> &str {
        &self.0.id
    }

    pub fn title(&self) -> &str {
        &self.0.title
    }

    /// Returns the task-golem native `Status`.
    pub fn status(&self) -> Status {
        self.0.status
    }

    pub fn dependencies(&self) -> &[String] {
        &self.0.dependencies
    }

    pub fn tags(&self) -> &[String] {
        &self.0.tags
    }

    pub fn blocked_reason(&self) -> Option<&str> {
        self.0.blocked_reason.as_deref()
    }

    pub fn created_at(&self) -> DateTime<Utc> {
        self.0.created_at
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.0.updated_at
    }
}

// --- Extension field typed getters ---

impl PgItem {
    pub fn phase(&self) -> Option<String> {
        self.get_string_ext(X_PG_PHASE)
    }

    pub fn phase_pool(&self) -> Option<PhasePool> {
        self.get_string_ext(X_PG_PHASE_POOL)
            .and_then(|s| match s.as_str() {
                "pre" => Some(PhasePool::Pre),
                "main" => Some(PhasePool::Main),
                other => {
                    crate::log_warn!(
                        "Item {}: invalid x-pg-phase-pool value '{}', treating as absent",
                        self.0.id,
                        other
                    );
                    None
                }
            })
    }

    pub fn size(&self) -> Option<SizeLevel> {
        self.get_string_ext(X_PG_SIZE)
            .and_then(|s| match s.as_str() {
                "small" => Some(SizeLevel::Small),
                "medium" => Some(SizeLevel::Medium),
                "large" => Some(SizeLevel::Large),
                other => {
                    crate::log_warn!(
                        "Item {}: invalid x-pg-size value '{}', treating as absent",
                        self.0.id,
                        other
                    );
                    None
                }
            })
    }

    pub fn complexity(&self) -> Option<DimensionLevel> {
        self.get_dimension_ext(X_PG_COMPLEXITY)
    }

    pub fn risk(&self) -> Option<DimensionLevel> {
        self.get_dimension_ext(X_PG_RISK)
    }

    pub fn impact(&self) -> Option<DimensionLevel> {
        self.get_dimension_ext(X_PG_IMPACT)
    }

    pub fn pipeline_type(&self) -> Option<String> {
        self.get_string_ext(X_PG_PIPELINE_TYPE)
    }

    pub fn origin(&self) -> Option<String> {
        self.get_string_ext(X_PG_ORIGIN)
    }

    pub fn blocked_type(&self) -> Option<BlockType> {
        self.get_string_ext(X_PG_BLOCKED_TYPE)
            .and_then(|s| match s.as_str() {
                "clarification" => Some(BlockType::Clarification),
                "decision" => Some(BlockType::Decision),
                other => {
                    crate::log_warn!(
                        "Item {}: invalid x-pg-blocked-type value '{}', treating as absent",
                        self.0.id,
                        other
                    );
                    None
                }
            })
    }

    pub fn is_pg_owned(&self) -> bool {
        self.0
            .extensions
            .get(X_PG_OWNER)
            .and_then(|value| value.as_str())
            == Some("phase-golem")
    }

    pub fn is_claimed_for_pg_execution(&self) -> bool {
        self.is_pg_owned()
            && self.status() == Status::Doing
            && self.0.claimed_by.as_deref() == Some("phase-golem")
            && self.0.claimed_at.is_some()
    }

    pub fn is_human_gate(&self) -> bool {
        self.is_pg_owned()
            && self
                .0
                .extensions
                .get(X_PG_HUMAN_DECISION)
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
    }

    pub fn template_node_key(&self) -> Option<String> {
        self.get_string_ext(X_PG_TEMPLATE_NODE_KEY)
    }

    pub fn executor_profile_snapshot(&self) -> Result<String, String> {
        let profile = self
            .get_string_ext(X_PG_EXECUTOR_PROFILE)
            .ok_or_else(|| format!("Item {} has no executor profile snapshot", self.id()))?;
        if profile.trim().is_empty() {
            return Err(format!(
                "Item {} has an empty executor profile snapshot",
                self.id()
            ));
        }
        Ok(profile)
    }

    pub fn execution_policy_snapshot(&self) -> Result<PublicExecutionPolicy, String> {
        self.deserialize_snapshot(X_PG_EXECUTION_POLICY, "execution policy")
    }

    pub fn verification_snapshot(&self) -> Result<VerificationPlan, String> {
        self.deserialize_snapshot(X_PG_VERIFICATION, "verification plan")
    }

    pub fn unblock_context(&self) -> Option<String> {
        self.get_string_ext(X_PG_UNBLOCK_CONTEXT)
    }

    pub fn last_phase_commit(&self) -> Option<String> {
        self.get_string_ext(X_PG_LAST_PHASE_COMMIT)
    }

    /// Deserializes `x-pg-description` JSON object into `StructuredDescription`.
    /// Returns `None` with a warning on deserialization failure.
    pub fn structured_description(&self) -> Option<StructuredDescription> {
        let value = self.0.extensions.get(X_PG_DESCRIPTION)?;
        match serde_json::from_value::<StructuredDescription>(value.clone()) {
            Ok(desc) if !desc.is_empty() => Some(desc),
            Ok(_) => None,
            Err(e) => {
                crate::log_warn!(
                    "Item {}: failed to deserialize x-pg-description: {}, treating as absent",
                    self.0.id,
                    e
                );
                None
            }
        }
    }

    // --- Private helpers ---

    fn get_string_ext(&self, key: &str) -> Option<String> {
        self.0
            .extensions
            .get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    fn get_dimension_ext(&self, key: &str) -> Option<DimensionLevel> {
        self.get_string_ext(key).and_then(|s| match s.as_str() {
            "low" => Some(DimensionLevel::Low),
            "medium" => Some(DimensionLevel::Medium),
            "high" => Some(DimensionLevel::High),
            other => {
                crate::log_warn!(
                    "Item {}: invalid {} value '{}', treating as absent",
                    self.0.id,
                    key,
                    other
                );
                None
            }
        })
    }

    fn deserialize_snapshot<T: DeserializeOwned>(
        &self,
        key: &str,
        label: &str,
    ) -> Result<T, String> {
        let value = self
            .0
            .extensions
            .get(key)
            .ok_or_else(|| format!("Item {} has no {label} snapshot", self.id()))?;
        serde_json::from_value(value.clone()).map_err(|error| {
            format!(
                "Item {} has an invalid {label} snapshot: {error}",
                self.id()
            )
        })
    }
}

// --- Free functions for mutation (operate on &mut Item directly) ---

/// Sets the `x-pg-phase` extension field. Pass `None` to clear.
pub fn set_phase(item: &mut Item, phase: Option<&str>) {
    match phase {
        Some(p) => {
            item.extensions
                .insert(X_PG_PHASE.to_string(), serde_json::json!(p));
        }
        None => {
            item.extensions.remove(X_PG_PHASE);
        }
    }
    item.updated_at = Utc::now();
}

/// Sets the `x-pg-phase-pool` extension field. Pass `None` to clear.
pub fn set_phase_pool(item: &mut Item, pool: Option<&PhasePool>) {
    match pool {
        Some(p) => {
            let value = match p {
                PhasePool::Pre => "pre",
                PhasePool::Main => "main",
            };
            item.extensions
                .insert(X_PG_PHASE_POOL.to_string(), serde_json::json!(value));
        }
        None => {
            item.extensions.remove(X_PG_PHASE_POOL);
        }
    }
    item.updated_at = Utc::now();
}

/// Sets the `x-pg-size` extension field. Pass `None` to clear.
pub fn set_size(item: &mut Item, size: Option<&SizeLevel>) {
    set_enum_ext(
        item,
        X_PG_SIZE,
        size.map(|s| match s {
            SizeLevel::Small => "small",
            SizeLevel::Medium => "medium",
            SizeLevel::Large => "large",
        }),
    );
}

/// Sets the `x-pg-complexity` extension field. Pass `None` to clear.
pub fn set_complexity(item: &mut Item, level: Option<&DimensionLevel>) {
    set_dimension_ext(item, X_PG_COMPLEXITY, level);
}

/// Sets the `x-pg-risk` extension field. Pass `None` to clear.
pub fn set_risk(item: &mut Item, level: Option<&DimensionLevel>) {
    set_dimension_ext(item, X_PG_RISK, level);
}

/// Sets the `x-pg-impact` extension field. Pass `None` to clear.
pub fn set_impact(item: &mut Item, level: Option<&DimensionLevel>) {
    set_dimension_ext(item, X_PG_IMPACT, level);
}

/// Sets the `x-pg-pipeline-type` extension field. Pass `None` to clear.
pub fn set_pipeline_type(item: &mut Item, pipeline_type: Option<&str>) {
    set_enum_ext(item, X_PG_PIPELINE_TYPE, pipeline_type);
}

/// Sets the `x-pg-last-phase-commit` extension field. Pass `None` to clear.
pub fn set_last_phase_commit(item: &mut Item, sha: Option<&str>) {
    set_enum_ext(item, X_PG_LAST_PHASE_COMMIT, sha);
}

/// Sets the `x-pg-blocked-type` extension field. Pass `None` to clear.
pub fn set_blocked_type(item: &mut Item, block_type: Option<&BlockType>) {
    set_enum_ext(
        item,
        X_PG_BLOCKED_TYPE,
        block_type.map(|b| match b {
            BlockType::Clarification => "clarification",
            BlockType::Decision => "decision",
        }),
    );
}

/// Sets the `x-pg-unblock-context` extension field. Pass `None` to clear.
pub fn set_unblock_context(item: &mut Item, context: Option<&str>) {
    set_enum_ext(item, X_PG_UNBLOCK_CONTEXT, context);
}

/// Sets the `x-pg-origin` extension field. Pass `None` to clear.
pub fn set_origin(item: &mut Item, origin: Option<&str>) {
    set_enum_ext(item, X_PG_ORIGIN, origin);
}

/// Sets the `x-pg-description` extension field and also populates
/// `Item.description` with the `context` field for `tg show` readability.
pub fn set_structured_description(item: &mut Item, desc: Option<&StructuredDescription>) {
    match desc {
        Some(d) => {
            let value =
                serde_json::to_value(d).expect("StructuredDescription is always serializable");
            item.extensions.insert(X_PG_DESCRIPTION.to_string(), value);
            // Populate native description with context field for tg show
            if d.context.is_empty() {
                item.description = None;
            } else {
                item.description = Some(d.context.clone());
            }
        }
        None => {
            item.extensions.remove(X_PG_DESCRIPTION);
            item.description = None;
        }
    }
    item.updated_at = Utc::now();
}

/// Applies a PG metadata update. Lifecycle variants are owned by the coordinator.
pub fn apply_metadata_update(item: &mut Item, update: ItemUpdate) {
    match update {
        ItemUpdate::TransitionStatus(_) | ItemUpdate::SetBlocked(_) | ItemUpdate::Unblock => {
            unreachable!("lifecycle updates must be applied by the coordinator")
        }
        ItemUpdate::SetPhase(phase) => {
            set_phase(item, Some(&phase));
        }
        ItemUpdate::SetPhasePool(pool) => {
            set_phase_pool(item, Some(&pool));
        }
        ItemUpdate::ClearPhase => {
            set_phase(item, None);
            set_phase_pool(item, None);
        }
        ItemUpdate::UpdateAssessments(assessments) => {
            apply_assessments(item, &assessments);
        }
        ItemUpdate::SetPipelineType(pipeline_type) => {
            set_pipeline_type(item, Some(&pipeline_type));
        }
        ItemUpdate::SetLastPhaseCommit(sha) => {
            set_last_phase_commit(item, Some(&sha));
        }
        ItemUpdate::SetDescription(description) => {
            set_structured_description(item, Some(&description));
        }
    }
}

/// Constructs a new `PgItem` from generic TG fields and PG metadata defaults.
pub fn new_from_parts(
    id: String,
    title: String,
    status: Status,
    dependencies: Vec<String>,
    tags: Vec<String>,
) -> PgItem {
    let now = Utc::now();
    let extensions = BTreeMap::from([(X_PG_OWNER.to_string(), serde_json::json!("phase-golem"))]);

    let item = Item {
        id,
        title,
        status,
        priority: 0,
        description: None,
        tags,
        dependencies,
        created_at: now,
        updated_at: now,
        blocked_reason: None,
        blocked_from_status: None,
        claimed_by: None,
        claimed_at: None,
        parent: None,
        extensions,
    };

    PgItem(item)
}

// --- Private helpers ---

fn set_enum_ext(item: &mut Item, key: &str, value: Option<&str>) {
    match value {
        Some(v) => {
            item.extensions
                .insert(key.to_string(), serde_json::json!(v));
        }
        None => {
            item.extensions.remove(key);
        }
    }
    item.updated_at = Utc::now();
}

fn set_dimension_ext(item: &mut Item, key: &str, level: Option<&DimensionLevel>) {
    set_enum_ext(
        item,
        key,
        level.map(|l| match l {
            DimensionLevel::Low => "low",
            DimensionLevel::Medium => "medium",
            DimensionLevel::High => "high",
        }),
    );
}

fn apply_assessments(item: &mut Item, assessments: &UpdatedAssessments) {
    if let Some(ref size) = assessments.size {
        set_size(item, Some(size));
    }
    if let Some(ref complexity) = assessments.complexity {
        set_complexity(item, Some(complexity));
    }
    if let Some(ref risk) = assessments.risk {
        set_risk(item, Some(risk));
    }
    if let Some(ref impact) = assessments.impact {
        set_impact(item, Some(impact));
    }
    item.updated_at = Utc::now();
}
