use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use task_golem::model::item::Item;
use task_golem::model::status::Status;

use crate::types::{
    BlockType, DimensionLevel, ItemStatus, ItemUpdate, PhasePool, SizeLevel, StructuredDescription,
    UpdatedAssessments,
};

// --- Extension key constants ---

pub const X_PG_STATUS: &str = "x-pg-status";
pub const X_PG_PHASE: &str = "x-pg-phase";
pub const X_PG_PHASE_POOL: &str = "x-pg-phase-pool";
pub const X_PG_SIZE: &str = "x-pg-size";
pub const X_PG_COMPLEXITY: &str = "x-pg-complexity";
pub const X_PG_RISK: &str = "x-pg-risk";
pub const X_PG_IMPACT: &str = "x-pg-impact";
pub const X_PG_REQUIRES_HUMAN_REVIEW: &str = "x-pg-requires-human-review";
pub const X_PG_PIPELINE_TYPE: &str = "x-pg-pipeline-type";
pub const X_PG_ORIGIN: &str = "x-pg-origin";
pub const X_PG_BLOCKED_TYPE: &str = "x-pg-blocked-type";
pub const X_PG_BLOCKED_FROM_STATUS: &str = "x-pg-blocked-from-status";
pub const X_PG_UNBLOCK_CONTEXT: &str = "x-pg-unblock-context";
pub const X_PG_LAST_PHASE_COMMIT: &str = "x-pg-last-phase-commit";
pub const X_PG_DESCRIPTION: &str = "x-pg-description";

// --- PgItem newtype ---

/// Newtype wrapper over task-golem's `Item`, providing typed access
/// to phase-golem-specific `x-pg-*` extension fields and bidirectional
/// status mapping between phase-golem's 6-state `ItemStatus` and
/// task-golem's 4-state `Status`.
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
    /// Bidirectional status mapping: reads task-golem `Status` and `x-pg-status`
    /// extension to produce phase-golem's `ItemStatus`.
    ///
    /// - `Todo` + `x-pg-status` absent -> `New` (default)
    /// - `Todo` + `x-pg-status: "new"` -> `New`
    /// - `Todo` + `x-pg-status: "scoping"` -> `Scoping`
    /// - `Todo` + `x-pg-status: "ready"` -> `Ready`
    /// - `Doing` -> `InProgress` (ignores stale `x-pg-status`)
    /// - `Done` -> `Done` (ignores stale `x-pg-status`)
    /// - `Blocked` -> `Blocked` (ignores stale `x-pg-status`)
    pub fn pg_status(&self) -> ItemStatus {
        match self.0.status {
            Status::Todo => {
                match self.get_string_ext(X_PG_STATUS) {
                    Some(s) => match s.as_str() {
                        "new" => ItemStatus::New,
                        "scoping" => ItemStatus::Scoping,
                        "ready" => ItemStatus::Ready,
                        other => {
                            crate::log_warn!(
                                "Item {}: invalid x-pg-status value '{}', defaulting to New",
                                self.0.id,
                                other
                            );
                            ItemStatus::New
                        }
                    },
                    // Absent x-pg-status on Todo defaults to New
                    None => ItemStatus::New,
                }
            }
            Status::Doing => ItemStatus::InProgress,
            Status::Done => ItemStatus::Done,
            Status::Blocked => ItemStatus::Blocked,
        }
    }

    pub fn phase(&self) -> Option<String> {
        self.get_string_ext(X_PG_PHASE)
    }

    pub fn phase_pool(&self) -> Option<PhasePool> {
        self.get_string_ext(X_PG_PHASE_POOL).and_then(|s| {
            match s.as_str() {
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
            }
        })
    }

    pub fn size(&self) -> Option<SizeLevel> {
        self.get_string_ext(X_PG_SIZE).and_then(|s| {
            match s.as_str() {
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

    /// Returns `true` if `x-pg-requires-human-review` is `true`; absent defaults to `false`.
    pub fn requires_human_review(&self) -> bool {
        self.0
            .extensions
            .get(X_PG_REQUIRES_HUMAN_REVIEW)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    pub fn pipeline_type(&self) -> Option<String> {
        self.get_string_ext(X_PG_PIPELINE_TYPE)
    }

    pub fn origin(&self) -> Option<String> {
        self.get_string_ext(X_PG_ORIGIN)
    }

    pub fn blocked_type(&self) -> Option<BlockType> {
        self.get_string_ext(X_PG_BLOCKED_TYPE).and_then(|s| {
            match s.as_str() {
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
            }
        })
    }

    /// Returns the authoritative `blocked_from_status` from the `x-pg-blocked-from-status`
    /// extension. Detects divergence: if native `blocked_from_status` is `None` but the
    /// extension is present, the extension is stale (e.g., after `tg unblock`) -- returns
    /// `None` with a warning.
    pub fn pg_blocked_from_status(&self) -> Option<ItemStatus> {
        let has_native = self.0.blocked_from_status.is_some();
        let ext_value = self.get_string_ext(X_PG_BLOCKED_FROM_STATUS);

        match (has_native, ext_value) {
            // Normal case: both present, use extension as authoritative
            (true, Some(s)) => parse_blocked_from_status(&self.0.id, &s),
            // Extension present but native cleared (tg unblock ran): stale
            (false, Some(_)) => {
                crate::log_warn!(
                    "Item {}: x-pg-blocked-from-status extension is stale (native field cleared), treating as absent",
                    self.0.id,
                );
                None
            }
            // No extension: item was not blocked via adapter, or extension was never set
            (true, None) => None,
            (false, None) => None,
        }
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
        self.get_string_ext(key).and_then(|s| {
            match s.as_str() {
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
            }
        })
    }
}

// --- Free functions for mutation (operate on &mut Item directly) ---

/// Sets both the task-golem native `Status` and the `x-pg-status` extension field.
///
/// For `InProgress`, `Done`, `Blocked`: sets the native status directly and clears
/// the `x-pg-status` extension (it is only meaningful for `Todo` sub-states).
///
/// For `New`, `Scoping`, `Ready`: sets native status to `Todo` and writes the
/// sub-state string to `x-pg-status`.
pub fn set_pg_status(item: &mut Item, status: ItemStatus) {
    let now = Utc::now();
    match status {
        ItemStatus::New => {
            item.status = Status::Todo;
            item.extensions
                .insert(X_PG_STATUS.to_string(), serde_json::json!("new"));
        }
        ItemStatus::Scoping => {
            item.status = Status::Todo;
            item.extensions
                .insert(X_PG_STATUS.to_string(), serde_json::json!("scoping"));
        }
        ItemStatus::Ready => {
            item.status = Status::Todo;
            item.extensions
                .insert(X_PG_STATUS.to_string(), serde_json::json!("ready"));
        }
        ItemStatus::InProgress => {
            item.status = Status::Doing;
            item.extensions.remove(X_PG_STATUS);
        }
        ItemStatus::Done => {
            item.status = Status::Done;
            item.extensions.remove(X_PG_STATUS);
        }
        ItemStatus::Blocked => {
            item.status = Status::Blocked;
            item.extensions.remove(X_PG_STATUS);
        }
    }
    item.updated_at = now;
}

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
    set_enum_ext(item, X_PG_SIZE, size.map(|s| match s {
        SizeLevel::Small => "small",
        SizeLevel::Medium => "medium",
        SizeLevel::Large => "large",
    }));
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
    set_enum_ext(item, X_PG_BLOCKED_TYPE, block_type.map(|b| match b {
        BlockType::Clarification => "clarification",
        BlockType::Decision => "decision",
    }));
}

/// Sets the `x-pg-blocked-from-status` extension field and the native
/// `blocked_from_status` field. The extension stores the full-fidelity 6-variant
/// `ItemStatus`; the native field stores a lossy 4-variant `Status` mapping.
/// Pass `None` to clear both.
pub fn set_blocked_from_status(item: &mut Item, status: Option<&ItemStatus>) {
    set_enum_ext(item, X_PG_BLOCKED_FROM_STATUS, status.map(|s| match s {
        ItemStatus::New => "new",
        ItemStatus::Scoping => "scoping",
        ItemStatus::Ready => "ready",
        ItemStatus::InProgress => "in_progress",
        ItemStatus::Done => "done",
        ItemStatus::Blocked => "blocked",
    }));
    // Keep native blocked_from_status in sync (lossy: New/Scoping/Ready -> Todo)
    item.blocked_from_status = status.map(|s| match s {
        ItemStatus::New | ItemStatus::Scoping | ItemStatus::Ready => Status::Todo,
        ItemStatus::InProgress => Status::Doing,
        ItemStatus::Done => Status::Done,
        ItemStatus::Blocked => Status::Blocked,
    });
}

/// Sets the `x-pg-unblock-context` extension field. Pass `None` to clear.
pub fn set_unblock_context(item: &mut Item, context: Option<&str>) {
    set_enum_ext(item, X_PG_UNBLOCK_CONTEXT, context);
}

/// Sets the `x-pg-requires-human-review` extension field.
pub fn set_requires_human_review(item: &mut Item, value: bool) {
    if value {
        item.extensions
            .insert(X_PG_REQUIRES_HUMAN_REVIEW.to_string(), serde_json::json!(true));
    } else {
        item.extensions.remove(X_PG_REQUIRES_HUMAN_REVIEW);
    }
    item.updated_at = Utc::now();
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
            let value = serde_json::to_value(d).expect("StructuredDescription is always serializable");
            item.extensions
                .insert(X_PG_DESCRIPTION.to_string(), value);
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

/// Dispatches an `ItemUpdate` variant to the appropriate field mutation.
///
/// This is the central mutation dispatch used by the coordinator's `UpdateItem`
/// handler. Operates on `&mut Item` directly to avoid owned-vs-borrow tension
/// in `with_lock` closures.
pub fn apply_update(item: &mut Item, update: ItemUpdate) {
    match update {
        ItemUpdate::TransitionStatus(new_status) => {
            let pg = PgItem(item.clone());
            let current = pg.pg_status();

            if !current.is_valid_transition(&new_status) {
                crate::log_warn!(
                    "Item {}: invalid transition {:?} -> {:?}, skipping",
                    item.id,
                    current,
                    new_status
                );
                return;
            }

            // When transitioning to Blocked, save the current status
            if new_status == ItemStatus::Blocked {
                set_blocked_from_status(item, Some(&current));
            }

            // When transitioning from Blocked, clear blocked fields
            if current == ItemStatus::Blocked {
                set_blocked_from_status(item, None);
                item.blocked_reason = None;
                set_blocked_type(item, None);
                set_unblock_context(item, None);
            }

            set_pg_status(item, new_status);
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
        ItemUpdate::SetBlocked(reason) => {
            let pg = PgItem(item.clone());
            let current = pg.pg_status();

            if !current.is_valid_transition(&ItemStatus::Blocked) {
                crate::log_warn!(
                    "Item {}: cannot block from {:?}, skipping",
                    item.id,
                    current
                );
                return;
            }

            set_blocked_from_status(item, Some(&current));
            set_pg_status(item, ItemStatus::Blocked);
            item.blocked_reason = Some(reason);
        }
        ItemUpdate::Unblock => {
            let pg = PgItem(item.clone());
            if pg.pg_status() != ItemStatus::Blocked {
                crate::log_warn!(
                    "Item {}: cannot unblock, not blocked (status: {:?}), skipping",
                    item.id,
                    pg.pg_status()
                );
                return;
            }

            // Read the blocked_from_status before clearing it
            let restore_to = pg.pg_blocked_from_status().unwrap_or(ItemStatus::New);

            // Clear all blocked fields (extension and native)
            set_blocked_from_status(item, None);
            item.blocked_reason = None;
            item.blocked_from_status = None;
            set_blocked_type(item, None);
            set_unblock_context(item, None);

            // Restore to the saved status
            set_pg_status(item, restore_to);
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

/// Constructs a new `PgItem` from parts with correct extension defaults.
///
/// Sets: `created_at`/`updated_at` = `Utc::now()`, `priority` = 0,
/// status = `Todo`, `x-pg-status` = `"new"`, `claimed_by`/`claimed_at` = `None`.
pub fn new_from_parts(
    id: String,
    title: String,
    status: ItemStatus,
    dependencies: Vec<String>,
    tags: Vec<String>,
) -> PgItem {
    let now = Utc::now();
    let mut extensions = BTreeMap::new();

    // Set initial x-pg-status based on the provided ItemStatus
    let native_status = match &status {
        ItemStatus::New => {
            extensions.insert(X_PG_STATUS.to_string(), serde_json::json!("new"));
            Status::Todo
        }
        ItemStatus::Scoping => {
            extensions.insert(X_PG_STATUS.to_string(), serde_json::json!("scoping"));
            Status::Todo
        }
        ItemStatus::Ready => {
            extensions.insert(X_PG_STATUS.to_string(), serde_json::json!("ready"));
            Status::Todo
        }
        ItemStatus::InProgress => Status::Doing,
        ItemStatus::Done => Status::Done,
        ItemStatus::Blocked => Status::Blocked,
    };

    let item = Item {
        id,
        title,
        status: native_status,
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
    set_enum_ext(item, key, level.map(|l| match l {
        DimensionLevel::Low => "low",
        DimensionLevel::Medium => "medium",
        DimensionLevel::High => "high",
    }));
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

fn parse_blocked_from_status(item_id: &str, s: &str) -> Option<ItemStatus> {
    match s {
        "new" => Some(ItemStatus::New),
        "scoping" => Some(ItemStatus::Scoping),
        "ready" => Some(ItemStatus::Ready),
        "in_progress" => Some(ItemStatus::InProgress),
        other => {
            crate::log_warn!(
                "Item {}: invalid x-pg-blocked-from-status value '{}', treating as absent",
                item_id,
                other
            );
            None
        }
    }
}
