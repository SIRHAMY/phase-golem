use std::collections::HashMap;
use std::path::Path;

use crate::config::{PhaseConfig, PipelineConfig};
use crate::types::{BacklogItem, PhasePool, StructuredDescription};

/// Parameters for building a workflow phase prompt.
pub struct PromptParams<'a> {
    pub phase: &'a str,
    pub phase_config: &'a PhaseConfig,
    pub item: &'a BacklogItem,
    pub result_path: &'a Path,
    pub change_folder: &'a Path,
    pub previous_summary: Option<&'a str>,
    pub unblock_notes: Option<&'a str>,
    pub failure_context: Option<&'a str>,
    /// Base directory for resolving config-relative paths (workflow files).
    /// When `--config` is used, this is the config file's parent directory.
    /// Otherwise, it equals the project root.
    pub config_base: &'a Path,
}

/// Build a full prompt for a workflow phase agent.
///
/// Structure: [Autonomous Preamble] + [Skill Invocation] + [Structured Output Suffix]
///
/// The preamble provides context about the item and autonomous execution mode.
/// The workflow invocation tells the agent which workflow files to read and follow.
/// The suffix instructs the agent to write structured JSON output.
pub fn build_prompt(params: &PromptParams) -> String {
    let preamble = build_preamble(
        "Autonomous Agent",
        "You are running autonomously as part of the phase-golem changes workflow.\n\
        No human is available for questions — use your judgment to make decisions.",
        params.item,
        None,
        params.previous_summary,
        params.unblock_notes,
        params.failure_context,
    );

    [
        preamble,
        build_skill_invocation(params.phase_config, params.change_folder, params.config_base),
        build_output_suffix(&params.item.id, params.phase, params.result_path),
    ]
    .join("\n\n")
}

/// Build a one-line-per-item summary of the backlog for triage duplicate detection.
///
/// Returns `None` if the backlog is empty (after excluding the current item).
/// Each line: `- {id}: {title} [{status}]`
pub fn build_backlog_summary(items: &[BacklogItem], exclude_id: &str) -> Option<String> {
    let lines: Vec<String> = items
        .iter()
        .filter(|i| i.id != exclude_id)
        .map(|i| {
            let status = format!("{:?}", i.status).to_lowercase();
            format!("- {}: {} [{}]", i.id, i.title, status)
        })
        .collect();

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Build a prompt for the triage agent (pre-workflow).
///
/// The triage agent assesses new items for size/complexity/risk/impact,
/// creates idea files if needed, and promotes small+low-risk items directly.
/// Includes available pipeline types from config for classification.
/// When `backlog_summary` is provided, includes it for duplicate detection.
pub fn build_triage_prompt(
    item: &BacklogItem,
    result_path: &Path,
    available_pipelines: &HashMap<String, PipelineConfig>,
    backlog_summary: Option<&str>,
) -> String {
    let pipeline_list = if available_pipelines.is_empty() {
        "- `feature` (default)".to_string()
    } else {
        let mut names: Vec<&String> = available_pipelines.keys().collect();
        names.sort();
        names
            .iter()
            .map(|name| format!("- `{}`", name))
            .collect::<Vec<_>>()
            .join("\n")
    };

    let mut sections = vec![build_preamble(
        "Autonomous Triage Agent",
        "You are running autonomously as a triage agent. No human is available for questions.",
        item,
        None,
        None,
        None,
        None,
    )
    .replace("## Item", "## Item to Triage")];

    if let Some(summary) = backlog_summary {
        sections.push(format!(
            "## Current Backlog\n\n\
            The following items already exist in the backlog. Check for duplicates — if this item \
            duplicates an existing one, report the existing item's ID in the `duplicates` field. \
            Higher-numbered ID merges into lower-numbered ID.\n\n{}",
            summary
        ));
    }

    sections.push(format!(
        "## Available Pipeline Types\n\n{}\n\n\
        ## Instructions\n\n\
        Assess this backlog item and determine how to route it:\n\n\
        1. **Read the item title and any available context** to understand what this work item involves.\n\
        2. **Check for duplicates** — compare against the Current Backlog (if provided). If this item \
           duplicates existing work, list the existing item ID(s) in the `duplicates` field.\n\
        3. **Classify the pipeline type** — choose the most appropriate pipeline from the list above.\n\
           Include `pipeline_type` in your structured output.\n\
        4. **Assess dimensions** — evaluate size, complexity, risk, and impact using these guidelines:\n\
           - **Size:** Small (1-3 files), Medium (4-10 files), Large (11+ files)\n\
           - **Complexity:** Low (single pattern), Medium (design decisions), High (new architecture)\n\
           - **Risk:** Low (no shared interfaces), Medium (modifies shared code), High (breaking changes)\n\
           - **Impact:** Low (nice-to-have), Medium (meaningful improvement), High (critical/blocking)\n\
        5. **Decide routing:**\n\
           - If the item is **small size AND low risk**: promote directly (no idea file needed).\n\
             Set `requires_human_review: false` in your result.\n\
           - If the item is **medium+ size OR medium+ risk**: create an idea file at\n\
             `_ideas/{{item_id}}_{{slug}}.md` with problem statement, proposed approach, and assessment.\n\
             Set `requires_human_review` based on risk level (true if high risk).\n\
        6. **Report your assessment** in the structured output.\n\n\
        Also use `blocked` if the work is not needed (e.g., already implemented, obsolete, out of scope).\n\n\
        Use your judgment. When uncertain, err on the side of creating an idea file and flagging for review.",
        pipeline_list,
    ));

    sections.push(build_triage_output_suffix(&item.id, result_path));

    sections.join("\n\n")
}

/// Build the structured output suffix for triage, which includes pipeline_type field.
fn build_triage_output_suffix(item_id: &str, result_path: &Path) -> String {
    format!(
        "## Structured Output\n\n\
        When you are finished, write a JSON result file to:\n\n\
        ```\n{result_path}\n```\n\n\
        The file must contain valid JSON matching this schema:\n\n\
        ```json\n\
        {{\n\
        \x20 \"item_id\": \"{item_id}\",\n\
        \x20 \"phase\": \"triage\",\n\
        \x20 \"result\": \"phase_complete | failed | blocked\",\n\
        \x20 \"summary\": \"Brief description of triage assessment\",\n\
        \x20 \"context\": \"Optional additional context\",\n\
        \x20 \"pipeline_type\": \"feature\",\n\
        \x20 \"updated_assessments\": {{\n\
        \x20   \"size\": \"small | medium | large\",\n\
        \x20   \"complexity\": \"low | medium | high\",\n\
        \x20   \"risk\": \"low | medium | high\",\n\
        \x20   \"impact\": \"low | medium | high\"\n\
        \x20 }},\n\
        \x20 \"commit_summary\": \"One-line summary for git commit message\",\n\
        \x20 \"follow_ups\": [\n\
        \x20   {{\n\
        \x20     \"title\": \"Follow-up item title\",\n\
        \x20     \"context\": \"Why this follow-up is needed (optional)\",\n\
        \x20     \"suggested_size\": \"small | medium | large (optional)\",\n\
        \x20     \"suggested_risk\": \"low | medium | high (optional)\"\n\
        \x20   }}\n\
        \x20 ],\n\
        \x20 \"duplicates\": [\"WRK-xxx\"]\n\
        }}\n\
        ```\n\n\
        **Result codes:**\n\
        - `phase_complete` — Triage complete, item assessed and routed.\n\
        - `failed` — Could not assess the item. Explain why in `context`.\n\
        - `blocked` — The item needs human input before it can be triaged. \
        Also use `blocked` if the work is not needed (e.g., already implemented, obsolete, out of scope).\n\n\
        **Important:**\n\
        - Set `pipeline_type` to classify this item into the appropriate pipeline.\n\
        - Include a short `commit_summary` (under 72 chars) describing what changed — used as the git commit title.\n\
        - List item IDs this work duplicates in `duplicates`. Higher-numbered ID merges into lower-numbered ID. Omit if no duplicates.\n\
        - The JSON must be valid — do not include comments or trailing commas.",
        result_path = result_path.display(),
        item_id = item_id,
    )
}

// --- Internal helpers ---

/// Build the preamble section of an autonomous prompt.
///
/// Shared by all prompt builders. Includes agent heading, item info,
/// and optional context sections (assessments, previous summary, unblock notes, failure context).
fn build_preamble(
    heading: &str,
    intro: &str,
    item: &BacklogItem,
    extra_item_field: Option<&str>,
    previous_summary: Option<&str>,
    unblock_notes: Option<&str>,
    failure_context: Option<&str>,
) -> String {
    let mut preamble = format!(
        "# {heading}\n\n\
        {intro}\n\
        Record any questions you would normally ask in an \"Assumptions\" section of the artifact,\n\
        documenting decisions made without human input.\n\n\
        ## Item\n\n\
        - **ID:** {id}\n\
        - **Title:** {title}",
        heading = heading,
        intro = intro,
        id = item.id,
        title = item.title,
    );

    if let Some(extra) = extra_item_field {
        preamble.push_str(&format!("\n{}", extra));
    }

    if let Some(assessments) = format_assessments(item) {
        preamble.push_str(&format!("\n\n## Current Assessments\n\n{}", assessments));
    }

    if let Some(ref desc) = item.description {
        let rendered = render_structured_description(desc);
        if !rendered.is_empty() {
            preamble.push_str(&format!("\n\n## Description\n\n{}", rendered));
        }
    }

    if let Some(summary) = previous_summary {
        preamble.push_str(&format!("\n\n## Previous Phase Summary\n\n{}", summary));
    }

    if let Some(notes) = unblock_notes {
        preamble.push_str(&format!(
            "\n\n## Unblock Context\n\nThis item was previously blocked. Context from the human:\n\n{}",
            notes
        ));
    }

    if let Some(context) = failure_context {
        preamble.push_str(&format!(
            "\n\n## Previous Failure\n\nThe previous attempt at this phase failed. Here is what happened:\n\n{}\n\n\
            Analyze the failure and try a different approach.",
            context
        ));
    }

    preamble
}

/// Build the workflow invocation section from phase config.
///
/// References workflow files by relative path. Any agent can read a file
/// and follow its instructions, making this robust across agent runtimes.
fn build_skill_invocation(phase_config: &PhaseConfig, change_folder: &Path, config_base: &Path) -> String {
    let change_path = change_folder.display();

    // Resolve workflow paths relative to config_base so agents can always find them.
    let resolved: Vec<String> = phase_config
        .workflows
        .iter()
        .map(|wf| config_base.join(wf).to_string_lossy().to_string())
        .collect();

    if resolved.len() == 1 {
        format!(
            "## Task\n\nRead and follow the workflow at `{}`.\n\nThe change folder for this item is: `{}`",
            resolved[0], change_path,
        )
    } else {
        let instructions: Vec<String> = resolved
            .iter()
            .enumerate()
            .map(|(i, wf)| format!("{}. Read and follow the workflow at `{}`.", i + 1, wf))
            .collect();
        format!(
            "## Task\n\nComplete the following workflows in order:\n\n{}\n\nThe change folder for this item is: `{}`",
            instructions.join("\n"),
            change_path,
        )
    }
}

/// Build the structured output suffix that instructs the agent to write a JSON result file.
fn build_output_suffix(item_id: &str, phase_str: &str, result_path: &Path) -> String {
    format!(
        "## Structured Output\n\n\
        When you are finished, write a JSON result file to:\n\n\
        ```\n{result_path}\n```\n\n\
        The file must contain valid JSON matching this schema:\n\n\
        ```json\n\
        {{\n\
        \x20 \"item_id\": \"{item_id}\",\n\
        \x20 \"phase\": \"{phase_str}\",\n\
        \x20 \"result\": \"phase_complete | subphase_complete | failed | blocked\",\n\
        \x20 \"summary\": \"Brief description of what was accomplished\",\n\
        \x20 \"context\": \"Optional additional context (for failures/blocks, explain why)\",\n\
        \x20 \"updated_assessments\": {{\n\
        \x20   \"size\": \"small | medium | large (optional)\",\n\
        \x20   \"complexity\": \"low | medium | high (optional)\",\n\
        \x20   \"risk\": \"low | medium | high (optional)\",\n\
        \x20   \"impact\": \"low | medium | high (optional)\"\n\
        \x20 }},\n\
        \x20 \"commit_summary\": \"One-line summary for git commit message\",\n\
        \x20 \"follow_ups\": [\n\
        \x20   {{\n\
        \x20     \"title\": \"Follow-up item title\",\n\
        \x20     \"context\": \"Why this follow-up is needed\",\n\
        \x20     \"suggested_size\": \"small | medium | large (optional)\",\n\
        \x20     \"suggested_risk\": \"low | medium | high (optional)\"\n\
        \x20   }}\n\
        \x20 ]\n\
        }}\n\
        ```\n\n\
        **Result codes:**\n\
        - `phase_complete` — This phase is fully done. All work completed successfully.\n\
        - `subphase_complete` — A sub-phase is done but more work remains in this phase (build only).\n\
        - `failed` — The phase could not be completed. Explain why in `context`.\n\
        - `blocked` — The phase needs human input to proceed, or the work is not needed \
        (e.g., already implemented, obsolete, out of scope). Explain what's needed in `context`.\n\n\
        **Important:**\n\
        - Update assessments if your work revealed the item is larger/smaller/riskier than expected.\n\
        - Report any follow-up work items discovered during this phase.\n\
        - Include a short `commit_summary` (under 72 chars) describing what changed — used as the git commit title.\n\
        - The JSON must be valid — do not include comments or trailing commas.",
        result_path = result_path.display(),
        item_id = item_id,
        phase_str = phase_str,
    )
}

/// Build a structured context preamble for autonomous execution mode.
///
/// This provides the agent with structured metadata about the phase-golem context:
/// mode, item metadata, pipeline/phase position, description, and optional
/// retry/unblock context.
///
/// Output format (markdown):
/// ```text
/// ## Phase Golem Context
///
/// **Mode:** autonomous
/// **Item:** WRK-003 — Orchestrator Pipeline Engine v2
/// **Pipeline:** feature
/// **Phase:** build (4/6, main)
/// **Description:** [user's free-form description]
///
/// ### Previous Phase Summary
/// [Extracted from last PhaseResult]
///
/// ### Retry Context
/// Attempt 2/3. Previous failure: [error summary]
///
/// ### Unblock Context
/// [Human's unblock notes from `phase-golem unblock`]
/// ```
/// Staged for Phase 6 (Scheduler) integration — will replace `build_preamble`
/// when the scheduler calls `execute_phase` with full pipeline context.
#[allow(dead_code)]
pub fn build_context_preamble(
    item: &BacklogItem,
    pipeline: &PipelineConfig,
    previous_summary: Option<&str>,
    unblock_notes: Option<&str>,
    failure_context: Option<&str>,
) -> String {
    let pipeline_type = item.pipeline_type.as_deref().unwrap_or("feature");
    let phase_position = format_phase_position(item, pipeline);

    let mut sections = vec![format!(
        "## Phase Golem Context\n\n\
        **Mode:** autonomous\n\
        **Item:** {} — {}\n\
        **Pipeline:** {}\n\
        **Phase:** {}",
        item.id, item.title, pipeline_type, phase_position
    )];

    if let Some(ref desc) = item.description {
        let rendered = render_structured_description(desc);
        if !rendered.is_empty() {
            sections.push(format!("### Description\n\n{}", rendered));
        }
    }

    if let Some(summary) = previous_summary {
        sections.push(format!("### Previous Phase Summary\n\n{}", summary));
    }

    if let Some(context) = failure_context {
        sections.push(format!(
            "### Retry Context\n\nPrevious failure: {}",
            context
        ));
    }

    if let Some(notes) = unblock_notes {
        sections.push(format!("### Unblock Context\n\n{}", notes));
    }

    sections.join("\n\n")
}

/// Format the phase position string (e.g., "build (4/6, main)").
fn format_phase_position(item: &BacklogItem, pipeline: &PipelineConfig) -> String {
    let phase_name = item.phase.as_deref().unwrap_or("unknown");
    let pool = item.phase_pool.as_ref();

    let (phase_list, pool_label) = match pool {
        Some(PhasePool::Pre) => (&pipeline.pre_phases, "pre"),
        _ => (&pipeline.phases, "main"),
    };

    let total = phase_list.len();
    let position = phase_list
        .iter()
        .position(|p| p.name == phase_name)
        .map(|idx| idx + 1)
        .unwrap_or(0);

    if position > 0 {
        format!("{} ({}/{}, {})", phase_name, position, total, pool_label)
    } else {
        format!("{} ({})", phase_name, pool_label)
    }
}

/// Render a `StructuredDescription` as labeled markdown lines.
///
/// Emits non-empty fields in order: Context, Problem, Solution, Impact, Sizing Rationale.
/// Each field is rendered as `**{Label}:** {content}`. Returns empty string if all fields
/// are empty strings.
fn render_structured_description(desc: &StructuredDescription) -> String {
    let fields: [(&str, &str); 5] = [
        ("Context", &desc.context),
        ("Problem", &desc.problem),
        ("Solution", &desc.solution),
        ("Impact", &desc.impact),
        ("Sizing Rationale", &desc.sizing_rationale),
    ];

    let lines: Vec<String> = fields
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(label, value)| format!("**{}:** {}", label, value))
        .collect();

    lines.join("\n")
}

/// Format current assessments for display in prompts.
fn format_assessments(item: &BacklogItem) -> Option<String> {
    let mut lines = Vec::new();

    if let Some(ref size) = item.size {
        lines.push(format!("- **Size:** {}", size));
    }
    if let Some(ref complexity) = item.complexity {
        lines.push(format!("- **Complexity:** {}", complexity));
    }
    if let Some(ref risk) = item.risk {
        lines.push(format!("- **Risk:** {}", risk));
    }
    if let Some(ref impact) = item.impact {
        lines.push(format!("- **Impact:** {}", impact));
    }

    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}
