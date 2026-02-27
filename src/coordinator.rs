use std::path::{Path, PathBuf};
use std::time::Duration;

use task_golem::model::item::Item;
use task_golem::store::Store;
use tokio::sync::{mpsc, oneshot};

use crate::git::StatusEntry;
use crate::pg_error::PgError;
use crate::pg_item::{self, PgItem};
use crate::types::{FollowUp, ItemStatus, ItemUpdate, PhaseResult, StructuredDescription};
use crate::{log_error, log_warn};

// --- Aliases for task-golem git module (distinguished from phase-golem's own git) ---
use task_golem::git as tg_git;
use task_golem::model::id::generate_id_with_prefix;

// --- Command enum ---

pub enum CoordinatorCommand {
    GetSnapshot {
        reply: oneshot::Sender<Result<Vec<PgItem>, PgError>>,
    },
    UpdateItem {
        id: String,
        update: ItemUpdate,
        reply: oneshot::Sender<Result<(), PgError>>,
    },
    CompletePhase {
        item_id: String,
        result: Box<PhaseResult>,
        is_destructive: bool,
        reply: oneshot::Sender<Result<(), PgError>>,
    },
    BatchCommit {
        reply: oneshot::Sender<Result<(), PgError>>,
    },
    GetHeadSha {
        reply: oneshot::Sender<Result<String, PgError>>,
    },
    IsAncestor {
        sha: String,
        reply: oneshot::Sender<Result<bool, PgError>>,
    },
    RecordPhaseStart {
        item_id: String,
        commit_sha: String,
        reply: oneshot::Sender<Result<(), PgError>>,
    },
    WriteWorklog {
        id: String,
        title: String,
        phase: String,
        outcome: String,
        summary: String,
        reply: oneshot::Sender<Result<(), PgError>>,
    },
    ArchiveItem {
        item_id: String,
        reply: oneshot::Sender<Result<(), PgError>>,
    },
    IngestFollowUps {
        follow_ups: Vec<FollowUp>,
        origin: String,
        reply: oneshot::Sender<Result<Vec<String>, PgError>>,
    },
    UnblockItem {
        item_id: String,
        context: Option<String>,
        reply: oneshot::Sender<Result<(), PgError>>,
    },
    MergeItem {
        source_id: String,
        target_id: String,
        reply: oneshot::Sender<Result<(), PgError>>,
    },
}

// --- CoordinatorHandle ---

#[derive(Clone)]
pub struct CoordinatorHandle {
    sender: mpsc::Sender<CoordinatorCommand>,
}

impl CoordinatorHandle {
    async fn send_command<T>(
        &self,
        command: CoordinatorCommand,
        rx: oneshot::Receiver<T>,
    ) -> Result<T, PgError> {
        self.sender
            .send(command)
            .await
            .map_err(|_| PgError::InternalPanic("coordinator shut down".to_string()))?;
        rx.await
            .map_err(|_| PgError::InternalPanic("coordinator dropped reply".to_string()))
    }

    pub async fn get_snapshot(&self) -> Result<Vec<PgItem>, PgError> {
        let (reply, rx) = oneshot::channel();
        self.send_command(CoordinatorCommand::GetSnapshot { reply }, rx)
            .await?
    }

    pub async fn update_item(&self, id: &str, update: ItemUpdate) -> Result<(), PgError> {
        let (reply, rx) = oneshot::channel();
        self.send_command(
            CoordinatorCommand::UpdateItem {
                id: id.to_string(),
                update,
                reply,
            },
            rx,
        )
        .await?
    }

    pub async fn complete_phase(
        &self,
        item_id: &str,
        result: PhaseResult,
        is_destructive: bool,
    ) -> Result<(), PgError> {
        let (reply, rx) = oneshot::channel();
        self.send_command(
            CoordinatorCommand::CompletePhase {
                item_id: item_id.to_string(),
                result: Box::new(result),
                is_destructive,
                reply,
            },
            rx,
        )
        .await?
    }

    pub async fn batch_commit(&self) -> Result<(), PgError> {
        let (reply, rx) = oneshot::channel();
        self.send_command(CoordinatorCommand::BatchCommit { reply }, rx)
            .await?
    }

    pub async fn get_head_sha(&self) -> Result<String, PgError> {
        let (reply, rx) = oneshot::channel();
        self.send_command(CoordinatorCommand::GetHeadSha { reply }, rx)
            .await?
    }

    pub async fn is_ancestor(&self, sha: &str) -> Result<bool, PgError> {
        let (reply, rx) = oneshot::channel();
        self.send_command(
            CoordinatorCommand::IsAncestor {
                sha: sha.to_string(),
                reply,
            },
            rx,
        )
        .await?
    }

    pub async fn record_phase_start(&self, item_id: &str, commit_sha: &str) -> Result<(), PgError> {
        let (reply, rx) = oneshot::channel();
        self.send_command(
            CoordinatorCommand::RecordPhaseStart {
                item_id: item_id.to_string(),
                commit_sha: commit_sha.to_string(),
                reply,
            },
            rx,
        )
        .await?
    }

    pub async fn write_worklog(
        &self,
        id: &str,
        title: &str,
        phase: &str,
        outcome: &str,
        summary: &str,
    ) -> Result<(), PgError> {
        let (reply, rx) = oneshot::channel();
        self.send_command(
            CoordinatorCommand::WriteWorklog {
                id: id.to_string(),
                title: title.to_string(),
                phase: phase.to_string(),
                outcome: outcome.to_string(),
                summary: summary.to_string(),
                reply,
            },
            rx,
        )
        .await?
    }

    pub async fn archive_item(&self, item_id: &str) -> Result<(), PgError> {
        let (reply, rx) = oneshot::channel();
        self.send_command(
            CoordinatorCommand::ArchiveItem {
                item_id: item_id.to_string(),
                reply,
            },
            rx,
        )
        .await?
    }

    pub async fn ingest_follow_ups(
        &self,
        follow_ups: Vec<FollowUp>,
        origin: &str,
    ) -> Result<Vec<String>, PgError> {
        let (reply, rx) = oneshot::channel();
        self.send_command(
            CoordinatorCommand::IngestFollowUps {
                follow_ups,
                origin: origin.to_string(),
                reply,
            },
            rx,
        )
        .await?
    }

    pub async fn unblock_item(
        &self,
        item_id: &str,
        context: Option<String>,
    ) -> Result<(), PgError> {
        let (reply, rx) = oneshot::channel();
        self.send_command(
            CoordinatorCommand::UnblockItem {
                item_id: item_id.to_string(),
                context,
                reply,
            },
            rx,
        )
        .await?
    }

    pub async fn merge_item(&self, source_id: &str, target_id: &str) -> Result<(), PgError> {
        let (reply, rx) = oneshot::channel();
        self.send_command(
            CoordinatorCommand::MergeItem {
                source_id: source_id.to_string(),
                target_id: target_id.to_string(),
                reply,
            },
            rx,
        )
        .await?
    }
}

// --- Pure helpers ---

fn has_staged_changes(status: &[StatusEntry]) -> bool {
    status.iter().any(|entry| {
        entry
            .status_code
            .starts_with(|c: char| c != ' ' && c != '?')
    })
}

fn build_phase_commit_message(item_id: &str, phase: &str, commit_summary: Option<&str>) -> String {
    let prefix = format!("[{}][{}]", item_id, phase);
    match commit_summary {
        Some(s) => {
            // Strip duplicate prefix if the agent already included it
            let trimmed = s
                .strip_prefix(&prefix)
                .map(|rest| rest.trim_start())
                .unwrap_or(s);
            format!("{} {}", prefix, trimmed)
        }
        None => format!("{} Phase output", prefix),
    }
}

fn build_batch_commit_message(phases: &[(String, String, Option<String>)]) -> String {
    // Single-phase batch: use same format as a direct phase commit
    if phases.len() == 1 {
        let (id, phase, summary) = &phases[0];
        return build_phase_commit_message(id, phase, summary.as_deref());
    }

    let label_parts: Vec<String> = phases
        .iter()
        .map(|(id, phase, _)| format!("[{}][{}]", id, phase))
        .collect();
    let label = format!("{} Phase outputs", label_parts.join(""));

    let summaries: Vec<String> = phases
        .iter()
        .filter_map(|(id, phase, summary)| {
            summary.as_ref().map(|s| {
                let prefix = format!("[{}][{}]", id, phase);
                let trimmed = s
                    .strip_prefix(&prefix)
                    .map(|rest| rest.trim_start())
                    .unwrap_or(s);
                format!("{} {}", prefix, trimmed)
            })
        })
        .collect();

    if summaries.is_empty() {
        label
    } else {
        // Summaries first (visible in git log --oneline), generic label in body
        format!("{}\n\n{}", summaries.join(" | "), label)
    }
}

/// Build merge context text from a source Item for appending to the target's description.
fn build_merge_context(source: &Item) -> String {
    let mut merge_parts = vec![format!(
        "[Merged from {}] Title: {}",
        source.id, source.title
    )];

    let pg_source = PgItem(source.clone());
    if let Some(desc) = pg_source.structured_description() {
        if !desc.context.is_empty() {
            merge_parts.push(format!("Context: {}", desc.context));
        }
        if !desc.problem.is_empty() {
            merge_parts.push(format!("Problem: {}", desc.problem));
        }
    } else if let Some(ref native_desc) = source.description {
        if !native_desc.is_empty() {
            merge_parts.push(format!("Context: {}", native_desc));
        }
    }

    if let Some(origin) = pg_source.origin() {
        merge_parts.push(format!("Origin: {}", origin));
    }

    merge_parts.join(". ")
}

// --- Retry helper ---

/// Maximum total attempts for store operations (1 initial + 2 retries).
const MAX_STORE_ATTEMPTS: u32 = 3;
/// Backoff duration between retry attempts.
const RETRY_BACKOFF: Duration = Duration::from_secs(1);

/// Execute a store operation with retry for LockTimeout errors.
///
/// The closure receives a cloned `Store` and returns `Result<T, PgError>`.
/// Retry wraps the entire `spawn_blocking` call (blocking thread freed between retries).
/// Non-retryable errors return immediately.
async fn with_store_retry<F, T>(store: &Store, f: F) -> Result<T, PgError>
where
    F: Fn(Store) -> Result<T, PgError> + Send + 'static + Clone,
    T: Send + std::fmt::Debug + 'static,
{
    let mut last_error: Option<PgError> = None;

    for attempt in 0..MAX_STORE_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(RETRY_BACKOFF).await;
        }

        let store_clone = store.clone();
        let f_clone = f.clone();

        let join_result = tokio::task::spawn_blocking(move || f_clone(store_clone)).await;

        let result = match join_result {
            Ok(r) => r,
            Err(e) => return Err(PgError::InternalPanic(format!("{e:?}"))),
        };

        match result {
            Ok(val) => return Ok(val),
            Err(ref e) if e.is_retryable() => {
                log_warn!(
                    "Store operation failed (attempt {}/{}): {}",
                    attempt + 1,
                    MAX_STORE_ATTEMPTS,
                    e
                );
                last_error = Some(result.unwrap_err());
            }
            Err(e) => return Err(e),
        }
    }

    Err(last_error
        .unwrap_or_else(|| PgError::InternalPanic("retry exhausted with no error".to_string())))
}

// --- Actor implementation ---

const CHANNEL_CAPACITY: usize = 32;

struct CoordinatorState {
    store: Store,
    project_root: PathBuf,
    prefix: String,
    /// Tracks non-destructive phase completions pending batch commit.
    /// Each entry: (item_id, phase, commit_summary).
    pending_batch_phases: Vec<(String, String, Option<String>)>,
}

impl CoordinatorState {
    fn worklog_dir(&self) -> PathBuf {
        self.project_root.join("_worklog")
    }
}

// --- Handler implementations ---

async fn handle_get_snapshot(state: &CoordinatorState) -> Result<Vec<PgItem>, PgError> {
    let store = state.store.clone();
    let items = tokio::task::spawn_blocking(move || store.load_active())
        .await
        .map_err(|e| PgError::InternalPanic(format!("{e:?}")))?
        .map_err(PgError::from)?;

    Ok(items.into_iter().map(PgItem).collect())
}

async fn handle_update_item(
    state: &CoordinatorState,
    id: String,
    update: ItemUpdate,
) -> Result<(), PgError> {
    with_store_retry(&state.store, move |store| {
        store
            .with_lock(|s| {
                let mut items = s.load_active()?;
                let idx = items
                    .iter()
                    .position(|i| i.id == id)
                    .ok_or_else(|| task_golem::errors::TgError::ItemNotFound(id.clone()))?;
                pg_item::apply_update(&mut items[idx], update.clone());
                s.save_active(&items)
            })
            .map_err(PgError::from)
    })
    .await
}

async fn handle_record_phase_start(
    state: &CoordinatorState,
    item_id: String,
    commit_sha: String,
) -> Result<(), PgError> {
    with_store_retry(&state.store, move |store| {
        store
            .with_lock(|s| {
                let mut items = s.load_active()?;
                let idx = items
                    .iter()
                    .position(|i| i.id == item_id)
                    .ok_or_else(|| task_golem::errors::TgError::ItemNotFound(item_id.clone()))?;
                pg_item::set_last_phase_commit(&mut items[idx], Some(&commit_sha));
                s.save_active(&items)
            })
            .map_err(PgError::from)
    })
    .await
}

fn handle_write_worklog(
    state: &CoordinatorState,
    id: &str,
    title: &str,
    phase: &str,
    outcome: &str,
    summary: &str,
) -> Result<(), PgError> {
    crate::worklog::write_entry(&state.worklog_dir(), id, title, phase, outcome, summary)
        .map_err(PgError::Git)
}

async fn handle_archive_item(state: &CoordinatorState, item_id: String) -> Result<(), PgError> {
    let worklog_dir = state.worklog_dir();

    // Store operation: find item, archive it, remove from active, save
    let archived_item = with_store_retry(&state.store, move |store| {
        store
            .with_lock(|s| {
                let mut items = s.load_active()?;
                let idx = items
                    .iter()
                    .position(|i| i.id == item_id)
                    .ok_or_else(|| task_golem::errors::TgError::ItemNotFound(item_id.clone()))?;

                let item = items.remove(idx);
                s.append_to_archive(&item)?;
                s.save_active(&items)?;
                Ok(item)
            })
            .map_err(PgError::from)
    })
    .await?;

    // Write worklog entry outside the lock
    let worklog_month = chrono::Utc::now().format("%Y-%m").to_string();
    let worklog_path = worklog_dir.join(format!("{}.md", worklog_month));

    write_archive_worklog_entry(&worklog_path, &archived_item)
        .map_err(|e| PgError::Git(format!("Worklog write failed: {}", e)))?;

    Ok(())
}

/// Write an archive worklog entry for a completed/archived item.
fn write_archive_worklog_entry(worklog_path: &Path, item: &Item) -> Result<(), String> {
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    let worklog_dir = worklog_path
        .parent()
        .ok_or_else(|| "Cannot determine worklog directory".to_string())?;

    fs::create_dir_all(worklog_dir).map_err(|e| {
        format!(
            "Failed to create worklog directory {}: {}",
            worklog_dir.display(),
            e
        )
    })?;

    let pg = PgItem(item.clone());
    let datetime = chrono::Utc::now().to_rfc3339();
    let phase = pg.phase().unwrap_or_else(|| "unknown".to_string());

    let entry = format!(
        "## {} — {} ({})\n\n- **Phase:** {}\n- **Outcome:** Archived\n- **Summary:** Item archived\n\n---\n\n",
        datetime, item.id, item.title, phase,
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

async fn handle_ingest_follow_ups(
    state: &CoordinatorState,
    follow_ups: Vec<FollowUp>,
    origin: String,
    prefix: String,
) -> Result<Vec<String>, PgError> {
    if follow_ups.is_empty() {
        return Ok(vec![]);
    }

    with_store_retry(&state.store, move |store| {
        store
            .with_lock(|s| {
                let mut items = s.load_active()?;
                let known_ids = s.all_known_ids()?;

                let mut new_ids = Vec::new();
                let mut current_known = known_ids;

                for fu in &follow_ups {
                    let id =
                        generate_id_with_prefix(&current_known, &prefix).map_err(|e| match e {
                            task_golem::errors::TgError::IdCollisionExhausted(n) => {
                                task_golem::errors::TgError::IdCollisionExhausted(n)
                            }
                            other => other,
                        })?;

                    current_known.insert(id.clone());

                    let mut pg = pg_item::new_from_parts(
                        id.clone(),
                        fu.title.clone(),
                        ItemStatus::New,
                        vec![],
                        vec![],
                    );

                    // Set origin
                    pg_item::set_origin(&mut pg.0, Some(&origin));

                    // Set suggested assessments if provided
                    if let Some(ref size) = fu.suggested_size {
                        pg_item::set_size(&mut pg.0, Some(size));
                    }
                    if let Some(ref risk) = fu.suggested_risk {
                        pg_item::set_risk(&mut pg.0, Some(risk));
                    }

                    // Set context as structured description if provided
                    if let Some(ref context) = fu.context {
                        let desc = StructuredDescription {
                            context: context.clone(),
                            problem: String::new(),
                            solution: String::new(),
                            impact: String::new(),
                            sizing_rationale: String::new(),
                        };
                        pg_item::set_structured_description(&mut pg.0, Some(&desc));
                    }

                    new_ids.push(id);
                    items.push(pg.0);
                }

                s.save_active(&items)?;
                Ok(new_ids)
            })
            .map_err(PgError::from)
    })
    .await
}

async fn handle_unblock_item(
    state: &CoordinatorState,
    item_id: String,
    context: Option<String>,
) -> Result<(), PgError> {
    with_store_retry(&state.store, move |store| {
        store
            .with_lock(|s| {
                let mut items = s.load_active()?;
                let idx = items
                    .iter()
                    .position(|i| i.id == item_id)
                    .ok_or_else(|| task_golem::errors::TgError::ItemNotFound(item_id.clone()))?;

                let pg = PgItem(items[idx].clone());
                if pg.pg_status() != ItemStatus::Blocked {
                    return Err(task_golem::errors::TgError::InvalidTransition {
                        from: items[idx].status,
                        to: task_golem::model::status::Status::Todo,
                    });
                }

                // Read the blocked_from_status before clearing
                let restore_to = pg.pg_blocked_from_status().unwrap_or(ItemStatus::New);

                // Clear all blocked fields (extension and native)
                pg_item::set_blocked_from_status(&mut items[idx], None);
                items[idx].blocked_reason = None;
                items[idx].blocked_from_status = None;
                pg_item::set_blocked_type(&mut items[idx], None);
                pg_item::set_unblock_context(&mut items[idx], None);

                // Set unblock context if provided
                if let Some(ref ctx) = context {
                    pg_item::set_unblock_context(&mut items[idx], Some(ctx));
                }

                // Restore to the saved status
                pg_item::set_pg_status(&mut items[idx], restore_to);

                // Reset last_phase_commit for staleness-blocked items
                pg_item::set_last_phase_commit(&mut items[idx], None);

                s.save_active(&items)
            })
            .map_err(PgError::from)
    })
    .await
}

async fn handle_merge_item(
    state: &CoordinatorState,
    source_id: String,
    target_id: String,
) -> Result<(), PgError> {
    if source_id == target_id {
        return Err(PgError::CycleDetected(format!(
            "Cannot merge item {} into itself",
            source_id
        )));
    }

    with_store_retry(&state.store, move |store| {
        store
            .with_lock(|s| {
                let mut items = s.load_active()?;

                let source_idx = items
                    .iter()
                    .position(|i| i.id == source_id)
                    .ok_or_else(|| {
                        task_golem::errors::TgError::ItemNotFound(format!(
                            "Source item {} not found",
                            source_id
                        ))
                    })?;

                let _target_idx =
                    items
                        .iter()
                        .position(|i| i.id == target_id)
                        .ok_or_else(|| {
                            task_golem::errors::TgError::ItemNotFound(format!(
                                "Target item {} not found",
                                target_id
                            ))
                        })?;

                // Remove source first
                let source = items.remove(source_idx);

                // Build merge context from source
                let merge_text = build_merge_context(&source);

                // Find target (index may have shifted after remove)
                let target = items
                    .iter_mut()
                    .find(|i| i.id == target_id)
                    .expect("target exists — validated above");

                // Append merge context to target description
                let pg_target = PgItem(target.clone());
                let mut desc = pg_target.structured_description().unwrap_or_default();

                if desc.context.is_empty() {
                    desc.context = merge_text;
                } else {
                    desc.context = format!("{}\n{}", desc.context, merge_text);
                }
                pg_item::set_structured_description(target, Some(&desc));

                // Union-merge dependencies (dedup, no self-refs)
                let source_deps = source.dependencies.clone();
                for dep in &source_deps {
                    if dep != &target_id && dep != &source_id && !target.dependencies.contains(dep)
                    {
                        target.dependencies.push(dep.clone());
                    }
                }

                target.updated_at = chrono::Utc::now();

                // Strip source ID from all remaining items' dependency lists
                for item in &mut items {
                    item.dependencies.retain(|dep| dep != &source_id);
                }

                // Archive the source
                s.append_to_archive(&source)?;

                s.save_active(&items)
            })
            .map_err(PgError::from)
    })
    .await
}

// --- Actor loop ---

async fn run_coordinator(
    mut rx: mpsc::Receiver<CoordinatorCommand>,
    store: Store,
    project_root: PathBuf,
    prefix: String,
) {
    // Startup probe: verify the store is accessible
    match store.load_active() {
        Ok(_) => {
            // Check for uncommitted changes as a warning
            let project_root_for_check = project_root.clone();
            if let Ok(output) = std::process::Command::new("git")
                .args(["status", "--porcelain", ".task-golem/tasks.jsonl"])
                .current_dir(&project_root_for_check)
                .output()
            {
                let status_text = String::from_utf8_lossy(&output.stdout);
                if !status_text.trim().is_empty() {
                    log_warn!(
                        "tasks.jsonl has uncommitted changes — run `git add .task-golem/ && git commit -m 'recovery'` or `git checkout .task-golem/tasks.jsonl` to resolve."
                    );
                }
            }
        }
        Err(ref e) if matches!(e, task_golem::errors::TgError::NotInitialized(_)) => {
            log_error!("Store not initialized: {}. Run `tg init` first.", e);
            // The coordinator will still start but GetSnapshot etc. will fail
        }
        Err(ref e)
            if matches!(
                e,
                task_golem::errors::TgError::StorageCorruption(_)
                    | task_golem::errors::TgError::SchemaVersionUnsupported { .. }
            ) =>
        {
            log_error!("Storage corruption detected on startup: {}. Recovery: `git checkout .task-golem/tasks.jsonl`", e);
            // Coordinator starts but operations will fail
        }
        Err(e) => {
            log_error!("Unexpected error during startup probe: {}", e);
        }
    }

    let mut state = CoordinatorState {
        store,
        project_root,
        prefix,
        pending_batch_phases: Vec::new(),
    };

    while let Some(cmd) = rx.recv().await {
        let is_fatal_result: Option<bool>;

        match cmd {
            CoordinatorCommand::GetSnapshot { reply } => {
                let result = handle_get_snapshot(&state).await;
                is_fatal_result = result.as_ref().err().map(|e| e.is_fatal());
                let _ = reply.send(result);
            }
            CoordinatorCommand::UpdateItem { id, update, reply } => {
                let result = handle_update_item(&state, id, update).await;
                is_fatal_result = result.as_ref().err().map(|e| e.is_fatal());
                let _ = reply.send(result);
            }
            CoordinatorCommand::CompletePhase {
                item_id,
                result: phase_result,
                is_destructive,
                reply,
            } => {
                let project_root = state.project_root.clone();
                // Clone for potential pending_batch_phases.push after .await
                let item_id_for_push = item_id.clone();
                let phase_for_push = phase_result.phase.clone();
                let commit_summary_for_push = phase_result.commit_summary.clone();

                // Step 1: Stage artifact files via phase-golem's git module
                let staging_result: Result<(), PgError> = {
                    let project_root_clone = project_root.clone();
                    match tokio::task::spawn_blocking(move || {
                        let status = crate::git::get_status(Some(&project_root_clone))
                            .map_err(PgError::Git)?;
                        let dirty_paths: Vec<PathBuf> = status
                            .iter()
                            .map(|entry| project_root_clone.join(&entry.path))
                            .collect();

                        if !dirty_paths.is_empty() {
                            let path_refs: Vec<&Path> =
                                dirty_paths.iter().map(|p| p.as_path()).collect();
                            crate::git::stage_paths(&path_refs, Some(&project_root_clone))
                                .map_err(PgError::Git)?;
                        }

                        Ok(())
                    })
                    .await
                    {
                        Ok(r) => r,
                        Err(e) => Err(PgError::InternalPanic(format!("{e:?}"))),
                    }
                };

                if let Err(e) = staging_result {
                    // Staging failed — abort without JSONL update
                    is_fatal_result = Some(e.is_fatal());
                    let _ = reply.send(Err(e));
                    // Check fatal below
                    if is_fatal_result == Some(true) {
                        break;
                    }
                    continue;
                }

                // Step 2: Update item state in store via with_lock
                let store_result = {
                    with_store_retry(&state.store, move |store| {
                        store
                            .with_lock(|s| {
                                let items = s.load_active()?;
                                // Item update is handled by the caller after CompletePhase
                                // CompletePhase itself just stages + commits; item state updates
                                // happen via separate UpdateItem calls in the executor
                                s.save_active(&items)
                            })
                            .map_err(PgError::from)
                    })
                    .await
                };

                if let Err(e) = store_result {
                    is_fatal_result = Some(e.is_fatal());
                    let _ = reply.send(Err(e));
                    if is_fatal_result == Some(true) {
                        break;
                    }
                    continue;
                }

                // Step 3: stage task-golem files + commit (for destructive) or accumulate batch
                if is_destructive {
                    let project_root_clone = project_root.clone();
                    let commit_result: Result<(), PgError> =
                        match tokio::task::spawn_blocking(move || {
                            tg_git::stage_self(&project_root_clone)
                                .map_err(|e| PgError::Git(format!("stage_self failed: {}", e)))?;

                            let message = build_phase_commit_message(
                                &item_id,
                                &phase_result.phase,
                                phase_result.commit_summary.as_deref(),
                            );

                            let post_status = crate::git::get_status(Some(&project_root_clone))
                                .map_err(PgError::Git)?;

                            if has_staged_changes(&post_status) {
                                tg_git::commit(&message, &project_root_clone)
                                    .map_err(|e| PgError::Git(format!("commit failed: {}", e)))?;
                            }

                            Ok(())
                        })
                        .await
                        {
                            Ok(r) => r,
                            Err(e) => Err(PgError::InternalPanic(format!("{e:?}"))),
                        };

                    if let Err(ref e) = commit_result {
                        // JSONL state is authoritative — git commit is best-effort
                        log_warn!("CompletePhase commit failed (JSONL state preserved): {}", e);
                    }

                    is_fatal_result = None;
                    // Return success even if commit failed — JSONL is authoritative
                    let _ = reply.send(Ok(()));
                } else {
                    // Non-destructive: stage task-golem files and accumulate
                    let project_root_clone = project_root.clone();
                    let stage_result: Result<(), PgError> =
                        match tokio::task::spawn_blocking(move || {
                            tg_git::stage_self(&project_root_clone)
                                .map_err(|e| PgError::Git(format!("stage_self failed: {}", e)))?;
                            Ok(())
                        })
                        .await
                        {
                            Ok(r) => r,
                            Err(e) => Err(PgError::InternalPanic(format!("{e:?}"))),
                        };

                    if let Err(ref e) = stage_result {
                        log_warn!("CompletePhase staging failed: {}", e);
                    }

                    state.pending_batch_phases.push((
                        item_id_for_push,
                        phase_for_push,
                        commit_summary_for_push,
                    ));

                    is_fatal_result = None;
                    let _ = reply.send(Ok(()));
                }
            }
            CoordinatorCommand::BatchCommit { reply } => {
                if state.pending_batch_phases.is_empty() {
                    is_fatal_result = None;
                    let _ = reply.send(Ok(()));
                } else {
                    let project_root = state.project_root.clone();
                    let pending_batch_phases = state.pending_batch_phases.clone();

                    let result: Result<(), PgError> = match tokio::task::spawn_blocking(move || {
                        tg_git::stage_self(&project_root)
                            .map_err(|e| PgError::Git(format!("stage_self failed: {}", e)))?;

                        let status =
                            crate::git::get_status(Some(&project_root)).map_err(PgError::Git)?;

                        if has_staged_changes(&status) {
                            let message = build_batch_commit_message(&pending_batch_phases);
                            tg_git::commit(&message, &project_root)
                                .map_err(|e| PgError::Git(format!("commit failed: {}", e)))?;
                        }

                        Ok(())
                    })
                    .await
                    {
                        Ok(r) => r,
                        Err(e) => Err(PgError::InternalPanic(format!("{e:?}"))),
                    };

                    if result.is_ok() {
                        state.pending_batch_phases.clear();
                    }

                    is_fatal_result = result.as_ref().err().map(|e| e.is_fatal());
                    let _ = reply.send(result);
                }
            }
            CoordinatorCommand::GetHeadSha { reply } => {
                let project_root = state.project_root.clone();
                let result: Result<String, PgError> = match tokio::task::spawn_blocking(move || {
                    crate::git::get_head_sha(&project_root).map_err(PgError::Git)
                })
                .await
                {
                    Ok(r) => r,
                    Err(e) => Err(PgError::InternalPanic(format!("{e:?}"))),
                };
                is_fatal_result = result.as_ref().err().map(|e| e.is_fatal());
                let _ = reply.send(result);
            }
            CoordinatorCommand::IsAncestor { sha, reply } => {
                let project_root = state.project_root.clone();
                let result: Result<bool, PgError> = match tokio::task::spawn_blocking(move || {
                    crate::git::is_ancestor(&sha, &project_root).map_err(PgError::Git)
                })
                .await
                {
                    Ok(r) => r,
                    Err(e) => Err(PgError::InternalPanic(format!("{e:?}"))),
                };
                is_fatal_result = result.as_ref().err().map(|e| e.is_fatal());
                let _ = reply.send(result);
            }
            CoordinatorCommand::RecordPhaseStart {
                item_id,
                commit_sha,
                reply,
            } => {
                let result = handle_record_phase_start(&state, item_id, commit_sha).await;
                is_fatal_result = result.as_ref().err().map(|e| e.is_fatal());
                let _ = reply.send(result);
            }
            CoordinatorCommand::WriteWorklog {
                id,
                title,
                phase,
                outcome,
                summary,
                reply,
            } => {
                let result = handle_write_worklog(&state, &id, &title, &phase, &outcome, &summary);
                is_fatal_result = result.as_ref().err().map(|e| e.is_fatal());
                let _ = reply.send(result);
            }
            CoordinatorCommand::ArchiveItem { item_id, reply } => {
                let result = handle_archive_item(&state, item_id).await;
                is_fatal_result = result.as_ref().err().map(|e| e.is_fatal());
                let _ = reply.send(result);
            }
            CoordinatorCommand::IngestFollowUps {
                follow_ups,
                origin,
                reply,
            } => {
                let result =
                    handle_ingest_follow_ups(&state, follow_ups, origin, state.prefix.clone())
                        .await;
                is_fatal_result = result.as_ref().err().map(|e| e.is_fatal());
                let _ = reply.send(result);
            }
            CoordinatorCommand::UnblockItem {
                item_id,
                context,
                reply,
            } => {
                let result = handle_unblock_item(&state, item_id, context).await;
                is_fatal_result = result.as_ref().err().map(|e| e.is_fatal());
                let _ = reply.send(result);
            }
            CoordinatorCommand::MergeItem {
                source_id,
                target_id,
                reply,
            } => {
                let result = handle_merge_item(&state, source_id, target_id).await;
                is_fatal_result = result.as_ref().err().map(|e| e.is_fatal());
                let _ = reply.send(result);
            }
        }

        // Fatal error propagation: break out of the handler loop
        if is_fatal_result == Some(true) {
            log_error!("Fatal coordinator error — shutting down handler loop");
            break;
        }
    }

    // Shutdown: no in-memory state to save (all state is in task-golem store)
}

// --- Spawn ---

pub fn spawn_coordinator(
    store: Store,
    project_root: PathBuf,
    prefix: String,
) -> (CoordinatorHandle, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);

    let task_handle = tokio::spawn(run_coordinator(rx, store, project_root, prefix));

    (CoordinatorHandle { sender: tx }, task_handle)
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // build_phase_commit_message tests
    // =========================================================================

    #[test]
    fn phase_commit_message_no_summary() {
        let msg = build_phase_commit_message("WRK-001", "build", None);
        assert_eq!(msg, "[WRK-001][build] Phase output");
    }

    #[test]
    fn phase_commit_message_plain_summary() {
        let msg = build_phase_commit_message("WRK-001", "build", Some("Add login form"));
        assert_eq!(msg, "[WRK-001][build] Add login form");
    }

    #[test]
    fn phase_commit_message_strips_duplicate_prefix() {
        let msg = build_phase_commit_message(
            "WRK-051",
            "triage",
            Some("[WRK-051][triage] Assess inbox creation"),
        );
        assert_eq!(msg, "[WRK-051][triage] Assess inbox creation");
    }

    #[test]
    fn phase_commit_message_does_not_strip_different_prefix() {
        let msg =
            build_phase_commit_message("WRK-001", "build", Some("[WRK-002][design] Wrong prefix"));
        assert_eq!(msg, "[WRK-001][build] [WRK-002][design] Wrong prefix");
    }

    // =========================================================================
    // build_batch_commit_message tests
    // =========================================================================

    #[test]
    fn batch_commit_message_no_summaries() {
        let phases = vec![
            ("WRK-001".to_string(), "build".to_string(), None),
            ("WRK-002".to_string(), "design".to_string(), None),
        ];
        let msg = build_batch_commit_message(&phases);
        assert_eq!(msg, "[WRK-001][build][WRK-002][design] Phase outputs");
    }

    #[test]
    fn batch_commit_message_with_summaries() {
        let phases = vec![
            (
                "WRK-001".to_string(),
                "build".to_string(),
                Some("Add form".to_string()),
            ),
            (
                "WRK-002".to_string(),
                "design".to_string(),
                Some("Layout update".to_string()),
            ),
        ];
        let msg = build_batch_commit_message(&phases);
        assert_eq!(
            msg,
            "[WRK-001][build] Add form | [WRK-002][design] Layout update\n\n[WRK-001][build][WRK-002][design] Phase outputs"
        );
    }

    #[test]
    fn batch_commit_single_phase_delegates_to_phase_message() {
        let phases = vec![(
            "WRK-051".to_string(),
            "triage".to_string(),
            Some("[WRK-051][triage] Assess inbox".to_string()),
        )];
        let msg = build_batch_commit_message(&phases);
        assert_eq!(msg, "[WRK-051][triage] Assess inbox");
    }

    #[test]
    fn batch_commit_single_phase_no_summary() {
        let phases = vec![("WRK-001".to_string(), "build".to_string(), None)];
        let msg = build_batch_commit_message(&phases);
        assert_eq!(msg, "[WRK-001][build] Phase output");
    }

    // =========================================================================
    // spawn_coordinator tests
    // =========================================================================

    #[tokio::test]
    async fn spawn_coordinator_returns_joinhandle() {
        let dir = tempfile::tempdir().expect("create tempdir");
        let tg_dir = dir.path().join(".task-golem");
        std::fs::create_dir_all(&tg_dir).expect("create .task-golem");
        let store = Store::new(tg_dir);
        store.save_active(&[]).expect("init store");
        std::fs::write(
            dir.path().join(".task-golem/archive.jsonl"),
            "{\"schema_version\":1}\n",
        )
        .expect("init archive");

        let (handle, task_handle) =
            spawn_coordinator(store, dir.path().to_path_buf(), "WRK".to_string());

        // Drop the handle to close the channel, which causes the coordinator to exit
        drop(handle);

        // The JoinHandle should resolve to Ok(())
        let result = task_handle.await;
        assert!(
            result.is_ok(),
            "JoinHandle should resolve to Ok(()), got: {:?}",
            result
        );
    }
}
