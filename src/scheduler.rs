use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::agent::AgentRunner;
use crate::config::{ExecutionConfig, PhaseGolemConfig, PipelineConfig};
use crate::coordinator::CoordinatorHandle;
use crate::executor;
use crate::filter;
use crate::pg_item;
use crate::prompt;
use crate::types::{
    BacklogFile, BacklogItem, DimensionLevel, ItemStatus, ItemUpdate, PhaseExecutionResult,
    PhasePool, PhaseResult, ResultCode, SchedulerAction, SizeLevel,
};
use crate::{log_debug, log_info, log_warn};

/// Number of consecutive retry exhaustions before circuit breaker trips.
const CIRCUIT_BREAKER_THRESHOLD: u32 = 2;

// --- Public types ---

/// Result of a scheduler run, returned to the caller for summary display.
#[derive(Debug)]
pub struct RunSummary {
    pub phases_executed: u32,
    pub items_completed: Vec<String>,
    pub items_blocked: Vec<String>,
    pub follow_ups_created: u32,
    pub items_merged: u32,
    pub halt_reason: HaltReason,
}

#[derive(Debug, PartialEq)]
pub enum HaltReason {
    AllDoneOrBlocked,
    CapReached,
    CircuitBreakerTripped,
    ShutdownRequested,
    TargetCompleted,
    TargetBlocked,
    FilterExhausted,
    NoMatchingItems,
}

/// Parameters for running the scheduler.
pub struct RunParams {
    pub targets: Vec<String>,
    pub filter: Vec<crate::filter::FilterCriterion>,
    pub cap: u32,
    pub root: PathBuf,
    /// Base directory for resolving config-relative paths (workflow files).
    /// When `--config` is used, this is the config file's parent directory.
    /// Otherwise, equals `root`.
    pub config_base: PathBuf,
    pub auto_advance: bool,
}

// --- Running task tracking ---

/// Information about a currently running task.
#[allow(dead_code)]
struct RunningTaskInfo {
    phase: String,
    phase_pool: PhasePool,
    is_destructive: bool,
}

/// Tracks currently running executor tasks.
#[derive(Default)]
pub struct RunningTasks {
    active: HashMap<String, RunningTaskInfo>,
}

impl RunningTasks {
    pub fn new() -> Self {
        Self::default()
    }

    fn has_destructive(&self) -> bool {
        self.active.values().any(|t| t.is_destructive)
    }

    fn non_destructive_count(&self) -> usize {
        self.active.values().filter(|t| !t.is_destructive).count()
    }

    fn is_item_running(&self, item_id: &str) -> bool {
        self.active.contains_key(item_id)
    }

    fn insert(&mut self, item_id: String, info: RunningTaskInfo) {
        self.active.insert(item_id, info);
    }

    fn remove(&mut self, item_id: &str) {
        self.active.remove(item_id);
    }

    fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// Insert a non-destructive running task (test helper).
    pub fn insert_non_destructive(&mut self, item_id: &str, phase: &str) {
        self.insert(
            item_id.to_string(),
            RunningTaskInfo {
                phase: phase.to_string(),
                phase_pool: PhasePool::Main,
                is_destructive: false,
            },
        );
    }

    /// Insert a destructive running task (test helper).
    pub fn insert_destructive(&mut self, item_id: &str, phase: &str) {
        self.insert(
            item_id.to_string(),
            RunningTaskInfo {
                phase: phase.to_string(),
                phase_pool: PhasePool::Main,
                is_destructive: true,
            },
        );
    }
}

// --- select_actions: pure function ---

/// Select the next actions to execute based on current state.
///
/// This is a pure function — no I/O, no async, trivially testable.
///
/// Priority rules (from design):
/// 1. If a destructive task is running → return empty (exclusive lock)
/// 2. Promote Ready → InProgress when in_progress_count < max_wip
/// 3. InProgress phases first (advance-furthest-first)
/// 4. Scoping phases next
/// 5. Triage last
///
/// Constraints:
/// - Fill up to max_concurrent slots
/// - If next phase is destructive, it must be the ONLY action
/// - Items already running are excluded
pub fn select_actions(
    snapshot: &BacklogFile,
    running: &RunningTasks,
    config: &ExecutionConfig,
    pipelines: &HashMap<String, PipelineConfig>,
) -> Vec<SchedulerAction> {
    // (1) If a destructive task is running, return empty
    if running.has_destructive() {
        return Vec::new();
    }

    let available_slots = config
        .max_concurrent
        .saturating_sub(running.non_destructive_count() as u32) as usize;

    if available_slots == 0 {
        return Vec::new();
    }

    let mut actions: Vec<SchedulerAction> = Vec::new();

    // Count current InProgress items (not Blocked, not Done)
    let in_progress_count = snapshot
        .items
        .iter()
        .filter(|i| i.status == ItemStatus::InProgress)
        .count() as u32;

    // (2) Promote Ready → InProgress when under max_wip
    // Promotions don't consume executor slots — they're instant state transitions
    let promotions_needed = config.max_wip.saturating_sub(in_progress_count) as usize;
    let ready_items = sorted_ready_items(&snapshot.items);
    let mut promoted = 0usize;
    for item in &ready_items {
        if promoted >= promotions_needed {
            break;
        }
        if skip_for_unmet_deps(item, &snapshot.items) {
            continue;
        }
        if !running.is_item_running(&item.id) {
            actions.push(SchedulerAction::Promote(item.id.clone()));
            promoted += 1;
        }
    }

    // (3 & 4) Build phase actions: InProgress first, then Scoping
    let mut phase_actions = Vec::new();

    // InProgress items with phases to run
    let in_progress_runnable = sorted_in_progress_items(&snapshot.items, pipelines);
    for item in &in_progress_runnable {
        if running.is_item_running(&item.id) {
            continue;
        }
        if skip_for_unmet_deps(item, &snapshot.items) {
            continue;
        }
        if let Some(action) = build_run_phase_action(item, pipelines) {
            phase_actions.push(action);
        }
    }

    // Scoping items with phases to run
    let scoping_runnable = sorted_scoping_items(&snapshot.items, pipelines);
    for item in &scoping_runnable {
        if running.is_item_running(&item.id) {
            continue;
        }
        if skip_for_unmet_deps(item, &snapshot.items) {
            continue;
        }
        if let Some(action) = build_run_phase_action(item, pipelines) {
            phase_actions.push(action);
        }
    }

    // (5) Triage New items (lowest priority)
    let new_items = sorted_new_items(&snapshot.items);
    for item in &new_items {
        if running.is_item_running(&item.id) {
            continue;
        }
        if skip_for_unmet_deps(item, &snapshot.items) {
            continue;
        }
        phase_actions.push(SchedulerAction::Triage(item.id.clone()));
    }

    // Fill slots respecting destructive exclusion
    let mut slots_remaining = available_slots;
    for action in phase_actions {
        if slots_remaining == 0 {
            break;
        }

        match &action {
            SchedulerAction::RunPhase { is_destructive, .. } if *is_destructive => {
                // Destructive must be the ONLY running task
                if running.is_empty()
                    && actions
                        .iter()
                        .all(|a| matches!(a, SchedulerAction::Promote(_)))
                {
                    // Only promotions so far (no executor tasks) and nothing running — safe
                    actions.push(action);
                    break; // No more actions after destructive
                }
                // Can't run destructive yet — stop filling slots so running tasks
                // drain naturally and the destructive action isn't starved.
                break;
            }
            _ => {
                // Non-destructive: check that no destructive is already queued
                let has_queued_destructive = actions.iter().any(|a| {
                    matches!(
                        a,
                        SchedulerAction::RunPhase {
                            is_destructive: true,
                            ..
                        }
                    )
                });
                if has_queued_destructive {
                    break; // Can't add anything after a destructive action
                }
                actions.push(action);
                slots_remaining -= 1;
            }
        }
    }

    actions
}

// --- Sorting helpers ---

/// Sort Ready items by impact (desc), then created date (asc, FIFO).
fn sorted_ready_items(items: &[BacklogItem]) -> Vec<&BacklogItem> {
    let mut ready: Vec<&BacklogItem> = items
        .iter()
        .filter(|i| i.status == ItemStatus::Ready)
        .collect();
    ready.sort_by(|a, b| {
        let impact_a = impact_sort_value(&a.impact);
        let impact_b = impact_sort_value(&b.impact);
        impact_b
            .cmp(&impact_a)
            .then_with(|| a.created.cmp(&b.created))
    });
    ready
}

/// Sort InProgress items by advance-furthest-first: higher phase index first,
/// then created date asc (FIFO).
fn sorted_in_progress_items<'a>(
    items: &'a [BacklogItem],
    pipelines: &HashMap<String, PipelineConfig>,
) -> Vec<&'a BacklogItem> {
    let mut in_progress: Vec<&BacklogItem> = items
        .iter()
        .filter(|i| i.status == ItemStatus::InProgress && i.phase.is_some())
        .collect();
    in_progress.sort_by(|a, b| {
        let idx_a = phase_index(a, pipelines);
        let idx_b = phase_index(b, pipelines);
        idx_b
            .cmp(&idx_a) // Higher index first (furthest-first)
            .then_with(|| a.created.cmp(&b.created))
    });
    in_progress
}

/// Sort Scoping items by phase index (desc), then created date (asc).
fn sorted_scoping_items<'a>(
    items: &'a [BacklogItem],
    pipelines: &HashMap<String, PipelineConfig>,
) -> Vec<&'a BacklogItem> {
    let mut scoping: Vec<&BacklogItem> = items
        .iter()
        .filter(|i| i.status == ItemStatus::Scoping && i.phase.is_some())
        .collect();
    scoping.sort_by(|a, b| {
        let idx_a = phase_index(a, pipelines);
        let idx_b = phase_index(b, pipelines);
        idx_b.cmp(&idx_a).then_with(|| a.created.cmp(&b.created))
    });
    scoping
}

/// Sort New items by created date (asc, FIFO).
fn sorted_new_items(items: &[BacklogItem]) -> Vec<&BacklogItem> {
    let mut new_items: Vec<&BacklogItem> = items
        .iter()
        .filter(|i| i.status == ItemStatus::New)
        .collect();
    new_items.sort_by(|a, b| a.created.cmp(&b.created));
    new_items
}

/// Build a comma-separated summary of unmet dependencies for an item.
///
/// Each unmet dependency is formatted as `"dep_id (status)"`.
/// Returns `None` if all dependencies are met (or item has no dependencies).
/// Returns `Some(summary)` listing each unmet dependency.
///
/// A dependency is met if:
/// - The dep ID is not found in `all_items` (absent = archived = met)
/// - The dep ID is found with status `Done`
pub fn unmet_dep_summary(item: &BacklogItem, all_items: &[BacklogItem]) -> Option<String> {
    if item.dependencies.is_empty() {
        return None;
    }
    let unmet: Vec<String> = item
        .dependencies
        .iter()
        .filter_map(|dep_id| {
            match all_items.iter().find(|i| i.id == *dep_id) {
                Some(dep_item) if dep_item.status != ItemStatus::Done => {
                    Some(format!("{} ({:?})", dep_id, dep_item.status))
                }
                _ => None, // Done or absent = met
            }
        })
        .collect();
    if unmet.is_empty() {
        None
    } else {
        Some(unmet.join(", "))
    }
}

/// Check and log if item has unmet dependencies. Returns true if unmet deps exist.
fn skip_for_unmet_deps(item: &BacklogItem, all_items: &[BacklogItem]) -> bool {
    if let Some(summary) = unmet_dep_summary(item, all_items) {
        log_debug!("Item {} skipped: unmet dependencies: {}", item.id, summary);
        return true;
    }
    false
}

/// Compute phase index for advance-furthest-first sorting.
///
/// InProgress items always sort ahead of Scoping items (higher base offset).
/// Within each pool, higher phase index = further along.
fn phase_index(item: &BacklogItem, pipelines: &HashMap<String, PipelineConfig>) -> usize {
    let pipeline_type = item.pipeline_type.as_deref().unwrap_or("feature");
    let pipeline = match pipelines.get(pipeline_type) {
        Some(p) => p,
        None => return 0,
    };

    let phase_name = match &item.phase {
        Some(name) => name.as_str(),
        None => return 0,
    };

    let pool = item.phase_pool.as_ref();
    match pool {
        Some(PhasePool::Pre) => pipeline
            .pre_phases
            .iter()
            .position(|p| p.name == phase_name)
            .unwrap_or(0),
        Some(PhasePool::Main) | None => {
            let pre_count = pipeline.pre_phases.len();
            let main_idx = pipeline
                .phases
                .iter()
                .position(|p| p.name == phase_name)
                .unwrap_or(0);
            pre_count + main_idx
        }
    }
}

fn impact_sort_value(impact: &Option<DimensionLevel>) -> u8 {
    match impact {
        Some(DimensionLevel::High) => 3,
        Some(DimensionLevel::Medium) => 2,
        Some(DimensionLevel::Low) => 1,
        None => 0,
    }
}

/// Build a RunPhase action for an item based on its current phase.
fn build_run_phase_action(
    item: &BacklogItem,
    pipelines: &HashMap<String, PipelineConfig>,
) -> Option<SchedulerAction> {
    let pipeline_type = item.pipeline_type.as_deref().unwrap_or("feature");
    let pipeline = pipelines.get(pipeline_type)?;
    let phase_name = item.phase.as_deref()?;

    let phase_config = pipeline
        .pre_phases
        .iter()
        .chain(pipeline.phases.iter())
        .find(|p| p.name == phase_name)?;

    let phase_pool = item.phase_pool.clone().unwrap_or(PhasePool::Main);

    Some(SchedulerAction::RunPhase {
        item_id: item.id.clone(),
        phase: phase_name.to_string(),
        phase_pool,
        is_destructive: phase_config.is_destructive,
    })
}

// --- Multi-target advancement ---

/// Advance past Done/Blocked/archived targets to find the next active target.
///
/// Returns the index of the next active target, or an index >= `targets.len()`
/// if all targets are exhausted (Done, Blocked, or archived).
///
/// Note: This skips targets with `ItemStatus::Blocked` in the snapshot (pre-existing
/// blocked state) but does NOT check `items_blocked` (run-time blocked tracking).
/// The caller handles run-time blocked detection separately with `TargetBlocked` halt.
pub fn advance_to_next_active_target(
    targets: &[String],
    current_index: usize,
    items_completed: &[String],
    snapshot: &BacklogFile,
) -> usize {
    let mut index = current_index;
    while index < targets.len() {
        let target = &targets[index];
        let target_item = snapshot.items.iter().find(|i| i.id == *target);
        match target_item {
            None => {
                log_warn!(
                    "[target] {} not found (archived?). Skipping ({}/{}).",
                    target,
                    index + 1,
                    targets.len()
                );
                index += 1;
            }
            Some(item) if items_completed.contains(&item.id) || item.status == ItemStatus::Done => {
                log_info!(
                    "[target] {} already done. Skipping ({}/{}).",
                    target,
                    index + 1,
                    targets.len()
                );
                index += 1;
            }
            Some(item) if item.status == ItemStatus::Blocked => {
                log_info!(
                    "[target] {} already blocked. Skipping ({}/{}).",
                    target,
                    index + 1,
                    targets.len()
                );
                index += 1;
            }
            _ => break,
        }
    }
    index
}

// --- Main scheduling loop ---

/// Run the scheduler loop.
///
/// This is the main entry point for the phase-golem scheduler.
///
/// The loop:
/// 1. Get snapshot from coordinator
/// 2. Select actions via `select_actions()` (pure function)
/// 3. Execute promotions immediately via coordinator
/// 4. Spawn executor tasks into JoinSet for phase actions
/// 5. Await one completion (or all if nothing to spawn)
/// 6. Process results: apply transitions, commit, handle follow-ups
/// 7. Batch commit non-destructive outputs
/// 8. Loop until all done/blocked, cap reached, or shutdown
pub async fn run_scheduler(
    coordinator: CoordinatorHandle,
    runner: Arc<impl AgentRunner + 'static>,
    config: PhaseGolemConfig,
    params: RunParams,
    cancel: CancellationToken,
) -> Result<RunSummary, String> {
    let mut state = SchedulerState {
        phases_executed: 0,
        cap: params.cap,
        consecutive_exhaustions: 0,
        items_completed: Vec::new(),
        items_blocked: Vec::new(),
        follow_ups_created: 0,
        items_merged: 0,
        current_target_index: 0,
    };

    let mut running = RunningTasks::new();
    let mut join_set: JoinSet<(String, PhaseExecutionResult)> = JoinSet::new();
    // Track previous summaries per item for context passing
    let mut previous_summaries: HashMap<String, String> = HashMap::new();

    log_info!(
        "Scheduler started (max_wip={}, max_concurrent={}).",
        config.execution.max_wip,
        config.execution.max_concurrent
    );

    loop {
        if cancel.is_cancelled() {
            // Drain remaining tasks and commit before exiting
            drain_join_set(
                &mut join_set,
                &mut running,
                &mut state,
                &coordinator,
                &config,
                &mut previous_summaries,
            )
            .await;
            let _ = coordinator.batch_commit().await;
            return Ok(build_summary(state, HaltReason::ShutdownRequested));
        }

        if state.is_circuit_breaker_tripped() {
            log_warn!(
                "Circuit breaker tripped: {} consecutive items exhausted retries",
                CIRCUIT_BREAKER_THRESHOLD
            );
            drain_join_set(
                &mut join_set,
                &mut running,
                &mut state,
                &coordinator,
                &config,
                &mut previous_summaries,
            )
            .await;
            let _ = coordinator.batch_commit().await;
            return Ok(build_summary(state, HaltReason::CircuitBreakerTripped));
        }

        // Get current snapshot (PgItem vec -> BacklogFile for legacy consumers)
        let pg_snapshot = coordinator.get_snapshot().await?;
        let snapshot = pg_item::to_backlog_file(&pg_snapshot);

        // Check target completion/block (multi-target with cursor advancement)
        if !params.targets.is_empty() {
            // Check if current target was blocked during this run (before advancement)
            if state.current_target_index < params.targets.len() {
                let target_id = &params.targets[state.current_target_index];
                if state.items_blocked.contains(target_id) {
                    if params.auto_advance {
                        log_info!(
                            "[target] {} blocked ({}/{}). Auto-advancing.",
                            target_id,
                            state.current_target_index + 1,
                            params.targets.len()
                        );
                        drain_join_set(
                            &mut join_set,
                            &mut running,
                            &mut state,
                            &coordinator,
                            &config,
                            &mut previous_summaries,
                        )
                        .await;
                        let _ = coordinator.batch_commit().await;
                        state.consecutive_exhaustions = 0;
                        state.current_target_index += 1;
                        continue;
                    } else {
                        log_info!(
                            "[target] {} blocked ({}/{}). Halting.",
                            target_id,
                            state.current_target_index + 1,
                            params.targets.len()
                        );
                        drain_join_set(
                            &mut join_set,
                            &mut running,
                            &mut state,
                            &coordinator,
                            &config,
                            &mut previous_summaries,
                        )
                        .await;
                        let _ = coordinator.batch_commit().await;
                        return Ok(build_summary(state, HaltReason::TargetBlocked));
                    }
                }
            }
            // Advance past Done/archived/pre-Blocked targets
            state.current_target_index = advance_to_next_active_target(
                &params.targets,
                state.current_target_index,
                &state.items_completed,
                &snapshot,
            );
            if state.current_target_index >= params.targets.len() {
                drain_join_set(
                    &mut join_set,
                    &mut running,
                    &mut state,
                    &coordinator,
                    &config,
                    &mut previous_summaries,
                )
                .await;
                let _ = coordinator.batch_commit().await;
                return Ok(build_summary(state, HaltReason::TargetCompleted));
            }
        }

        // Filter application — restrict snapshot for filter mode
        let filtered_snapshot = if !params.filter.is_empty() {
            let filtered = filter::apply_filters(&params.filter, &snapshot);
            let criteria_display = filter::format_filter_criteria(&params.filter);
            // Check halt conditions based on filter results
            if filtered.items.is_empty() {
                // Determine if no items match at all, or all matching are Done/Blocked/archived.
                // Check both the current snapshot and items we've already completed/blocked
                // (which may have been archived and removed from the snapshot).
                let any_match_in_snapshot = snapshot
                    .items
                    .iter()
                    .any(|item| params.filter.iter().all(|c| filter::matches_item(c, item)));
                let has_prior_progress =
                    !state.items_completed.is_empty() || !state.items_blocked.is_empty();
                if !any_match_in_snapshot && !has_prior_progress {
                    log_info!(
                        "[filter] No items match filter criteria: {}",
                        criteria_display
                    );
                    drain_join_set(
                        &mut join_set,
                        &mut running,
                        &mut state,
                        &coordinator,
                        &config,
                        &mut previous_summaries,
                    )
                    .await;
                    let _ = coordinator.batch_commit().await;
                    return Ok(build_summary(state, HaltReason::NoMatchingItems));
                } else {
                    log_info!(
                        "[filter] All items matching {} are done or blocked.",
                        criteria_display
                    );
                    drain_join_set(
                        &mut join_set,
                        &mut running,
                        &mut state,
                        &coordinator,
                        &config,
                        &mut previous_summaries,
                    )
                    .await;
                    let _ = coordinator.batch_commit().await;
                    return Ok(build_summary(state, HaltReason::FilterExhausted));
                }
            }
            // Check if all remaining filtered items are Done or Blocked
            let all_done_or_blocked = filtered
                .items
                .iter()
                .all(|i| matches!(i.status, ItemStatus::Done | ItemStatus::Blocked));
            if all_done_or_blocked {
                log_info!(
                    "[filter] All items matching {} are done or blocked.",
                    criteria_display
                );
                drain_join_set(
                    &mut join_set,
                    &mut running,
                    &mut state,
                    &coordinator,
                    &config,
                    &mut previous_summaries,
                )
                .await;
                let _ = coordinator.batch_commit().await;
                return Ok(build_summary(state, HaltReason::FilterExhausted));
            }
            Some(filtered)
        } else {
            None
        };

        // Select actions (three-way dispatch: targets, filter, normal)
        let actions = if !params.targets.is_empty() {
            select_targeted_actions(
                &snapshot,
                &running,
                &config.execution,
                &config.pipelines,
                &params.targets[state.current_target_index],
            )
        } else if let Some(ref filtered) = filtered_snapshot {
            select_actions(filtered, &running, &config.execution, &config.pipelines)
        } else {
            select_actions(&snapshot, &running, &config.execution, &config.pipelines)
        };

        if actions.is_empty() && running.is_empty() {
            // Nothing to do and nothing running
            // Log items blocked by unmet dependencies for diagnostics
            let dep_blocked: Vec<String> = snapshot
                .items
                .iter()
                .filter(|i| i.status != ItemStatus::Done)
                .filter_map(|i| {
                    unmet_dep_summary(i, &snapshot.items)
                        .map(|summary| format!("{} (waiting on: {})", i.id, summary))
                })
                .collect();
            if !dep_blocked.is_empty() {
                log_info!(
                    "Items blocked by unmet dependencies: {}",
                    dep_blocked.join("; ")
                );
            }
            log_info!("No actionable items — all done or blocked.");
            return Ok(build_summary(state, HaltReason::AllDoneOrBlocked));
        }

        if !actions.is_empty() {
            let action_descriptions: Vec<String> = actions
                .iter()
                .map(|a| match a {
                    SchedulerAction::Promote(id) => format!("promote {}", id),
                    SchedulerAction::Triage(id) => format!("triage {}", id),
                    SchedulerAction::RunPhase { item_id, phase, .. } => {
                        format!("{} → {}", item_id, phase)
                    }
                })
                .collect();
            log_info!("\nScheduling: [{}]", action_descriptions.join(", "));
        }

        // Process actions
        for action in actions {
            match action {
                SchedulerAction::Promote(item_id) => {
                    handle_promote(&snapshot, &coordinator, &item_id, &config).await?;
                }
                SchedulerAction::Triage(item_id) => {
                    if state.is_cap_reached() {
                        break;
                    }
                    state.phases_executed += 1;
                    spawn_triage(
                        &mut join_set,
                        &mut running,
                        &coordinator,
                        runner.clone(),
                        &config,
                        &item_id,
                        &params.root,
                    )
                    .await;
                }
                SchedulerAction::RunPhase {
                    item_id,
                    phase,
                    phase_pool,
                    is_destructive,
                } => {
                    if state.is_cap_reached() {
                        break;
                    }
                    state.phases_executed += 1;

                    log_info!(
                        "[{}][{}] Starting phase ({})",
                        item_id,
                        phase.to_uppercase(),
                        if is_destructive {
                            "destructive"
                        } else {
                            "non-destructive"
                        }
                    );
                    log_debug!(
                        "Progress: {}/{} phases used",
                        state.phases_executed,
                        state.cap
                    );

                    running.insert(
                        item_id.clone(),
                        RunningTaskInfo {
                            phase: phase.clone(),
                            phase_pool: phase_pool.clone(),
                            is_destructive,
                        },
                    );

                    let coord = coordinator.clone();
                    let runner_clone = runner.clone();
                    let cfg = config.clone();
                    let root = params.root.clone();
                    let config_base = params.config_base.clone();
                    let prev_summary = previous_summaries.get(&item_id).cloned();
                    let cancel_clone = cancel.clone();

                    join_set.spawn(async move {
                        // Get a fresh snapshot of the item for execution
                        let pg_snap = match coord.get_snapshot().await {
                            Ok(s) => s,
                            Err(e) => {
                                return (
                                    item_id,
                                    PhaseExecutionResult::Failed(format!(
                                        "Failed to get snapshot: {}",
                                        e
                                    )),
                                )
                            }
                        };
                        let item: BacklogItem = match pg_snap.iter().find(|i| i.id() == item_id) {
                            Some(i) => i.clone().into(),
                            None => {
                                return (
                                    item_id,
                                    PhaseExecutionResult::Failed(
                                        "Item not found in snapshot".to_string(),
                                    ),
                                )
                            }
                        };

                        let pipeline_type = item.pipeline_type.as_deref().unwrap_or("feature");
                        let pipeline = match cfg.pipelines.get(pipeline_type) {
                            Some(p) => p,
                            None => {
                                return (
                                    item_id,
                                    PhaseExecutionResult::Failed(format!(
                                        "Pipeline '{}' not found",
                                        pipeline_type
                                    )),
                                )
                            }
                        };

                        let phase_config = match pipeline
                            .pre_phases
                            .iter()
                            .chain(pipeline.phases.iter())
                            .find(|p| p.name == phase)
                        {
                            Some(pc) => pc,
                            None => {
                                return (
                                    item_id,
                                    PhaseExecutionResult::Failed(format!(
                                        "Phase '{}' not found in pipeline",
                                        phase
                                    )),
                                )
                            }
                        };

                        let result = executor::execute_phase(
                            &item,
                            phase_config,
                            &cfg,
                            &coord,
                            runner_clone.as_ref(),
                            &cancel_clone,
                            &root,
                            prev_summary.as_deref(),
                            &config_base,
                        )
                        .await;

                        (item_id, result)
                    });
                }
            }
        }

        // If cap is reached and all in-flight work is done, exit cleanly
        if state.is_cap_reached() && join_set.is_empty() {
            if let Err(e) = coordinator.batch_commit().await {
                log_warn!("Warning: batch commit failed: {}", e);
            }
            return Ok(build_summary(state, HaltReason::CapReached));
        }

        // Wait for at least one task completion (or timeout if nothing is running)
        if !join_set.is_empty() {
            tokio::select! {
                Some(result) = join_set.join_next() => {
                    match result {
                        Ok((item_id, exec_result)) => {
                            running.remove(&item_id);
                            handle_task_completion(
                                &item_id,
                                exec_result,
                                &coordinator,
                                &config,
                                &mut state,
                                &mut previous_summaries,
                            ).await?;
                        }
                        Err(e) => {
                            log_debug!("Task join error: {}", e);
                        }
                    }
                }
                _ = cancel.cancelled() => {
                    drain_join_set(&mut join_set, &mut running, &mut state, &coordinator, &config, &mut previous_summaries).await;
                    let _ = coordinator.batch_commit().await;
                    return Ok(build_summary(state, HaltReason::ShutdownRequested));
                }
            }
        } else if running.is_empty() {
            // No tasks running and nothing to spawn — check if we need to wait
            // for promotions to take effect (next loop iteration will see updated snapshot)
            // Give coordinator a moment to process updates
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Batch commit non-destructive outputs
        if let Err(e) = coordinator.batch_commit().await {
            log_warn!("Warning: batch commit failed: {}", e);
        }
    }
}

// --- Targeted selection ---

/// Like `select_actions` but restricted to a specific target item.
pub fn select_targeted_actions(
    snapshot: &BacklogFile,
    running: &RunningTasks,
    _config: &ExecutionConfig,
    pipelines: &HashMap<String, PipelineConfig>,
    target_id: &str,
) -> Vec<SchedulerAction> {
    // Find the target item
    let target = match snapshot.items.iter().find(|i| i.id == target_id) {
        Some(item) => item,
        None => return Vec::new(),
    };

    // If target has unmet dependencies, skip it
    if skip_for_unmet_deps(target, &snapshot.items) {
        return Vec::new();
    }

    // If target is done or blocked and not running, nothing to do
    if matches!(target.status, ItemStatus::Done | ItemStatus::Blocked)
        && !running.is_item_running(target_id)
    {
        return Vec::new();
    }

    // If destructive is running, wait
    if running.has_destructive() {
        return Vec::new();
    }

    let mut actions = Vec::new();

    match target.status {
        ItemStatus::New => {
            if !running.is_item_running(target_id) {
                actions.push(SchedulerAction::Triage(target_id.to_string()));
            }
        }
        ItemStatus::Ready => {
            actions.push(SchedulerAction::Promote(target_id.to_string()));
        }
        ItemStatus::Scoping | ItemStatus::InProgress => {
            if !running.is_item_running(target_id) {
                if let Some(action) = build_run_phase_action(target, pipelines) {
                    actions.push(action);
                }
            }
        }
        ItemStatus::Blocked | ItemStatus::Done => {
            // Nothing to do
        }
    }

    actions
}

// --- Task completion handling ---

/// Handle the result of a completed executor task.
async fn handle_task_completion(
    item_id: &str,
    exec_result: PhaseExecutionResult,
    coordinator: &CoordinatorHandle,
    config: &PhaseGolemConfig,
    state: &mut SchedulerState,
    previous_summaries: &mut HashMap<String, String>,
) -> Result<(), String> {
    // Snapshot freshness contract:
    // - Handlers that read the backlog before mutating (subphase_complete, failed,
    //   blocked, cancelled) use the pre-fetched snapshot passed by reference.
    // - triage_success uses the pre-fetched snapshot for its initial worklog write,
    //   then re-fetches after mutations (process_merges/apply_triage_result).
    // - phase_success mutates first (assessments, follow-ups), then fetches its
    //   own snapshot at the mutation boundary — it does not use the pre-fetched one.
    let snapshot = pg_item::to_backlog_file(&coordinator.get_snapshot().await?);

    match exec_result {
        PhaseExecutionResult::Success(phase_result) => {
            if phase_result.phase == "triage" {
                handle_triage_success(
                    &snapshot,
                    item_id,
                    &phase_result,
                    coordinator,
                    config,
                    state,
                )
                .await
            } else {
                handle_phase_success(
                    item_id,
                    phase_result,
                    coordinator,
                    config,
                    state,
                    previous_summaries,
                )
                .await
            }
        }
        PhaseExecutionResult::SubphaseComplete(phase_result) => {
            handle_subphase_complete(
                &snapshot,
                item_id,
                phase_result,
                coordinator,
                config,
                state,
                previous_summaries,
            )
            .await
        }
        PhaseExecutionResult::Failed(reason) => {
            handle_phase_failed(&snapshot, item_id, &reason, coordinator, state, previous_summaries)
                .await
        }
        PhaseExecutionResult::Blocked(reason) => {
            handle_phase_blocked(
                &snapshot,
                item_id,
                &reason,
                coordinator,
                state,
                previous_summaries,
            )
            .await
        }
        PhaseExecutionResult::Cancelled => {
            log_info!("[{}] Phase cancelled", item_id);
            // Write worklog entry
            if let Some(item) = snapshot.items.iter().find(|i| i.id == item_id) {
                let phase = item.phase.as_deref().unwrap_or("unknown");
                let _ = coordinator
                    .write_worklog(&item.id, &item.title, phase, "Cancelled", "Shutdown requested")
                    .await;
            }
            Ok(())
        }
    }
}

/// Remove a terminal item's entry from `previous_summaries`.
///
/// Called when an item reaches Done or Blocked — its summary will never be
/// needed again, so we free the memory immediately.
fn cleanup_terminal_summary(item_id: &str, previous_summaries: &mut HashMap<String, String>) {
    previous_summaries.remove(item_id);
}

async fn handle_phase_success(
    item_id: &str,
    phase_result: PhaseResult,
    coordinator: &CoordinatorHandle,
    config: &PhaseGolemConfig,
    state: &mut SchedulerState,
    previous_summaries: &mut HashMap<String, String>,
) -> Result<(), String> {
    let phase = phase_result.phase.clone();
    let summary = phase_result.summary.clone();

    log_info!(
        "[{}][{}] Result: PHASE_COMPLETE — {}",
        item_id,
        phase.to_uppercase(),
        summary
    );

    // Apply assessment updates
    if let Some(ref assessments) = phase_result.updated_assessments {
        coordinator
            .update_item(item_id, ItemUpdate::UpdateAssessments(assessments.clone()))
            .await?;
    }

    // Ingest follow-ups
    let fu_count = ingest_follow_ups(coordinator, &phase_result, config).await;
    state.follow_ups_created += fu_count;
    if fu_count > 0 {
        log_info!("Follow-ups: {} new items added to backlog", fu_count);
    }

    // Get current item state for transition resolution
    let snapshot = pg_item::to_backlog_file(&coordinator.get_snapshot().await?);
    let item = snapshot
        .items
        .iter()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("Item {} not found after phase completion", item_id))?;

    let pipeline_type = item.pipeline_type.as_deref().unwrap_or("feature");
    let pipeline = config
        .pipelines
        .get(pipeline_type)
        .ok_or_else(|| format!("Pipeline '{}' not found", pipeline_type))?;

    // Determine phase config for complete_phase call
    let phase_config = pipeline
        .pre_phases
        .iter()
        .chain(pipeline.phases.iter())
        .find(|p| p.name == phase);
    let is_destructive = phase_config.map(|pc| pc.is_destructive).unwrap_or(false);

    // Write worklog entry
    let _ = coordinator
        .write_worklog(&item.id, &item.title, &phase, "Complete", &summary)
        .await;

    // Complete phase (stage + commit for destructive, stage for non-destructive)
    coordinator
        .complete_phase(item_id, phase_result.clone(), is_destructive)
        .await?;

    // Resolve transitions
    let updates = executor::resolve_transition(item, &phase_result, pipeline, &config.guardrails);
    let mut is_terminal = false;
    for update in updates {
        match &update {
            ItemUpdate::TransitionStatus(ItemStatus::Done) => {
                is_terminal = true;
                coordinator.update_item(item_id, update).await?;
                // Archive the item
                coordinator.archive_item(item_id).await?;
                state.items_completed.push(item_id.to_string());
                state.consecutive_exhaustions = 0;
                log_info!("{} completed and archived", item_id);
            }
            ItemUpdate::SetBlocked(reason) => {
                is_terminal = true;
                log_info!("[{}] Blocked: {}", item_id, reason);
                coordinator.update_item(item_id, update).await?;
                state.items_blocked.push(item_id.to_string());
            }
            _ => {
                coordinator.update_item(item_id, update).await?;
            }
        }
    }

    if is_terminal {
        cleanup_terminal_summary(item_id, previous_summaries);
    } else {
        previous_summaries.insert(item_id.to_string(), summary);
        if previous_summaries.len() > config.execution.max_wip as usize * 20 {
            log_debug!(
                "previous_summaries size ({}) exceeds threshold (max_wip * 20 = {})",
                previous_summaries.len(),
                config.execution.max_wip as usize * 20
            );
        }
    }
    Ok(())
}

async fn handle_subphase_complete(
    snapshot: &BacklogFile,
    item_id: &str,
    phase_result: PhaseResult,
    coordinator: &CoordinatorHandle,
    config: &PhaseGolemConfig,
    state: &mut SchedulerState,
    previous_summaries: &mut HashMap<String, String>,
) -> Result<(), String> {
    let phase = phase_result.phase.clone();
    let summary = phase_result.summary.clone();

    log_info!(
        "[{}][{}] Result: SUBPHASE_COMPLETE — {}",
        item_id,
        phase.to_uppercase(),
        summary
    );

    // Write worklog entry
    if let Some(item) = snapshot.items.iter().find(|i| i.id == item_id) {
        let _ = coordinator
            .write_worklog(&item.id, &item.title, &phase, "Subphase Complete", &summary)
            .await;
    }

    // Apply assessment updates
    if let Some(ref assessments) = phase_result.updated_assessments {
        coordinator
            .update_item(item_id, ItemUpdate::UpdateAssessments(assessments.clone()))
            .await?;
    }

    // Ingest follow-ups
    let fu_count = ingest_follow_ups(coordinator, &phase_result, config).await;
    state.follow_ups_created += fu_count;

    // Complete phase (commit subphase output)
    coordinator
        .complete_phase(item_id, phase_result, true) // commit immediately for subphase
        .await?;

    // Update previous summary — re-queue happens naturally on next loop iteration
    previous_summaries.insert(item_id.to_string(), summary);
    if previous_summaries.len() > config.execution.max_wip as usize * 20 {
        log_debug!(
            "previous_summaries size ({}) exceeds threshold (max_wip * 20 = {})",
            previous_summaries.len(),
            config.execution.max_wip as usize * 20
        );
    }

    Ok(())
}

async fn handle_phase_failed(
    snapshot: &BacklogFile,
    item_id: &str,
    reason: &str,
    coordinator: &CoordinatorHandle,
    state: &mut SchedulerState,
    previous_summaries: &mut HashMap<String, String>,
) -> Result<(), String> {
    log_info!("[{}] Phase failed: {}", item_id, reason);

    // Write worklog entry
    if let Some(item) = snapshot.items.iter().find(|i| i.id == item_id) {
        let phase = item.phase.as_deref().unwrap_or("unknown");
        let _ = coordinator
            .write_worklog(&item.id, &item.title, phase, "Failed", reason)
            .await;
    }

    coordinator
        .update_item(item_id, ItemUpdate::SetBlocked(reason.to_string()))
        .await?;

    state.items_blocked.push(item_id.to_string());
    state.consecutive_exhaustions += 1;

    cleanup_terminal_summary(item_id, previous_summaries);
    Ok(())
}

async fn handle_phase_blocked(
    snapshot: &BacklogFile,
    item_id: &str,
    reason: &str,
    coordinator: &CoordinatorHandle,
    state: &mut SchedulerState,
    previous_summaries: &mut HashMap<String, String>,
) -> Result<(), String> {
    log_info!("[{}] Phase blocked: {}", item_id, reason);

    // Write worklog entry
    if let Some(item) = snapshot.items.iter().find(|i| i.id == item_id) {
        let phase = item.phase.as_deref().unwrap_or("unknown");
        let _ = coordinator
            .write_worklog(&item.id, &item.title, phase, "Blocked", reason)
            .await;
    }

    coordinator
        .update_item(item_id, ItemUpdate::SetBlocked(reason.to_string()))
        .await?;

    state.items_blocked.push(item_id.to_string());
    state.consecutive_exhaustions = 0;

    cleanup_terminal_summary(item_id, previous_summaries);
    Ok(())
}

/// Parse the numeric suffix from an item ID (e.g., "WRK-042" -> 42).
fn parse_item_numeric_suffix(id: &str) -> Option<u32> {
    id.rsplit('-').next().and_then(|s| s.parse().ok())
}

/// Process duplicate merges reported by triage.
///
/// For each duplicate, determines merge direction by numeric suffix (higher merges into lower).
/// Returns `true` if the current item was merged away (caller should skip further processing).
async fn process_merges(
    item_id: &str,
    duplicates: &[String],
    coordinator: &CoordinatorHandle,
    state: &mut SchedulerState,
) -> Result<bool, String> {
    if duplicates.is_empty() {
        return Ok(false);
    }

    let current_num = parse_item_numeric_suffix(item_id);

    for dup_id in duplicates {
        // Validate the duplicate exists and isn't Done
        let snap = pg_item::to_backlog_file(&coordinator.get_snapshot().await?);
        let dup_item = match snap.items.iter().find(|i| i.id == *dup_id) {
            Some(item) if item.status == ItemStatus::Done => {
                log_info!(
                    "[{}] Skipping merge with {} (already done)",
                    item_id,
                    dup_id
                );
                continue;
            }
            Some(item) => item.clone(),
            None => {
                log_warn!(
                    "[{}] Skipping merge with {} (not found in backlog)",
                    item_id,
                    dup_id
                );
                continue;
            }
        };

        // Determine direction: higher numeric ID merges into lower
        let dup_num = parse_item_numeric_suffix(dup_id);
        let (source_id, target_id) = match (current_num, dup_num) {
            (Some(c), Some(d)) if c > d => (item_id, dup_id.as_str()),
            (Some(c), Some(d)) if d > c => (dup_id.as_str(), item_id),
            _ => {
                // Fallback: current item (being triaged) is the source
                (item_id, dup_id.as_str())
            }
        };

        // Write worklog for the item being merged away
        let merged_away_item = if source_id == item_id {
            snap.items.iter().find(|i| i.id == item_id).cloned()
        } else {
            Some(dup_item)
        };
        if let Some(item) = merged_away_item {
            let _ = coordinator
                .write_worklog(
                    &item.id,
                    &item.title,
                    "triage",
                    "Merged",
                    &format!("Merged into {}", target_id),
                )
                .await;
        }

        // Perform the merge
        match coordinator.merge_item(source_id, target_id).await {
            Ok(()) => {
                log_info!(
                    "[{}] Merged {} into {} (duplicate)",
                    item_id,
                    source_id,
                    target_id
                );
                state.items_merged += 1;

                // If current item was merged away, signal caller to stop processing
                if source_id == item_id {
                    return Ok(true);
                }
            }
            Err(e) => {
                log_warn!(
                    "[{}] Failed to merge {} into {}: {}",
                    item_id,
                    source_id,
                    target_id,
                    e
                );
            }
        }
    }

    Ok(false)
}

async fn handle_triage_success(
    snapshot: &BacklogFile,
    item_id: &str,
    phase_result: &PhaseResult,
    coordinator: &CoordinatorHandle,
    config: &PhaseGolemConfig,
    state: &mut SchedulerState,
) -> Result<(), String> {
    log_info!(
        "[{}][TRIAGE] Result: {} — {}",
        item_id,
        match phase_result.result {
            ResultCode::PhaseComplete => "PHASE_COMPLETE",
            ResultCode::Failed => "FAILED",
            ResultCode::Blocked => "BLOCKED",
            ResultCode::SubphaseComplete => "SUBPHASE_COMPLETE",
        },
        phase_result.summary
    );

    // Write worklog entry for triage
    if let Some(item) = snapshot.items.iter().find(|i| i.id == item_id) {
        let outcome = match phase_result.result {
            ResultCode::PhaseComplete => "Complete",
            ResultCode::Failed => "Failed",
            ResultCode::Blocked => "Blocked",
            ResultCode::SubphaseComplete => "Subphase Complete",
        };
        let _ = coordinator
            .write_worklog(&item.id, &item.title, "triage", outcome, &phase_result.summary)
            .await;
    }

    // Ingest follow-ups from triage
    let fu_count = ingest_follow_ups(coordinator, phase_result, config).await;
    state.follow_ups_created += fu_count;

    // Process duplicate merges before committing
    let is_merged = process_merges(item_id, &phase_result.duplicates, coordinator, state).await?;
    if is_merged {
        // Current item was merged away — commit and skip further processing
        coordinator
            .complete_phase(item_id, phase_result.clone(), true)
            .await
            .ok(); // Item may be gone, ignore errors
        return Ok(());
    }

    // Commit triage output
    coordinator
        .complete_phase(item_id, phase_result.clone(), true) // immediate commit
        .await?;

    // Apply triage result (route item based on assessments)
    apply_triage_result(coordinator, item_id, phase_result, config).await?;

    // Check if item got blocked by triage
    let triage_snap = pg_item::to_backlog_file(&coordinator.get_snapshot().await?);
    if let Some(item) = triage_snap.items.iter().find(|i| i.id == item_id) {
        if item.status == ItemStatus::Blocked {
            state.items_blocked.push(item_id.to_string());
        }
    }

    Ok(())
}

// --- Promotion ---

async fn handle_promote(
    snapshot: &BacklogFile,
    coordinator: &CoordinatorHandle,
    item_id: &str,
    config: &PhaseGolemConfig,
) -> Result<(), String> {
    let item = snapshot
        .items
        .iter()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("Item {} not found for promotion", item_id))?;

    let pipeline_type = item.pipeline_type.as_deref().unwrap_or("feature");
    let pipeline = config
        .pipelines
        .get(pipeline_type)
        .ok_or_else(|| format!("Pipeline '{}' not found for promotion", pipeline_type))?;

    let first_phase = pipeline
        .phases
        .first()
        .ok_or_else(|| format!("Pipeline '{}' has no main phases", pipeline_type))?;

    coordinator
        .update_item(
            item_id,
            ItemUpdate::TransitionStatus(ItemStatus::InProgress),
        )
        .await?;
    coordinator
        .update_item(item_id, ItemUpdate::SetPhase(first_phase.name.clone()))
        .await?;
    coordinator
        .update_item(item_id, ItemUpdate::SetPhasePool(PhasePool::Main))
        .await?;

    log_info!(
        "{} → in_progress (starting at {})",
        item_id,
        first_phase.name
    );
    Ok(())
}

// --- Triage spawning ---

async fn spawn_triage(
    join_set: &mut JoinSet<(String, PhaseExecutionResult)>,
    running: &mut RunningTasks,
    coordinator: &CoordinatorHandle,
    runner: Arc<impl AgentRunner + 'static>,
    config: &PhaseGolemConfig,
    item_id: &str,
    root: &Path,
) {
    log_info!("[{}][TRIAGE] Starting triage", item_id);

    running.insert(
        item_id.to_string(),
        RunningTaskInfo {
            phase: "triage".to_string(),
            phase_pool: PhasePool::Pre,
            is_destructive: false,
        },
    );

    let coord = coordinator.clone();
    let cfg = config.clone();
    let item_id = item_id.to_string();
    let root = root.to_path_buf();

    join_set.spawn(async move {
        let pg_snap = match coord.get_snapshot().await {
            Ok(s) => s,
            Err(e) => {
                return (
                    item_id,
                    PhaseExecutionResult::Failed(format!("Failed to get snapshot: {}", e)),
                )
            }
        };
        let snap = pg_item::to_backlog_file(&pg_snap);
        let item: BacklogItem = match pg_snap.iter().find(|i| i.id() == item_id) {
            Some(i) => i.clone().into(),
            None => {
                return (
                    item_id,
                    PhaseExecutionResult::Failed("Item not found".to_string()),
                )
            }
        };

        let backlog_summary = prompt::build_backlog_summary(&snap.items, &item_id);
        let result_path = executor::result_file_path(&root, &item_id, "triage");
        let prompt_str = prompt::build_triage_prompt(
            &item,
            &result_path,
            &cfg.pipelines,
            backlog_summary.as_deref(),
        );
        let timeout = Duration::from_secs(cfg.execution.phase_timeout_minutes as u64 * 60);

        match runner.run_agent(&prompt_str, &result_path, timeout).await {
            Ok(phase_result) => (item_id, PhaseExecutionResult::Success(phase_result)),
            Err(e) => (item_id, PhaseExecutionResult::Failed(e)),
        }
    });
}

// --- Triage result handling ---

pub async fn apply_triage_result(
    coordinator: &CoordinatorHandle,
    item_id: &str,
    result: &PhaseResult,
    config: &PhaseGolemConfig,
) -> Result<(), String> {
    // Apply assessment updates
    if let Some(ref assessments) = result.updated_assessments {
        coordinator
            .update_item(item_id, ItemUpdate::UpdateAssessments(assessments.clone()))
            .await?;
    }

    // Apply structured description if provided and non-empty
    if let Some(ref description) = result.description {
        if !description.is_empty() {
            coordinator
                .update_item(item_id, ItemUpdate::SetDescription(description.clone()))
                .await?;
        }
    }

    // Apply pipeline_type if provided
    if let Some(ref pipeline_type) = result.pipeline_type {
        // Validate pipeline type exists
        if config.pipelines.contains_key(pipeline_type) {
            coordinator
                .update_item(item_id, ItemUpdate::SetPipelineType(pipeline_type.clone()))
                .await?;
        } else {
            // Invalid pipeline type — block
            coordinator
                .update_item(
                    item_id,
                    ItemUpdate::SetBlocked(format!(
                        "Triage assigned invalid pipeline type '{}'. Available: {}",
                        pipeline_type,
                        config
                            .pipelines
                            .keys()
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                )
                .await?;
            return Ok(());
        }
    }

    match result.result {
        ResultCode::PhaseComplete => {
            // Get current item state to check routing
            let route_snap = pg_item::to_backlog_file(&coordinator.get_snapshot().await?);
            let item = route_snap
                .items
                .iter()
                .find(|i| i.id == item_id)
                .ok_or_else(|| format!("Item {} not found after triage", item_id))?;

            let is_small_low_risk = matches!(item.size, Some(SizeLevel::Small))
                && matches!(item.risk, Some(DimensionLevel::Low) | None);

            let pipeline_type = item.pipeline_type.as_deref().unwrap_or("feature");
            let pipeline = config.pipelines.get(pipeline_type);
            let has_pre_phases = pipeline.map(|p| !p.pre_phases.is_empty()).unwrap_or(false);

            if is_small_low_risk || !has_pre_phases {
                // Small+low-risk or no pre_phases: promote to Scoping then Ready
                coordinator
                    .update_item(item_id, ItemUpdate::TransitionStatus(ItemStatus::Scoping))
                    .await?;
                coordinator
                    .update_item(item_id, ItemUpdate::TransitionStatus(ItemStatus::Ready))
                    .await?;
            } else {
                // Needs pre-phases: transition to Scoping and set first pre_phase
                coordinator
                    .update_item(item_id, ItemUpdate::TransitionStatus(ItemStatus::Scoping))
                    .await?;

                if let Some(p) = pipeline {
                    if let Some(first_pre) = p.pre_phases.first() {
                        coordinator
                            .update_item(item_id, ItemUpdate::SetPhase(first_pre.name.clone()))
                            .await?;
                        coordinator
                            .update_item(item_id, ItemUpdate::SetPhasePool(PhasePool::Pre))
                            .await?;
                    }
                }
            }
        }
        ResultCode::Blocked => {
            let reason = result
                .context
                .as_deref()
                .unwrap_or(&result.summary)
                .to_string();
            coordinator
                .update_item(item_id, ItemUpdate::SetBlocked(reason))
                .await?;
        }
        _ => {
            // Failed or SubphaseComplete — stay in New
        }
    }

    Ok(())
}

// --- Follow-up ingestion ---

async fn ingest_follow_ups(
    coordinator: &CoordinatorHandle,
    result: &PhaseResult,
    _config: &PhaseGolemConfig,
) -> u32 {
    if result.follow_ups.is_empty() {
        return 0;
    }

    let origin = format!("{}/{}", result.item_id, result.phase);
    match coordinator
        .ingest_follow_ups(result.follow_ups.clone(), &origin)
        .await
    {
        Ok(new_ids) => new_ids.len() as u32,
        Err(e) => {
            log_warn!("Warning: failed to ingest follow-ups: {}", e);
            0
        }
    }
}

// --- Drain helper ---

async fn drain_join_set(
    join_set: &mut JoinSet<(String, PhaseExecutionResult)>,
    running: &mut RunningTasks,
    state: &mut SchedulerState,
    coordinator: &CoordinatorHandle,
    config: &PhaseGolemConfig,
    previous_summaries: &mut HashMap<String, String>,
) {
    while let Some(result) = join_set.join_next().await {
        match result {
            Ok((item_id, exec_result)) => {
                running.remove(&item_id);
                let _ = handle_task_completion(
                    &item_id,
                    exec_result,
                    coordinator,
                    config,
                    state,
                    previous_summaries,
                )
                .await;
            }
            Err(e) => {
                log_debug!("Task join error during drain: {}", e);
            }
        }
    }
}

// --- Internal state ---

struct SchedulerState {
    phases_executed: u32,
    cap: u32,
    consecutive_exhaustions: u32,
    items_completed: Vec<String>,
    items_blocked: Vec<String>,
    follow_ups_created: u32,
    items_merged: u32,
    current_target_index: usize,
}

impl SchedulerState {
    fn is_cap_reached(&self) -> bool {
        self.phases_executed >= self.cap
    }

    fn is_circuit_breaker_tripped(&self) -> bool {
        self.consecutive_exhaustions >= CIRCUIT_BREAKER_THRESHOLD
    }
}

fn build_summary(mut state: SchedulerState, halt_reason: HaltReason) -> RunSummary {
    state.items_blocked.sort();
    state.items_blocked.dedup();
    RunSummary {
        phases_executed: state.phases_executed,
        items_completed: state.items_completed,
        items_blocked: state.items_blocked,
        follow_ups_created: state.follow_ups_created,
        items_merged: state.items_merged,
        halt_reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_summary_deduplicates_items_blocked() {
        let state = SchedulerState {
            phases_executed: 0,
            cap: 100,
            consecutive_exhaustions: 0,
            items_completed: Vec::new(),
            items_blocked: vec![
                "WRK-003".to_string(),
                "WRK-001".to_string(),
                "WRK-002".to_string(),
                "WRK-001".to_string(),
            ],
            follow_ups_created: 0,
            items_merged: 0,
            current_target_index: 0,
        };

        let summary = build_summary(state, HaltReason::TargetCompleted);

        assert_eq!(summary.items_blocked.len(), 3);
        assert_eq!(summary.items_blocked, vec!["WRK-001", "WRK-002", "WRK-003"]);
    }
}
