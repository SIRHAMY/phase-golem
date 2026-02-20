use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use tempfile::NamedTempFile;

use crate::config::load_config;
use crate::log_warn;
use crate::types::{
    BacklogFile, BacklogItem, DimensionLevel, FollowUp, InboxItem, ItemStatus, SizeLevel,
    StructuredDescription, UpdatedAssessments,
};

const EXPECTED_SCHEMA_VERSION: u32 = 3;

/// Load a BacklogFile from a YAML file at the given path.
///
/// If the file is below the current schema version, auto-migrates through
/// the chain (v1 → v2 → v3). Each step writes to disk before the next runs,
/// so partial migration is retry-safe.
/// Validates schema_version matches the expected version after migration.
/// Unknown fields are silently ignored (forward compatibility).
pub fn load(path: &Path, project_root: &Path) -> Result<BacklogFile, String> {
    let contents = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    // Check schema version to decide whether migration is needed
    let parsed_yaml: serde_yaml_ng::Value = serde_yaml_ng::from_str(&contents)
        .map_err(|e| format!("Failed to parse YAML from {}: {}", path.display(), e))?;

    let schema_version = parsed_yaml
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;

    if schema_version < EXPECTED_SCHEMA_VERSION {
        // Chain migrations sequentially: v1 → v2 → v3
        if schema_version == 1 {
            let config = load_config(project_root)?;
            let pipeline = config.pipelines.get("feature").ok_or_else(|| {
                "Migration requires 'feature' pipeline in config, but none found".to_string()
            })?;
            crate::migration::migrate_v1_to_v2(path, pipeline)?;
            // File is now v2 on disk; fall through to v2→v3
        }
        if schema_version <= 2 {
            let backlog = crate::migration::migrate_v2_to_v3(path)?;
            // File is now v3 on disk; return the migrated backlog directly
            return Ok(backlog);
        }
    }

    if schema_version != EXPECTED_SCHEMA_VERSION {
        return Err(format!(
            "Unsupported schema_version {} in {} (expected {})",
            schema_version,
            path.display(),
            EXPECTED_SCHEMA_VERSION
        ));
    }

    let backlog: BacklogFile = serde_yaml_ng::from_value(parsed_yaml)
        .map_err(|e| format!("Failed to parse YAML from {}: {}", path.display(), e))?;

    Ok(backlog)
}

/// Save a BacklogFile to a YAML file at the given path using atomic write.
///
/// Uses write-temp-rename pattern: writes to a temporary file in the same
/// directory, syncs to disk, then atomically renames to the target path.
/// This ensures the file is either the old version or the new version, never partial.
pub fn save(path: &Path, backlog: &BacklogFile) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("Cannot determine parent directory of {}", path.display()))?;

    fs::create_dir_all(parent)
        .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;

    let yaml = serde_yaml_ng::to_string(backlog)
        .map_err(|e| format!("Failed to serialize backlog to YAML: {}", e))?;

    let temp_file = NamedTempFile::new_in(parent)
        .map_err(|e| format!("Failed to create temp file in {}: {}", parent.display(), e))?;

    fs::write(temp_file.path(), &yaml).map_err(|e| format!("Failed to write temp file: {}", e))?;

    // sync to disk before rename
    let file = fs::File::open(temp_file.path())
        .map_err(|e| format!("Failed to open temp file for sync: {}", e))?;
    file.sync_all()
        .map_err(|e| format!("Failed to sync temp file: {}", e))?;

    temp_file
        .persist(path)
        .map_err(|e| format!("Failed to rename temp file to {}: {}", path.display(), e))?;

    Ok(())
}

/// Generate the next sequential ID for a backlog item.
///
/// Finds the highest numeric suffix across all items with the given prefix,
/// takes the max of that and `backlog.next_item_id` (high-water mark),
/// increments by 1, and returns the formatted ID and the new suffix.
/// Zero-pads to 3 digits minimum.
pub fn generate_next_id(backlog: &BacklogFile, prefix: &str) -> (String, u32) {
    let prefix_with_dash = format!("{}-", prefix);

    let max_num = backlog
        .items
        .iter()
        .filter_map(|item| {
            item.id
                .strip_prefix(&prefix_with_dash)
                .and_then(|suffix| suffix.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0)
        .max(backlog.next_item_id);

    let next = max_num + 1;
    (format!("{}-{:03}", prefix, next), next)
}

/// Create a new backlog item with the given title and optional size/risk.
///
/// The item is created with status `New`, a generated ID, and current timestamps.
/// Descriptions are set during triage, not at creation time.
pub fn add_item(
    backlog: &mut BacklogFile,
    title: &str,
    size: Option<SizeLevel>,
    risk: Option<DimensionLevel>,
    prefix: &str,
) -> BacklogItem {
    let (id, suffix) = generate_next_id(backlog, prefix);
    backlog.next_item_id = suffix;
    let now = chrono::Utc::now().to_rfc3339();

    let item = BacklogItem {
        id,
        title: title.to_string(),
        status: ItemStatus::New,
        size,
        risk,
        created: now.clone(),
        updated: now,
        ..Default::default()
    };

    backlog.items.push(item.clone());
    item
}

/// Transition an item's status, validating the transition is allowed.
///
/// For transitions to `Blocked`: saves the current status as `blocked_from_status`.
/// For transitions from `Blocked`: clears blocked fields.
pub fn transition_status(item: &mut BacklogItem, new_status: ItemStatus) -> Result<(), String> {
    if !item.status.is_valid_transition(&new_status) {
        return Err(format!(
            "Invalid status transition for {}: {:?} -> {:?}",
            item.id, item.status, new_status
        ));
    }

    if new_status == ItemStatus::Blocked {
        item.blocked_from_status = Some(item.status.clone());
    }

    if item.status == ItemStatus::Blocked {
        // Unblocking: clear all blocked fields
        item.blocked_from_status = None;
        item.blocked_reason = None;
        item.blocked_type = None;
        item.unblock_context = None;
    }

    item.status = new_status;
    item.updated = chrono::Utc::now().to_rfc3339();

    Ok(())
}

/// Merge non-None assessment fields from an UpdatedAssessments into an item.
pub fn update_assessments(item: &mut BacklogItem, assessments: &UpdatedAssessments) {
    if let Some(ref size) = assessments.size {
        item.size = Some(size.clone());
    }
    if let Some(ref complexity) = assessments.complexity {
        item.complexity = Some(complexity.clone());
    }
    if let Some(ref risk) = assessments.risk {
        item.risk = Some(risk.clone());
    }
    if let Some(ref impact) = assessments.impact {
        item.impact = Some(impact.clone());
    }
    item.updated = chrono::Utc::now().to_rfc3339();
}

/// Archive a completed item: prune from BACKLOG.yaml first, then write worklog entry.
///
/// Also removes the archived item's ID from all remaining items' dependency lists,
/// since the dependency is now satisfied (done/archived = met).
///
/// Crash safety: if the process crashes between pruning and writing, the item
/// stays in the backlog (safe — will be re-archived on next run).
pub fn archive_item(
    backlog: &mut BacklogFile,
    item_id: &str,
    backlog_path: &Path,
    worklog_path: &Path,
) -> Result<(), String> {
    let item_idx = backlog
        .items
        .iter()
        .position(|item| item.id == item_id)
        .ok_or_else(|| format!("Item {} not found in backlog", item_id))?;

    let item = backlog.items.remove(item_idx);

    // Strip the archived item's ID from all remaining dependency lists
    for remaining in &mut backlog.items {
        remaining.dependencies.retain(|dep| dep != item_id);
    }

    // Save backlog first (prune)
    save(backlog_path, backlog)?;

    // Write worklog entry
    write_archive_worklog_entry(worklog_path, &item)?;

    Ok(())
}

/// Ingest follow-ups from a phase result into the backlog as new items.
///
/// Each follow-up gets a generated ID, status `New`, and origin set to the
/// source item/phase that created it.
pub fn ingest_follow_ups(
    backlog: &mut BacklogFile,
    follow_ups: &[FollowUp],
    origin: &str,
    prefix: &str,
) -> Vec<BacklogItem> {
    let now = chrono::Utc::now().to_rfc3339();

    follow_ups
        .iter()
        .map(|fu| {
            let (id, suffix) = generate_next_id(backlog, prefix);
            backlog.next_item_id = suffix;
            let item = BacklogItem {
                id,
                title: fu.title.clone(),
                status: ItemStatus::New,
                size: fu.suggested_size.clone(),
                risk: fu.suggested_risk.clone(),
                origin: Some(origin.to_string()),
                created: now.clone(),
                updated: now.clone(),
                ..Default::default()
            };
            backlog.items.push(item.clone());
            item
        })
        .collect()
}

/// Load inbox items from a YAML file at the given path.
///
/// Expects a bare YAML sequence: `- title: ...\n- title: ...`
///
/// Returns `Ok(None)` if the file does not exist (normal path — no inbox pending).
/// Returns `Ok(Some(vec![]))` if the file is empty or whitespace-only.
/// Returns `Err` if the file exists but cannot be parsed.
pub fn load_inbox(inbox_path: &Path) -> Result<Option<Vec<InboxItem>>, String> {
    let contents = match fs::read_to_string(inbox_path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Failed to read {}: {}", inbox_path.display(), e)),
    };

    if contents.trim().is_empty() {
        return Ok(Some(vec![]));
    }

    let items: Vec<InboxItem> = serde_yaml_ng::from_str(&contents).map_err(|e| {
        format!(
            "Failed to parse inbox YAML from {}: {}. Expected a bare YAML sequence, e.g.:\n- title: \"My item\"\n  description: \"Details\"",
            inbox_path.display(),
            e
        )
    })?;

    Ok(Some(items))
}

/// Ingest inbox items into the backlog, creating BacklogItems with generated IDs.
///
/// Items with empty or whitespace-only titles are skipped (logged as warnings).
/// Returns the list of created BacklogItems.
pub fn ingest_inbox_items(
    backlog: &mut BacklogFile,
    items: &[InboxItem],
    prefix: &str,
) -> Vec<BacklogItem> {
    let now = chrono::Utc::now().to_rfc3339();

    items
        .iter()
        .filter_map(|inbox_item| {
            if inbox_item.title.trim().is_empty() {
                log_warn!("Skipping inbox item with empty title");
                return None;
            }

            let (id, suffix) = generate_next_id(backlog, prefix);
            backlog.next_item_id = suffix;

            let item = BacklogItem {
                id,
                title: inbox_item.title.clone(),
                description: inbox_item.description
                    .as_ref()
                    .filter(|d| !d.trim().is_empty())
                    .map(|d| StructuredDescription {
                        context: d.trim().to_string(),
                        ..Default::default()
                    }),
                status: ItemStatus::New,
                size: inbox_item.size.clone(),
                risk: inbox_item.risk.clone(),
                impact: inbox_item.impact.clone(),
                origin: Some("inbox".to_string()),
                dependencies: inbox_item.dependencies.clone(),
                created: now.clone(),
                updated: now.clone(),
                pipeline_type: inbox_item.pipeline_type.clone(),
                ..Default::default()
            };

            backlog.items.push(item.clone());
            Some(item)
        })
        .collect()
}

/// Delete the inbox file. Returns Ok(()) if the file does not exist.
pub fn clear_inbox(inbox_path: &Path) -> Result<(), String> {
    match fs::remove_file(inbox_path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Failed to delete {}: {}", inbox_path.display(), e)),
    }
}

/// Remove dependency references to IDs that no longer exist in the backlog.
///
/// This is a safety net for stale dependencies left behind by manual edits,
/// crashes between archive steps, or other edge cases. A missing dependency
/// ID means the item was archived (completed), so the dependency is satisfied.
///
/// Returns the number of stale references removed.
pub fn prune_stale_dependencies(backlog: &mut BacklogFile) -> usize {
    let all_ids: std::collections::HashSet<String> =
        backlog.items.iter().map(|item| item.id.clone()).collect();

    let mut pruned_count = 0;
    for item in &mut backlog.items {
        let before = item.dependencies.len();
        item.dependencies.retain(|dep| all_ids.contains(dep));
        pruned_count += before - item.dependencies.len();
    }

    pruned_count
}

/// Result of merging two backlog items.
#[derive(Debug)]
pub struct MergeResult {
    pub target_id: String,
    pub source_id: String,
}

/// Merge a source item into a target item, removing the source from the backlog.
///
/// - Appends source title + description context/problem/origin to target's description.context
/// - Union-merges source dependencies into target (dedup, no self-refs)
/// - Strips source ID from all remaining items' dependency lists
/// - Refreshes target's `updated` timestamp
///
/// Performs no disk I/O — caller is responsible for persisting changes.
pub fn merge_item(
    backlog: &mut BacklogFile,
    source_id: &str,
    target_id: &str,
) -> Result<MergeResult, String> {
    if source_id == target_id {
        return Err(format!("Cannot merge item {} into itself", source_id));
    }

    let source_idx = backlog
        .items
        .iter()
        .position(|i| i.id == source_id)
        .ok_or_else(|| format!("Source item {} not found in backlog", source_id))?;

    let _target_idx = backlog
        .items
        .iter()
        .position(|i| i.id == target_id)
        .ok_or_else(|| format!("Target item {} not found in backlog", target_id))?;

    // Remove source first
    let source = backlog.items.remove(source_idx);

    // Build merge context from source
    let mut merge_parts = vec![format!(
        "[Merged from {}] Title: {}",
        source_id, source.title
    )];
    if let Some(ref desc) = source.description {
        if !desc.context.is_empty() {
            merge_parts.push(format!("Context: {}", desc.context));
        }
        if !desc.problem.is_empty() {
            merge_parts.push(format!("Problem: {}", desc.problem));
        }
    }
    if let Some(ref origin) = source.origin {
        merge_parts.push(format!("Origin: {}", origin));
    }
    let merge_text = merge_parts.join(". ");

    // Find target (index may have shifted after remove)
    let target = backlog
        .items
        .iter_mut()
        .find(|i| i.id == target_id)
        .expect("target exists — validated above");

    // Append to target description.context
    let desc = target.description.get_or_insert_with(Default::default);
    if desc.context.is_empty() {
        desc.context = merge_text;
    } else {
        desc.context = format!("{}\n{}", desc.context, merge_text);
    }

    // Union-merge dependencies (dedup, no self-refs)
    for dep in &source.dependencies {
        if dep != target_id && dep != source_id && !target.dependencies.contains(dep) {
            target.dependencies.push(dep.clone());
        }
    }

    target.updated = chrono::Utc::now().to_rfc3339();

    // Strip source ID from all remaining items' dependency lists
    for item in &mut backlog.items {
        item.dependencies.retain(|dep| dep != source_id);
    }

    Ok(MergeResult {
        target_id: target_id.to_string(),
        source_id: source_id.to_string(),
    })
}

// --- Internal helpers ---

/// Append a worklog entry for an archived item.
fn write_archive_worklog_entry(worklog_path: &Path, item: &BacklogItem) -> Result<(), String> {
    let parent = worklog_path.parent().ok_or_else(|| {
        format!(
            "Cannot determine parent directory of {}",
            worklog_path.display()
        )
    })?;

    fs::create_dir_all(parent).map_err(|e| {
        format!(
            "Failed to create worklog directory {}: {}",
            parent.display(),
            e
        )
    })?;

    let now = chrono::Utc::now().to_rfc3339();
    let phase_str = item.phase.as_deref().unwrap_or("N/A");
    let entry = format!(
        "## {} — {} ({})\n\n- **Status:** Done\n- **Phase:** {}\n\n---\n\n",
        now, item.id, item.title, phase_str,
    );

    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(worklog_path)
        .map_err(|e| {
            format!(
                "Failed to open worklog at {}: {}",
                worklog_path.display(),
                e
            )
        })?;

    file.write_all(entry.as_bytes()).map_err(|e| {
        format!(
            "Failed to write worklog at {}: {}",
            worklog_path.display(),
            e
        )
    })?;

    Ok(())
}
