use serde::{Deserialize, Deserializer, Serialize};
use task_golem::events::{self, Event};
use task_golem::model::status::Status;

use crate::config::PublicExecutionPolicy;

// --- Enums ---

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
pub enum TrustedResultCode {
    Complete,
    Blocked,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TrustedExecutorResult {
    pub item_id: String,
    pub phase: String,
    pub result: TrustedResultCode,
    pub summary: String,
    pub evidence_references: Vec<String>,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttemptOutcome {
    Complete,
    Blocked,
    Rejected,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct SupervisedAttempt {
    pub schema: String,
    pub item_id: String,
    pub phase: String,
    pub executor_profile: String,
    pub attempt: u32,
    pub max_attempts: u32,
    pub public_policy: PublicExecutionPolicy,
    pub outcome: AttemptOutcome,
    pub summary: String,
    pub executor_evidence: Vec<String>,
    pub verification_evidence: Vec<String>,
}

pub const MAX_ATTEMPT_NOTE_TEXT_BYTES: usize = 1536;
const ATTEMPT_NOTE_PREFIX: &str = "phase-golem-attempt ";

impl SupervisedAttempt {
    pub fn validated_note_text(&self) -> Result<String, String> {
        let evidence = serde_json::to_string(self)
            .map_err(|error| format!("Cannot serialize PG attempt evidence: {error}"))?;
        let note = format!("{ATTEMPT_NOTE_PREFIX}{evidence}");
        if note.len() > MAX_ATTEMPT_NOTE_TEXT_BYTES {
            return Err(format!(
                "PG attempt evidence is {} bytes, exceeding the {}-byte protocol budget",
                note.len(),
                MAX_ATTEMPT_NOTE_TEXT_BYTES
            ));
        }

        let event = Event::note(&self.item_id, events::author::resolve(), &note);
        let event_line_bytes = serde_json::to_string(&event)
            .expect("TG Event serialization must succeed")
            .len()
            + 1;
        if event_line_bytes > events::append::MAX_EVENT_LINE_BYTES {
            return Err(format!(
                "PG attempt event is {event_line_bytes} bytes, exceeding TG's {}-byte event limit",
                events::append::MAX_EVENT_LINE_BYTES
            ));
        }
        Ok(note)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SupervisedTransition {
    Complete,
    Blocked,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SupervisedOutcome {
    Complete,
    Blocked,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BlockType {
    Clarification,
    Decision,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
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

pub fn parse_item_status(s: &str) -> Result<Status, String> {
    s.to_lowercase().parse()
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
    TransitionStatus(Status),
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
    Skipped(String),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerAction {
    Triage(String),
    Claim(String),
    HumanGate(String),
    Block {
        item_id: String,
        reason: String,
    },
    RunPhase {
        item_id: String,
        phase: String,
        phase_pool: PhasePool,
        is_destructive: bool,
    },
}

// --- Structs ---

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<StructuredDescription>,
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

impl StructuredDescription {
    pub fn is_empty(&self) -> bool {
        self.context.is_empty()
            && self.problem.is_empty()
            && self.solution.is_empty()
            && self.impact.is_empty()
            && self.sizing_rationale.is_empty()
    }
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
