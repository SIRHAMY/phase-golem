use serde::{Deserialize, Deserializer, Serialize};

// --- Enums ---

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    #[default]
    New,
    Scoping,
    Ready,
    InProgress,
    Done,
    Blocked,
}

impl ItemStatus {
    /// Validates whether a transition from this status to `to` is allowed.
    ///
    /// Rules:
    /// - Any non-terminal, non-blocked status can transition to Blocked
    /// - Blocked can return to any non-terminal status (unblock)
    /// - Forward progression: New -> Scoping -> Ready -> InProgress -> Done
    /// - Done is terminal — items cannot leave Done
    pub fn is_valid_transition(&self, to: &ItemStatus) -> bool {
        use ItemStatus::*;

        // Any non-terminal, non-blocked status can transition to Blocked
        if *to == Blocked && *self != Done && *self != Blocked {
            return true;
        }

        // Blocked can return to any non-terminal status
        if *self == Blocked && *to != Done && *to != Blocked {
            return true;
        }

        // Forward progression transitions
        matches!(
            (self, to),
            (New, Scoping) | (Scoping, Ready) | (Ready, InProgress) | (InProgress, Done)
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResultCode {
    SubphaseComplete,
    PhaseComplete,
    Failed,
    Blocked,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlockType {
    Clarification,
    Decision,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SizeLevel {
    Small,
    Medium,
    Large,
}

impl std::fmt::Display for SizeLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SizeLevel::Small => write!(f, "small"),
            SizeLevel::Medium => write!(f, "medium"),
            SizeLevel::Large => write!(f, "large"),
        }
    }
}

pub fn parse_size_level(s: &str) -> Result<SizeLevel, String> {
    match s.to_lowercase().as_str() {
        "small" | "s" => Ok(SizeLevel::Small),
        "medium" | "m" => Ok(SizeLevel::Medium),
        "large" | "l" => Ok(SizeLevel::Large),
        _ => Err(format!(
            "Invalid size '{}': expected small, medium, or large",
            s
        )),
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DimensionLevel {
    Low,
    Medium,
    High,
}

impl std::fmt::Display for DimensionLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DimensionLevel::Low => write!(f, "low"),
            DimensionLevel::Medium => write!(f, "medium"),
            DimensionLevel::High => write!(f, "high"),
        }
    }
}

pub fn parse_dimension_level(s: &str) -> Result<DimensionLevel, String> {
    match s.to_lowercase().as_str() {
        "low" | "l" => Ok(DimensionLevel::Low),
        "medium" | "m" => Ok(DimensionLevel::Medium),
        "high" | "h" => Ok(DimensionLevel::High),
        _ => Err(format!(
            "Invalid level '{}': expected low, medium, or high",
            s
        )),
    }
}

pub fn parse_item_status(s: &str) -> Result<ItemStatus, String> {
    match s.to_lowercase().as_str() {
        "new" => Ok(ItemStatus::New),
        "scoping" => Ok(ItemStatus::Scoping),
        "ready" => Ok(ItemStatus::Ready),
        "in_progress" => Ok(ItemStatus::InProgress),
        "done" => Ok(ItemStatus::Done),
        "blocked" => Ok(ItemStatus::Blocked),
        _ => Err(format!(
            "Invalid status '{}': expected new, scoping, ready, in_progress, done, or blocked",
            s
        )),
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhasePool {
    Pre,
    Main,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ItemUpdate {
    TransitionStatus(ItemStatus),
    SetPhase(String),
    SetPhasePool(PhasePool),
    ClearPhase,
    SetBlocked(String),
    Unblock,
    UpdateAssessments(UpdatedAssessments),
    SetPipelineType(String),
    SetLastPhaseCommit(String),
    SetDescription(StructuredDescription),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PhaseExecutionResult {
    Success(PhaseResult),
    SubphaseComplete(PhaseResult),
    Failed(String),
    Blocked(String),
    Cancelled,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerAction {
    Triage(String),
    Promote(String),
    RunPhase {
        item_id: String,
        phase: String,
        phase_pool: PhasePool,
        is_destructive: bool,
    },
}

// --- Structs ---

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct BacklogItem {
    pub id: String,
    pub title: String,
    pub status: ItemStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<SizeLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity: Option<DimensionLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<DimensionLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impact: Option<DimensionLevel>,
    #[serde(default)]
    pub requires_human_review: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_from_status: Option<ItemStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_type: Option<BlockType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unblock_context: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
    pub created: String,
    pub updated: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<StructuredDescription>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase_pool: Option<PhasePool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_phase_commit: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct BacklogFile {
    pub schema_version: u32,
    #[serde(default)]
    pub items: Vec<BacklogItem>,
    /// Highest numeric suffix ever assigned for ID generation.
    /// Used as a floor in generate_next_id() to prevent ID reuse after archival.
    /// Formula: next_id = max(current_items_max, next_item_id) + 1
    #[serde(default)]
    pub next_item_id: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PhaseResult {
    pub item_id: String,
    pub phase: String,
    pub result: ResultCode,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_assessments: Option<UpdatedAssessments>,
    #[serde(default)]
    pub follow_ups: Vec<FollowUp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_summary: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub duplicates: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StructuredDescription {
    #[serde(default)]
    pub context: String,
    #[serde(default)]
    pub problem: String,
    #[serde(default)]
    pub solution: String,
    #[serde(default)]
    pub impact: String,
    #[serde(default)]
    pub sizing_rationale: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FollowUp {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_size: Option<SizeLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_risk: Option<DimensionLevel>,
}

/// Accepts both a plain string (title only) and a full object.
/// This makes deserialization resilient to agents that output
/// `"follow_ups": ["some title"]` instead of the structured format.
impl<'de> Deserialize<'de> for FollowUp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum FollowUpRaw {
            String(String),
            Struct {
                title: String,
                #[serde(default)]
                context: Option<String>,
                #[serde(default)]
                suggested_size: Option<SizeLevel>,
                #[serde(default)]
                suggested_risk: Option<DimensionLevel>,
            },
        }

        match FollowUpRaw::deserialize(deserializer)? {
            FollowUpRaw::String(title) => Ok(FollowUp {
                title,
                context: None,
                suggested_size: None,
                suggested_risk: None,
            }),
            FollowUpRaw::Struct {
                title,
                context,
                suggested_size,
                suggested_risk,
            } => Ok(FollowUp {
                title,
                context,
                suggested_size,
                suggested_risk,
            }),
        }
    }
}

/// Simplified input schema for human-written inbox items.
/// Deserialized from BACKLOG_INBOX.yaml.
#[derive(Debug, Clone, Deserialize)]
pub struct InboxItem {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub size: Option<SizeLevel>,
    #[serde(default)]
    pub risk: Option<DimensionLevel>,
    #[serde(default)]
    pub impact: Option<DimensionLevel>,
    #[serde(default)]
    pub pipeline_type: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UpdatedAssessments {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<SizeLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub complexity: Option<DimensionLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<DimensionLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impact: Option<DimensionLevel>,
}
