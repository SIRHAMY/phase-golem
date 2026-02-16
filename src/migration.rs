use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use tempfile::NamedTempFile;

use crate::config::PipelineConfig;
use crate::types::{BacklogFile, BacklogItem, ItemStatus, PhasePool, StructuredDescription};
use crate::{log_debug, log_info, log_warn};

// --- V1 Definitions (preserved for parsing old BACKLOG.yaml files) ---

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct V1BacklogFile {
    #[serde(default = "default_v1_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub items: Vec<V1BacklogItem>,
}

fn default_v1_schema_version() -> u32 {
    1
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum V1ItemStatus {
    New,
    Researching,
    Scoped,
    Ready,
    InProgress,
    Done,
    Blocked,
}

#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum V1WorkflowPhase {
    Prd,
    Research,
    Design,
    Spec,
    Build,
    Review,
}

impl V1WorkflowPhase {
    fn as_str(&self) -> &'static str {
        match self {
            V1WorkflowPhase::Prd => "prd",
            V1WorkflowPhase::Research => "research",
            V1WorkflowPhase::Design => "design",
            V1WorkflowPhase::Spec => "spec",
            V1WorkflowPhase::Build => "build",
            V1WorkflowPhase::Review => "review",
        }
    }
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct V1BacklogItem {
    pub id: String,
    pub title: String,
    pub status: V1ItemStatus,
    #[serde(default)]
    pub phase: Option<V1WorkflowPhase>,
    #[serde(default)]
    pub size: Option<crate::types::SizeLevel>,
    #[serde(default)]
    pub complexity: Option<crate::types::DimensionLevel>,
    #[serde(default)]
    pub risk: Option<crate::types::DimensionLevel>,
    #[serde(default)]
    pub impact: Option<crate::types::DimensionLevel>,
    #[serde(default)]
    pub requires_human_review: bool,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub blocked_from_status: Option<V1ItemStatus>,
    #[serde(default)]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub blocked_type: Option<crate::types::BlockType>,
    #[serde(default)]
    pub unblock_context: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub created: String,
    pub updated: String,
}

// --- Migration Logic ---

fn map_v1_status(status: &V1ItemStatus) -> ItemStatus {
    match status {
        V1ItemStatus::New => ItemStatus::New,
        V1ItemStatus::Researching => ItemStatus::Scoping,
        V1ItemStatus::Scoped => ItemStatus::Ready,
        V1ItemStatus::Ready => ItemStatus::Ready,
        V1ItemStatus::InProgress => ItemStatus::InProgress,
        V1ItemStatus::Done => ItemStatus::Done,
        V1ItemStatus::Blocked => ItemStatus::Blocked,
    }
}

fn map_v1_item(v1: &V1BacklogItem) -> BacklogItem {
    let status = map_v1_status(&v1.status);

    let phase: Option<String> = v1.phase.as_ref().map(|p| p.as_str().to_string());

    // Set phase_pool based on mapped status
    let phase_pool = match status {
        ItemStatus::Scoping => {
            // Researching items mapped to Scoping — if they had a phase it was a
            // v1 WorkflowPhase which doesn't apply to pre_phases. Clear phase and
            // let scheduler auto-promote on next run.
            None
        }
        ItemStatus::InProgress => {
            if phase.is_some() {
                Some(PhasePool::Main)
            } else {
                None
            }
        }
        _ => None,
    };

    // For Researching→Scoping items, clear the phase since v1 phases don't
    // map to pre_phases (scheduler will handle re-assignment)
    let phase = if v1.status == V1ItemStatus::Researching {
        None
    } else {
        phase
    };

    // Map blocked_from_status
    let blocked_from_status = v1.blocked_from_status.as_ref().map(map_v1_status);

    BacklogItem {
        id: v1.id.clone(),
        title: v1.title.clone(),
        status,
        phase,
        size: v1.size.clone(),
        complexity: v1.complexity.clone(),
        risk: v1.risk.clone(),
        impact: v1.impact.clone(),
        requires_human_review: v1.requires_human_review,
        origin: v1.origin.clone(),
        blocked_from_status,
        blocked_reason: v1.blocked_reason.clone(),
        blocked_type: v1.blocked_type.clone(),
        unblock_context: v1.unblock_context.clone(),
        tags: v1.tags.clone(),
        dependencies: v1.dependencies.clone(),
        created: v1.created.clone(),
        updated: v1.updated.clone(),
        pipeline_type: Some("feature".to_string()),
        phase_pool,
        ..Default::default()
    }
}

/// Migrate a v1 BACKLOG.yaml to v2 format.
///
/// Reads the file, parses as v1, maps statuses and phases, writes back as v2.
/// Uses atomic write-temp-rename pattern.
///
/// If the file is already v2+, parses as V2 and maps descriptions to
/// StructuredDescription via parse_description (returns a BacklogFile
/// with the on-disk schema_version, not necessarily v2).
pub fn migrate_v1_to_v2(path: &Path, pipeline: &PipelineConfig) -> Result<BacklogFile, String> {
    let contents = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    // Check if already v2 by parsing the version field
    let version_check: serde_yaml_ng::Value = serde_yaml_ng::from_str(&contents)
        .map_err(|e| format!("Failed to parse YAML from {}: {}", path.display(), e))?;

    let schema_version = version_check
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(1);

    if schema_version >= 2 {
        // Already v2+ — parse as V2BacklogFile and map to current BacklogFile
        let v2: V2BacklogFile = serde_yaml_ng::from_str(&contents)
            .map_err(|e| format!("Failed to parse v2 YAML from {}: {}", path.display(), e))?;
        let items: Vec<BacklogItem> = v2.items.iter().map(map_v2_item).collect();
        return Ok(BacklogFile {
            schema_version: v2.schema_version,
            items,
            next_item_id: v2.next_item_id,
        });
    }

    // Parse as v1
    let v1: V1BacklogFile = serde_yaml_ng::from_str(&contents)
        .map_err(|e| format!("Failed to parse v1 YAML from {}: {}", path.display(), e))?;

    log_info!(
        "Migrating BACKLOG.yaml v1 → v2: {} ({} items)",
        path.display(),
        v1.items.len()
    );

    // Build set of valid phase names from pipeline config
    let valid_phases: HashSet<&str> = pipeline
        .pre_phases
        .iter()
        .chain(pipeline.phases.iter())
        .map(|p| p.name.as_str())
        .collect();

    // Map to v2 with per-item change logging
    let mut status_changes: usize = 0;
    let mut phase_clears: usize = 0;
    let mut validation_clears: usize = 0;
    let mut unchanged: usize = 0;
    let mut items: Vec<BacklogItem> = Vec::with_capacity(v1.items.len());

    for v1_item in &v1.items {
        let mut v2_item = map_v1_item(v1_item);

        let v1_status_name = format!("{:?}", v1_item.status);
        let v2_status_name = format!("{:?}", v2_item.status);
        let status_changed = v1_status_name != v2_status_name;
        if status_changed {
            log_info!(
                "  {}: status {} → {}",
                v1_item.id,
                v1_status_name,
                v2_status_name
            );
            status_changes += 1;
        }

        let mut phase_cleared = false;
        if let Some(ref old_phase) = v1_item.phase {
            if v2_item.phase.is_none() {
                log_info!(
                    "  {}: phase cleared (was '{}')",
                    v1_item.id,
                    old_phase.as_str()
                );
                phase_clears += 1;
                phase_cleared = true;
            }
        }

        if let (Some(ref v1_blocked), Some(ref v2_blocked)) =
            (&v1_item.blocked_from_status, &v2_item.blocked_from_status)
        {
            let v1_blocked_name = format!("{:?}", v1_blocked);
            let v2_blocked_name = format!("{:?}", v2_blocked);
            if v1_blocked_name != v2_blocked_name {
                log_warn!(
                    "  {}: blocked_from_status mapped {} → {}",
                    v1_item.id,
                    v1_blocked_name,
                    v2_blocked_name
                );
            }
        }

        log_debug!(
            "  {}: v1{{status:{:?}, phase:{:?}}} → v2{{status:{:?}, phase:{:?}, phase_pool:{:?}, pipeline_type:{:?}}}",
            v1_item.id,
            v1_item.status,
            v1_item.phase,
            v2_item.status,
            v2_item.phase,
            v2_item.phase_pool,
            v2_item.pipeline_type
        );

        // Validate phase against pipeline config
        let mut validation_cleared = false;
        if let Some(ref name) = v2_item.phase {
            if !valid_phases.contains(name.as_str()) {
                log_warn!(
                    "  {}: phase '{}' not found in feature pipeline phases; cleared",
                    v1_item.id,
                    name
                );
                v2_item.phase = None;
                v2_item.phase_pool = None;
                validation_clears += 1;
                validation_cleared = true;
            }
        }

        if !status_changed && !phase_cleared && !validation_cleared {
            unchanged += 1;
        }

        items.push(v2_item);
    }

    log_info!(
        "Migrated {} items: {} status changes, {} phase cleared, {} validation cleared, {} unchanged",
        v1.items.len(),
        status_changes,
        phase_clears,
        validation_clears,
        unchanged
    );

    let backlog = BacklogFile {
        schema_version: 2,
        items,
        next_item_id: 0,
    };

    // Atomic write
    let parent = path
        .parent()
        .ok_or_else(|| format!("Cannot determine parent directory of {}", path.display()))?;

    let yaml = serde_yaml_ng::to_string(&backlog)
        .map_err(|e| format!("Failed to serialize backlog to YAML: {}", e))?;

    let temp_file = NamedTempFile::new_in(parent)
        .map_err(|e| format!("Failed to create temp file in {}: {}", parent.display(), e))?;

    fs::write(temp_file.path(), &yaml).map_err(|e| format!("Failed to write temp file: {}", e))?;

    let file = fs::File::open(temp_file.path())
        .map_err(|e| format!("Failed to open temp file for sync: {}", e))?;
    file.sync_all()
        .map_err(|e| format!("Failed to sync temp file: {}", e))?;

    temp_file
        .persist(path)
        .map_err(|e| format!("Failed to rename temp file to {}: {}", path.display(), e))?;

    log_info!("Migration complete: {}", path.display());

    Ok(backlog)
}

// --- V2 Definitions (preserved for parsing v2 BACKLOG.yaml files) ---

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct V2BacklogFile {
    pub schema_version: u32,
    #[serde(default)]
    pub items: Vec<V2BacklogItem>,
    #[serde(default)]
    pub next_item_id: u32,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct V2BacklogItem {
    pub id: String,
    pub title: String,
    pub status: ItemStatus,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub size: Option<crate::types::SizeLevel>,
    #[serde(default)]
    pub complexity: Option<crate::types::DimensionLevel>,
    #[serde(default)]
    pub risk: Option<crate::types::DimensionLevel>,
    #[serde(default)]
    pub impact: Option<crate::types::DimensionLevel>,
    #[serde(default)]
    pub requires_human_review: bool,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub blocked_from_status: Option<ItemStatus>,
    #[serde(default)]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub blocked_type: Option<crate::types::BlockType>,
    #[serde(default)]
    pub unblock_context: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub created: String,
    pub updated: String,
    #[serde(default)]
    pub pipeline_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub phase_pool: Option<PhasePool>,
    #[serde(default)]
    pub last_phase_commit: Option<String>,
}

// --- V2 → V3 Migration Logic ---

fn map_v2_item(v2: &V2BacklogItem) -> BacklogItem {
    let description = v2.description.as_deref().map(|text| {
        let desc = parse_description(text);
        // Warn for non-conforming descriptions (no recognized headers → full text in context)
        if desc.problem.is_empty()
            && desc.solution.is_empty()
            && desc.impact.is_empty()
            && desc.sizing_rationale.is_empty()
            && !desc.context.is_empty()
        {
            let preview: String = text.chars().take(80).collect();
            log_warn!(
                "  {}: non-conforming description (no headers): \"{}\"",
                v2.id,
                preview
            );
        }
        desc
    });

    BacklogItem {
        id: v2.id.clone(),
        title: v2.title.clone(),
        status: v2.status.clone(),
        phase: v2.phase.clone(),
        size: v2.size.clone(),
        complexity: v2.complexity.clone(),
        risk: v2.risk.clone(),
        impact: v2.impact.clone(),
        requires_human_review: v2.requires_human_review,
        origin: v2.origin.clone(),
        blocked_from_status: v2.blocked_from_status.clone(),
        blocked_reason: v2.blocked_reason.clone(),
        blocked_type: v2.blocked_type.clone(),
        unblock_context: v2.unblock_context.clone(),
        tags: v2.tags.clone(),
        dependencies: v2.dependencies.clone(),
        created: v2.created.clone(),
        updated: v2.updated.clone(),
        pipeline_type: v2.pipeline_type.clone(),
        description,
        phase_pool: v2.phase_pool.clone(),
        last_phase_commit: v2.last_phase_commit.clone(),
    }
}

/// Migrate a v2 BACKLOG.yaml to v3 format.
///
/// Reads the file, parses as v2, transforms descriptions via `parse_description`,
/// writes back as v3. Uses atomic write-temp-rename pattern.
pub fn migrate_v2_to_v3(path: &Path) -> Result<BacklogFile, String> {
    let contents = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let version_check: serde_yaml_ng::Value = serde_yaml_ng::from_str(&contents)
        .map_err(|e| format!("Failed to parse YAML from {}: {}", path.display(), e))?;

    let schema_version = version_check
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;

    if schema_version != 2 {
        return Err(format!(
            "migrate_v2_to_v3 expected schema_version 2, got {} in {}",
            schema_version,
            path.display()
        ));
    }

    let v2: V2BacklogFile = serde_yaml_ng::from_str(&contents)
        .map_err(|e| format!("Failed to parse v2 YAML from {}: {}", path.display(), e))?;

    log_info!(
        "Migrating BACKLOG.yaml v2 → v3: {} ({} items)",
        path.display(),
        v2.items.len()
    );

    let items: Vec<BacklogItem> = v2.items.iter().map(map_v2_item).collect();

    let backlog = BacklogFile {
        schema_version: 3,
        items,
        next_item_id: v2.next_item_id,
    };

    // Atomic write
    let parent = path
        .parent()
        .ok_or_else(|| format!("Cannot determine parent directory of {}", path.display()))?;

    let yaml = serde_yaml_ng::to_string(&backlog)
        .map_err(|e| format!("Failed to serialize backlog to YAML: {}", e))?;

    let temp_file = NamedTempFile::new_in(parent)
        .map_err(|e| format!("Failed to create temp file in {}: {}", parent.display(), e))?;

    fs::write(temp_file.path(), &yaml).map_err(|e| format!("Failed to write temp file: {}", e))?;

    let file = fs::File::open(temp_file.path())
        .map_err(|e| format!("Failed to open temp file for sync: {}", e))?;
    file.sync_all()
        .map_err(|e| format!("Failed to sync temp file: {}", e))?;

    temp_file
        .persist(path)
        .map_err(|e| format!("Failed to rename temp file to {}: {}", path.display(), e))?;

    log_info!("Migration v2 → v3 complete: {}", path.display());

    Ok(backlog)
}

// --- Description Parsing ---

/// Known section headers for structured descriptions.
/// All entries must be lowercase ASCII — the parser uses byte-length slicing
/// from the lowercased input to extract content after the colon, which is only
/// safe when `to_lowercase()` preserves byte length (guaranteed for ASCII).
const SECTION_HEADERS: &[(&str, &str)] = &[
    ("context:", "context"),
    ("problem:", "problem"),
    ("solution:", "solution"),
    ("impact:", "impact"),
    ("sizing rationale:", "sizing_rationale"),
];

/// Parse a freeform description string into a `StructuredDescription`.
///
/// Scans for known section headers (`Context:`, `Problem:`, `Solution:`,
/// `Impact:`, `Sizing rationale:`) at line starts (case-insensitive, after trim).
/// Content following each header is accumulated until the next header or end of input.
///
/// If no headers are found, the entire text is placed in the `context` field
/// with all other fields as empty strings. The parser is infallible — it always
/// produces a valid `StructuredDescription`.
pub fn parse_description(text: &str) -> StructuredDescription {
    // Section indices: 0=context, 1=problem, 2=solution, 3=impact, 4=sizing_rationale
    let mut sections: [Vec<String>; 5] = Default::default();
    let mut current_section: Option<usize> = None;
    let mut any_header_found = false;
    let mut pre_header_lines: Vec<String> = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        let trimmed_lower = trimmed.to_lowercase();

        let matched_section = SECTION_HEADERS
            .iter()
            .enumerate()
            .find_map(|(i, &(header, _))| {
                if trimmed_lower.starts_with(header) {
                    Some((i, header.len()))
                } else {
                    None
                }
            });

        if let Some((section_idx, header_len)) = matched_section {
            any_header_found = true;
            current_section = Some(section_idx);

            // Duplicate header: clear ALL previous content for this section,
            // including continuation lines from the first occurrence.
            // "Last occurrence wins entirely" — no merging across duplicates.
            sections[section_idx].clear();

            // Include content after the colon on the same line.
            // Safety: header_len comes from an ASCII-only header constant,
            // so byte-length slicing on the original string is always valid.
            debug_assert!(trimmed.is_char_boundary(header_len));
            let after_colon = trimmed[header_len..].trim();
            if !after_colon.is_empty() {
                sections[section_idx].push(after_colon.to_string());
            }
        } else {
            match current_section {
                Some(idx) => sections[idx].push(trimmed.to_string()),
                None => pre_header_lines.push(trimmed.to_string()),
            }
        }
    }

    if !any_header_found {
        return StructuredDescription {
            context: text.trim().to_string(),
            problem: String::new(),
            solution: String::new(),
            impact: String::new(),
            sizing_rationale: String::new(),
        };
    }

    // Prepend any pre-header lines to context
    if !pre_header_lines.is_empty() {
        let mut combined = pre_header_lines;
        combined.append(&mut sections[0]);
        sections[0] = combined;
    }

    let [context, problem, solution, impact, sizing_rationale] = sections;

    StructuredDescription {
        context: join_and_trim(&context),
        problem: join_and_trim(&problem),
        solution: join_and_trim(&solution),
        impact: join_and_trim(&impact),
        sizing_rationale: join_and_trim(&sizing_rationale),
    }
}

fn join_and_trim(lines: &[String]) -> String {
    let joined = lines.join("\n");
    joined.trim().to_string()
}
