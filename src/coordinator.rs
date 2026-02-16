use std::path::{Path, PathBuf};

use tokio::sync::{mpsc, oneshot};

use crate::git::StatusEntry;
use crate::types::{BacklogFile, BacklogItem, FollowUp, ItemStatus, ItemUpdate, PhaseResult};
use crate::{log_error, log_warn};

// --- Command enum ---

pub enum CoordinatorCommand {
    GetSnapshot {
        reply: oneshot::Sender<BacklogFile>,
    },
    UpdateItem {
        id: String,
        update: ItemUpdate,
        reply: oneshot::Sender<Result<(), String>>,
    },
    CompletePhase {
        item_id: String,
        result: PhaseResult,
        is_destructive: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    BatchCommit {
        reply: oneshot::Sender<Result<(), String>>,
    },
    GetHeadSha {
        reply: oneshot::Sender<Result<String, String>>,
    },
    IsAncestor {
        sha: String,
        reply: oneshot::Sender<Result<bool, String>>,
    },
    RecordPhaseStart {
        item_id: String,
        commit_sha: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    WriteWorklog {
        item: Box<BacklogItem>,
        phase: String,
        outcome: String,
        summary: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    ArchiveItem {
        item_id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    IngestFollowUps {
        follow_ups: Vec<FollowUp>,
        origin: String,
        reply: oneshot::Sender<Result<Vec<String>, String>>,
    },
    UnblockItem {
        item_id: String,
        context: Option<String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    IngestInbox {
        reply: oneshot::Sender<Result<Vec<String>, String>>,
    },
    MergeItem {
        source_id: String,
        target_id: String,
        reply: oneshot::Sender<Result<(), String>>,
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
    ) -> Result<T, String> {
        self.sender
            .send(command)
            .await
            .map_err(|_| "coordinator shut down".to_string())?;
        rx.await
            .map_err(|_| "coordinator dropped reply".to_string())
    }

    pub async fn get_snapshot(&self) -> Result<BacklogFile, String> {
        let (reply, rx) = oneshot::channel();
        self.send_command(CoordinatorCommand::GetSnapshot { reply }, rx)
            .await
    }

    pub async fn update_item(&self, id: &str, update: ItemUpdate) -> Result<(), String> {
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
    ) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.send_command(
            CoordinatorCommand::CompletePhase {
                item_id: item_id.to_string(),
                result,
                is_destructive,
                reply,
            },
            rx,
        )
        .await?
    }

    pub async fn batch_commit(&self) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.send_command(CoordinatorCommand::BatchCommit { reply }, rx)
            .await?
    }

    pub async fn get_head_sha(&self) -> Result<String, String> {
        let (reply, rx) = oneshot::channel();
        self.send_command(CoordinatorCommand::GetHeadSha { reply }, rx)
            .await?
    }

    pub async fn is_ancestor(&self, sha: &str) -> Result<bool, String> {
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

    pub async fn record_phase_start(&self, item_id: &str, commit_sha: &str) -> Result<(), String> {
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
        item: BacklogItem,
        phase: &str,
        outcome: &str,
        summary: &str,
    ) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        self.send_command(
            CoordinatorCommand::WriteWorklog {
                item: Box::new(item),
                phase: phase.to_string(),
                outcome: outcome.to_string(),
                summary: summary.to_string(),
                reply,
            },
            rx,
        )
        .await?
    }

    pub async fn archive_item(&self, item_id: &str) -> Result<(), String> {
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
    ) -> Result<Vec<String>, String> {
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

    pub async fn unblock_item(&self, item_id: &str, context: Option<String>) -> Result<(), String> {
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

    pub async fn ingest_inbox(&self) -> Result<Vec<String>, String> {
        let (reply, rx) = oneshot::channel();
        self.send_command(CoordinatorCommand::IngestInbox { reply }, rx)
            .await?
    }

    pub async fn merge_item(&self, source_id: &str, target_id: &str) -> Result<(), String> {
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

fn restore_from_blocked(item: &mut BacklogItem) -> Result<(), String> {
    let restore_to = item.blocked_from_status.clone().unwrap_or(ItemStatus::New);
    crate::backlog::transition_status(item, restore_to)
}

// --- Actor implementation ---

const CHANNEL_CAPACITY: usize = 32;

struct CoordinatorState {
    backlog: BacklogFile,
    backlog_path: PathBuf,
    inbox_path: PathBuf,
    project_root: PathBuf,
    prefix: String,
    /// Tracks non-destructive phase completions pending batch commit.
    /// Each entry: (item_id, phase, commit_summary).
    pending_batch_phases: Vec<(String, String, Option<String>)>,
}

impl CoordinatorState {
    fn find_item_mut(&mut self, id: &str) -> Result<&mut BacklogItem, String> {
        self.backlog
            .items
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| format!("Item {} not found in backlog", id))
    }

    fn save_backlog(&self) -> Result<(), String> {
        crate::backlog::save(&self.backlog_path, &self.backlog)
    }

    fn worklog_dir(&self) -> PathBuf {
        self.project_root.join("_worklog")
    }
}

fn handle_get_snapshot(state: &CoordinatorState) -> BacklogFile {
    state.backlog.clone()
}

fn handle_update_item(
    state: &mut CoordinatorState,
    id: &str,
    update: ItemUpdate,
) -> Result<(), String> {
    let item = state.find_item_mut(id)?;

    match update {
        ItemUpdate::TransitionStatus(new_status) => {
            crate::backlog::transition_status(item, new_status)?;
        }
        ItemUpdate::SetPhase(phase) => {
            item.phase = Some(phase);
            item.updated = chrono::Utc::now().to_rfc3339();
        }
        ItemUpdate::SetPhasePool(pool) => {
            item.phase_pool = Some(pool);
            item.updated = chrono::Utc::now().to_rfc3339();
        }
        ItemUpdate::ClearPhase => {
            item.phase = None;
            item.phase_pool = None;
            item.updated = chrono::Utc::now().to_rfc3339();
        }
        ItemUpdate::SetBlocked(reason) => {
            crate::backlog::transition_status(item, ItemStatus::Blocked)?;
            item.blocked_reason = Some(reason);
        }
        ItemUpdate::Unblock => {
            if item.status != ItemStatus::Blocked {
                return Err(format!("Item {} is not blocked", id));
            }
            restore_from_blocked(item)?;
        }
        ItemUpdate::UpdateAssessments(assessments) => {
            crate::backlog::update_assessments(item, &assessments);
        }
        ItemUpdate::SetPipelineType(pipeline_type) => {
            item.pipeline_type = Some(pipeline_type);
            item.updated = chrono::Utc::now().to_rfc3339();
        }
        ItemUpdate::SetLastPhaseCommit(sha) => {
            item.last_phase_commit = Some(sha);
            item.updated = chrono::Utc::now().to_rfc3339();
        }
        ItemUpdate::SetDescription(description) => {
            item.description = Some(description);
            item.updated = chrono::Utc::now().to_rfc3339();
        }
    }

    state.save_backlog()
}

fn handle_record_phase_start(
    state: &mut CoordinatorState,
    item_id: &str,
    commit_sha: &str,
) -> Result<(), String> {
    let item = state.find_item_mut(item_id)?;
    item.last_phase_commit = Some(commit_sha.to_string());
    item.updated = chrono::Utc::now().to_rfc3339();
    state.save_backlog()
}

fn handle_write_worklog(
    state: &CoordinatorState,
    item: &BacklogItem,
    phase: &str,
    outcome: &str,
    summary: &str,
) -> Result<(), String> {
    crate::worklog::write_entry(&state.worklog_dir(), item, phase, outcome, summary)
}

fn handle_archive_single_item(state: &mut CoordinatorState, item_id: &str) -> Result<(), String> {
    let worklog_dir = state.worklog_dir();
    let worklog_month = chrono::Utc::now().format("%Y-%m").to_string();
    let worklog_path = worklog_dir.join(format!("{}.md", worklog_month));

    crate::backlog::archive_item(
        &mut state.backlog,
        item_id,
        &state.backlog_path,
        &worklog_path,
    )
}

fn handle_ingest_follow_ups(
    state: &mut CoordinatorState,
    follow_ups: &[FollowUp],
    origin: &str,
) -> Result<Vec<String>, String> {
    let new_items =
        crate::backlog::ingest_follow_ups(&mut state.backlog, follow_ups, origin, &state.prefix);
    let new_ids: Vec<String> = new_items.iter().map(|i| i.id.clone()).collect();
    state.save_backlog()?;
    Ok(new_ids)
}

fn handle_unblock_item(
    state: &mut CoordinatorState,
    item_id: &str,
    context: Option<String>,
) -> Result<(), String> {
    let item = state.find_item_mut(item_id)?;

    if item.status != ItemStatus::Blocked {
        return Err(format!(
            "Item {} is not blocked (status: {:?})",
            item_id, item.status
        ));
    }

    restore_from_blocked(item)?;

    if let Some(ctx) = context {
        item.unblock_context = Some(ctx);
    }

    // Reset last_phase_commit for staleness-blocked items
    item.last_phase_commit = None;

    state.save_backlog()
}

fn handle_merge_item(
    state: &mut CoordinatorState,
    source_id: &str,
    target_id: &str,
) -> Result<(), String> {
    crate::backlog::merge_item(&mut state.backlog, source_id, target_id)?;
    state.save_backlog()
}

fn handle_ingest_inbox(state: &mut CoordinatorState) -> Result<Vec<String>, String> {
    let items = match crate::backlog::load_inbox(&state.inbox_path) {
        Err(msg) => {
            log_warn!(
                "Failed to parse BACKLOG_INBOX.yaml: {}. File left in place for manual correction.",
                msg
            );
            return Ok(vec![]);
        }
        Ok(None) => return Ok(vec![]),
        Ok(Some(items)) => items,
    };

    if items.is_empty() {
        let _ = crate::backlog::clear_inbox(&state.inbox_path);
        return Ok(vec![]);
    }

    // Record pre-ingestion state for rollback
    let pre_items_len = state.backlog.items.len();
    let pre_next_item_id = state.backlog.next_item_id;

    let created = crate::backlog::ingest_inbox_items(&mut state.backlog, &items, &state.prefix);
    let new_ids: Vec<String> = created.iter().map(|i| i.id.clone()).collect();

    // Save backlog — rollback on failure
    if let Err(e) = state.save_backlog() {
        log_error!("Failed to save backlog after inbox ingestion: {}", e);
        state.backlog.items.truncate(pre_items_len);
        state.backlog.next_item_id = pre_next_item_id;
        return Err(e);
    }

    // Clear inbox — warn on failure but still return success
    if let Err(e) = crate::backlog::clear_inbox(&state.inbox_path) {
        log_warn!(
            "Failed to delete inbox file after ingestion: {}. Items already saved.",
            e
        );
    }

    Ok(new_ids)
}

// --- Actor loop ---

async fn run_coordinator(
    mut rx: mpsc::Receiver<CoordinatorCommand>,
    backlog: BacklogFile,
    backlog_path: PathBuf,
    inbox_path: PathBuf,
    project_root: PathBuf,
    prefix: String,
) {
    let mut state = CoordinatorState {
        backlog,
        backlog_path,
        inbox_path,
        project_root,
        prefix,
        pending_batch_phases: Vec::new(),
    };

    while let Some(cmd) = rx.recv().await {
        match cmd {
            CoordinatorCommand::GetSnapshot { reply } => {
                let snapshot = handle_get_snapshot(&state);
                let _ = reply.send(snapshot);
            }
            CoordinatorCommand::UpdateItem { id, update, reply } => {
                let result = handle_update_item(&mut state, &id, update);
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

                let result = tokio::task::spawn_blocking(move || {
                    let status = crate::git::get_status(Some(&project_root))?;
                    let dirty_paths: Vec<PathBuf> = status
                        .iter()
                        .map(|entry| project_root.join(&entry.path))
                        .collect();

                    if !dirty_paths.is_empty() {
                        let path_refs: Vec<&Path> =
                            dirty_paths.iter().map(|p| p.as_path()).collect();
                        crate::git::stage_paths(&path_refs, Some(&project_root))?;
                    }

                    if is_destructive {
                        let message = build_phase_commit_message(
                            &item_id,
                            &phase_result.phase,
                            phase_result.commit_summary.as_deref(),
                        );
                        let post_status = crate::git::get_status(Some(&project_root))?;
                        if has_staged_changes(&post_status) {
                            crate::git::commit(&message, Some(&project_root))?;
                        }
                    }

                    Ok(())
                })
                .await
                .unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {}", e)));

                if !is_destructive && result.is_ok() {
                    state.pending_batch_phases.push((
                        item_id_for_push,
                        phase_for_push,
                        commit_summary_for_push,
                    ));
                }

                let _ = reply.send(result);
            }
            CoordinatorCommand::BatchCommit { reply } => {
                if state.pending_batch_phases.is_empty() {
                    let _ = reply.send(Ok(()));
                } else {
                    let project_root = state.project_root.clone();
                    let pending_batch_phases = state.pending_batch_phases.clone();

                    let result = tokio::task::spawn_blocking(move || {
                        let status = crate::git::get_status(Some(&project_root))?;
                        if has_staged_changes(&status) {
                            let message = build_batch_commit_message(&pending_batch_phases);
                            crate::git::commit(&message, Some(&project_root))?;
                        }
                        Ok(())
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {}", e)));

                    if result.is_ok() {
                        state.pending_batch_phases.clear();
                    }

                    let _ = reply.send(result);
                }
            }
            CoordinatorCommand::GetHeadSha { reply } => {
                let project_root = state.project_root.clone();
                let result =
                    tokio::task::spawn_blocking(move || crate::git::get_head_sha(&project_root))
                        .await
                        .unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {}", e)));
                let _ = reply.send(result);
            }
            CoordinatorCommand::IsAncestor { sha, reply } => {
                let project_root = state.project_root.clone();
                let result = tokio::task::spawn_blocking(move || {
                    crate::git::is_ancestor(&sha, &project_root)
                })
                .await
                .unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {}", e)));
                let _ = reply.send(result);
            }
            CoordinatorCommand::RecordPhaseStart {
                item_id,
                commit_sha,
                reply,
            } => {
                let result = handle_record_phase_start(&mut state, &item_id, &commit_sha);
                let _ = reply.send(result);
            }
            CoordinatorCommand::WriteWorklog {
                item,
                phase,
                outcome,
                summary,
                reply,
            } => {
                let result = handle_write_worklog(&state, &item, &phase, &outcome, &summary);
                let _ = reply.send(result);
            }
            CoordinatorCommand::ArchiveItem { item_id, reply } => {
                let result = handle_archive_single_item(&mut state, &item_id);
                let _ = reply.send(result);
            }
            CoordinatorCommand::IngestFollowUps {
                follow_ups,
                origin,
                reply,
            } => {
                let result = handle_ingest_follow_ups(&mut state, &follow_ups, &origin);
                let _ = reply.send(result);
            }
            CoordinatorCommand::UnblockItem {
                item_id,
                context,
                reply,
            } => {
                let result = handle_unblock_item(&mut state, &item_id, context);
                let _ = reply.send(result);
            }
            CoordinatorCommand::IngestInbox { reply } => {
                let result = handle_ingest_inbox(&mut state);
                let _ = reply.send(result);
            }
            CoordinatorCommand::MergeItem {
                source_id,
                target_id,
                reply,
            } => {
                let result = handle_merge_item(&mut state, &source_id, &target_id);
                let _ = reply.send(result);
            }
        }
    }

    // Shutdown: save final backlog state when all senders drop
    let _ = state.save_backlog();
}

// --- Spawn ---

pub fn spawn_coordinator(
    backlog: BacklogFile,
    backlog_path: PathBuf,
    inbox_path: PathBuf,
    project_root: PathBuf,
    prefix: String,
) -> (CoordinatorHandle, tokio::task::JoinHandle<()>) {
    let (tx, rx) = mpsc::channel(CHANNEL_CAPACITY);

    let task_handle = tokio::spawn(run_coordinator(
        rx,
        backlog,
        backlog_path,
        inbox_path,
        project_root,
        prefix,
    ));

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
        let backlog = BacklogFile {
            schema_version: 3,
            items: Vec::new(),
            next_item_id: 0,
        };
        let dir = tempfile::tempdir().expect("create tempdir");
        let backlog_path = dir.path().join("BACKLOG.yaml");
        let inbox_path = dir.path().join("BACKLOG_INBOX.yaml");

        let (handle, task_handle) = spawn_coordinator(
            backlog,
            backlog_path,
            inbox_path,
            dir.path().to_path_buf(),
            "WRK".to_string(),
        );

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
