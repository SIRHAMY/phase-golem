use crate::agent::TrustedExecutorAdapter;
use crate::config::PhaseGolemConfig;
use crate::coordinator::{CoordinatorHandle, WorkSnapshot};
use crate::executor::{self, DeterministicVerifier};
use crate::filter::{self, FilterCriterion};
use crate::pg_item::PgItem;
use crate::types::{ItemUpdate, SupervisedOutcome};
use task_golem::model::status::Status;
use tokio_util::sync::CancellationToken;

#[derive(Debug, PartialEq, Eq)]
pub enum HaltReason {
    Idle,
    ReadyHumanGate,
    BudgetReached,
    UnrecoverableFailure,
    ShutdownRequested,
    SelectedScopeComplete,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RunSummary {
    pub tasks_executed: u32,
    pub items_completed: Vec<String>,
    pub items_blocked: Vec<String>,
    pub halt_reason: HaltReason,
}

pub struct RunParams {
    pub targets: Vec<String>,
    pub filter: Vec<FilterCriterion>,
    pub cap: u32,
}

/// Drains one selected TG work scope in the foreground, one task at a time.
///
/// Each iteration reads TG readiness again so executor-created work can become
/// eligible in a later cycle. The loop never resumes a pre-existing claim.
pub async fn run_foreground_supervisor(
    coordinator: CoordinatorHandle,
    config: &PhaseGolemConfig,
    params: &RunParams,
    cancel: &CancellationToken,
    adapter: &impl TrustedExecutorAdapter,
    verifier: &impl DeterministicVerifier,
) -> Result<RunSummary, String> {
    let mut summary = RunSummary {
        tasks_executed: 0,
        items_completed: Vec::new(),
        items_blocked: Vec::new(),
        halt_reason: HaltReason::Idle,
    };
    let mut selected_work_seen = false;

    loop {
        if cancel.is_cancelled() {
            summary.halt_reason = HaltReason::ShutdownRequested;
            return Ok(summary);
        }
        if summary.tasks_executed >= params.cap {
            summary.halt_reason = HaltReason::BudgetReached;
            return Ok(summary);
        }

        let snapshot = coordinator.get_snapshot().await.map_err(String::from)?;
        if !snapshot.dependency_evaluation.integrity_issues.is_empty() {
            summary.halt_reason = HaltReason::UnrecoverableFailure;
            return Ok(summary);
        }
        let selected_items = selected_items(&snapshot, params);
        selected_work_seen |= !selected_items.is_empty();

        if selected_items
            .iter()
            .any(|item| item.status() == Status::Doing)
        {
            summary.halt_reason = HaltReason::UnrecoverableFailure;
            return Ok(summary);
        }

        let Some(item) = ready_selected_items(&snapshot, params).into_iter().next() else {
            summary.halt_reason = if selected_scope_is_complete(
                &snapshot,
                &selected_items,
                params,
                selected_work_seen,
            ) {
                HaltReason::SelectedScopeComplete
            } else {
                HaltReason::Idle
            };
            return Ok(summary);
        };
        if item.is_human_gate() {
            summary.halt_reason = HaltReason::ReadyHumanGate;
            return Ok(summary);
        }
        let item_id = item.id().to_string();

        let outcome = executor::supervise_trusted_executor_with_cancellation(
            &item_id,
            config,
            &coordinator,
            adapter,
            verifier,
            cancel,
        )
        .await;

        match outcome {
            Ok(SupervisedOutcome::Complete) => {
                summary.tasks_executed += 1;
                summary.items_completed.push(item_id.clone());
            }
            Ok(SupervisedOutcome::Blocked) => {
                summary.tasks_executed += 1;
                summary.items_blocked.push(item_id.clone());
            }
            Err(error) => {
                if cancel.is_cancelled() {
                    summary.halt_reason = HaltReason::ShutdownRequested;
                    return Ok(summary);
                }
                if item_is_claimed_for_execution(&coordinator, &item_id).await? {
                    let reason = format!("Unrecoverable supervised execution failure: {error}");
                    coordinator
                        .update_item(&item_id, ItemUpdate::SetBlocked(reason))
                        .await
                        .map_err(String::from)?;
                    summary.tasks_executed += 1;
                    summary.items_blocked.push(item_id.clone());
                }
                summary.halt_reason = HaltReason::UnrecoverableFailure;
                return Ok(summary);
            }
        }

        if cancel.is_cancelled() {
            summary.halt_reason = HaltReason::ShutdownRequested;
            return Ok(summary);
        }
    }
}

async fn item_is_claimed_for_execution(
    coordinator: &CoordinatorHandle,
    item_id: &str,
) -> Result<bool, String> {
    let snapshot = coordinator.get_snapshot().await.map_err(String::from)?;
    Ok(snapshot.iter().any(|item| {
        item.id() == item_id && item.status() == Status::Doing && item.is_claimed_for_pg_execution()
    }))
}

/// Allows an external scheduler to invoke the same finite loop as foreground execution.
pub async fn run_scheduled_wakeup(
    coordinator: CoordinatorHandle,
    config: &PhaseGolemConfig,
    params: &RunParams,
    cancel: &CancellationToken,
    adapter: &impl TrustedExecutorAdapter,
    verifier: &impl DeterministicVerifier,
) -> Result<RunSummary, String> {
    run_foreground_supervisor(coordinator, config, params, cancel, adapter, verifier).await
}

fn selected_items<'a>(snapshot: &'a WorkSnapshot, params: &RunParams) -> Vec<&'a PgItem> {
    snapshot
        .iter()
        .filter(|item| item.is_pg_owned())
        .filter(|item| is_in_selected_scope(item, params))
        .collect()
}

fn ready_selected_items<'a>(snapshot: &'a WorkSnapshot, params: &RunParams) -> Vec<&'a PgItem> {
    snapshot
        .dependency_evaluation
        .ready_items
        .iter()
        .filter_map(|ready| snapshot.iter().find(|item| item.id() == ready.id))
        .filter(|item| item.is_pg_owned())
        .filter(|item| is_in_selected_scope(item, params))
        .collect()
}

fn is_in_selected_scope(item: &PgItem, params: &RunParams) -> bool {
    (params.targets.is_empty() || params.targets.iter().any(|id| id == item.id()))
        && params
            .filter
            .iter()
            .all(|criterion| filter::matches_item(criterion, item))
}

fn selected_scope_is_complete(
    snapshot: &WorkSnapshot,
    selected_items: &[&PgItem],
    params: &RunParams,
    selected_work_seen: bool,
) -> bool {
    if !params.targets.is_empty() {
        return params.targets.iter().all(|target| {
            snapshot.archived_done_ids.contains(target)
                || selected_items.iter().any(|item| {
                    item.id() == target && matches!(item.status(), Status::Done | Status::Blocked)
                })
        });
    }

    selected_work_seen
        && selected_items
            .iter()
            .all(|item| matches!(item.status(), Status::Done | Status::Blocked))
}
