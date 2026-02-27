use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::agent::AgentRunner;
use crate::config::{
    GuardrailsConfig, PhaseConfig, PhaseGolemConfig, PipelineConfig, StalenessAction,
};
use crate::coordinator::CoordinatorHandle;
use crate::pg_item::PgItem;
use crate::prompt;
use crate::types::{
    DimensionLevel, ItemStatus, ItemUpdate, PhaseExecutionResult, PhasePool, PhaseResult,
    ResultCode, SizeLevel,
};
use crate::{log_info, log_warn};

// --- Result identity validation ---

/// Validate that a phase result's identity metadata matches expectations.
///
/// Returns `Ok(())` if `result.item_id` and `result.phase` match the expected values.
/// Returns `Err` with a descriptive message on mismatch. This applies to ALL result
/// codes — even a `Failed` result should have correct identity metadata.
pub fn validate_result_identity(
    result: &PhaseResult,
    expected_item_id: &str,
    expected_phase: &str,
) -> Result<(), String> {
    let mut mismatches = Vec::new();

    if result.item_id != expected_item_id {
        mismatches.push(format!(
            "item_id: expected '{}', got '{}'",
            expected_item_id, result.item_id
        ));
    }

    if result.phase != expected_phase {
        mismatches.push(format!(
            "phase: expected '{}', got '{}'",
            expected_phase, result.phase
        ));
    }

    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Result identity mismatch: {}",
            mismatches.join("; ")
        ))
    }
}

// --- Staleness ---

/// Result of a staleness check before phase execution.
#[derive(Debug, PartialEq)]
pub enum StalenessResult {
    /// No staleness detected, proceed with execution.
    Proceed,
    /// Phase artifacts may be stale, but config says warn and continue.
    Warn,
    /// Phase artifacts are stale and config says block.
    Block(String),
}

/// Check whether a prior phase's artifacts are stale relative to current HEAD.
///
/// Only meaningful for destructive phases. Non-destructive phases skip this check.
///
/// Logic:
/// - No `last_phase_commit` → Proceed (first phase or legacy item)
/// - SHA is ancestor of HEAD (exit 0) → Proceed (not stale)
/// - SHA is NOT ancestor (exit 1) → depends on `staleness` config:
///   - Ignore → Proceed
///   - Warn → Warn
///   - Block → Block with reason
/// - Unknown commit (exit 128 / error) → Block regardless of config (data integrity)
pub async fn check_staleness(
    item: &PgItem,
    phase_config: &PhaseConfig,
    coordinator: &CoordinatorHandle,
) -> StalenessResult {
    let last_commit = match item.last_phase_commit() {
        Some(sha) => sha,
        None => return StalenessResult::Proceed,
    };

    match coordinator.is_ancestor(&last_commit).await {
        Ok(true) => StalenessResult::Proceed,
        Ok(false) => {
            // Commit no longer in history (e.g., after rebase)
            match phase_config.staleness {
                StalenessAction::Ignore => StalenessResult::Proceed,
                StalenessAction::Warn => StalenessResult::Warn,
                StalenessAction::Block => StalenessResult::Block(format!(
                    "Stale: prior phase based on commit {} no longer in history",
                    last_commit
                )),
            }
        }
        Err(e) => {
            // Unknown commit or git error — block regardless of config
            StalenessResult::Block(format!(
                "Staleness check failed for commit {}: {}",
                last_commit, e
            ))
        }
    }
}

// --- Transition resolution ---

/// Determine what item updates to apply after a phase completes.
///
/// This is a pure function — no I/O, no async, trivially testable.
///
/// Returns a list of `ItemUpdate` mutations that the caller should apply
/// to the item via the coordinator.
///
/// Cases:
/// - Last pre_phase completed → check guardrails → ClearPhase + Ready, or SetBlocked
/// - Last main phase completed → TransitionStatus(Done)
/// - Mid-pipeline → SetPhase(next) + SetLastPhaseCommit
/// - Phase failed (result code) → SetBlocked with reason
/// - Retry exhaustion → SetBlocked with reason
pub fn resolve_transition(
    item: &PgItem,
    result: &PhaseResult,
    pipeline: &PipelineConfig,
    guardrails: &GuardrailsConfig,
) -> Vec<ItemUpdate> {
    match result.result {
        ResultCode::PhaseComplete => resolve_phase_complete(item, result, pipeline, guardrails),
        ResultCode::Failed => {
            // Failed result: the caller handles retry counting.
            // If we get here, retries are exhausted.
            vec![ItemUpdate::SetBlocked(format!(
                "Phase {} failed after retries exhausted. Last failure: {}",
                result.phase, result.summary
            ))]
        }
        ResultCode::Blocked => {
            let reason = result
                .context
                .as_deref()
                .unwrap_or(&result.summary)
                .to_string();
            vec![ItemUpdate::SetBlocked(reason)]
        }
        ResultCode::SubphaseComplete => {
            // SubphaseComplete is handled by the caller (executor loop),
            // not by resolve_transition. This branch should not be reached.
            vec![]
        }
    }
}

fn resolve_phase_complete(
    item: &PgItem,
    result: &PhaseResult,
    pipeline: &PipelineConfig,
    guardrails: &GuardrailsConfig,
) -> Vec<ItemUpdate> {
    let phase_pool = item.phase_pool();
    let current_phase = result.phase.as_str();

    match phase_pool.as_ref() {
        Some(PhasePool::Pre) => {
            // Check if this is the last pre_phase
            let is_last = pipeline
                .pre_phases
                .last()
                .map(|p| p.name == current_phase)
                .unwrap_or(false);

            if is_last {
                // Last pre_phase: check guardrails for auto-promote
                if item.requires_human_review() {
                    return vec![ItemUpdate::SetBlocked(
                        "Requires human review before entering pipeline".to_string(),
                    )];
                }

                if !passes_guardrails(item, guardrails) {
                    return vec![ItemUpdate::SetBlocked(
                        "Exceeds autonomous guardrail thresholds".to_string(),
                    )];
                }

                vec![
                    ItemUpdate::ClearPhase,
                    ItemUpdate::TransitionStatus(ItemStatus::Ready),
                ]
            } else {
                // Mid pre_phases: advance to next
                let next = next_phase_in_list(&pipeline.pre_phases, current_phase);
                match next {
                    Some(name) => {
                        let mut updates = vec![ItemUpdate::SetPhase(name)];
                        if let Some(ref sha) = result.based_on_commit {
                            updates.push(ItemUpdate::SetLastPhaseCommit(sha.clone()));
                        }
                        updates
                    }
                    None => vec![ItemUpdate::SetBlocked(format!(
                        "Phase {} not found in pre_phases",
                        current_phase
                    ))],
                }
            }
        }
        Some(PhasePool::Main) | None => {
            // Check if this is the last main phase
            let is_last = pipeline
                .phases
                .last()
                .map(|p| p.name == current_phase)
                .unwrap_or(false);

            if is_last {
                vec![ItemUpdate::TransitionStatus(ItemStatus::Done)]
            } else {
                let next = next_phase_in_list(&pipeline.phases, current_phase);
                match next {
                    Some(name) => {
                        let mut updates = vec![ItemUpdate::SetPhase(name)];
                        if let Some(ref sha) = result.based_on_commit {
                            updates.push(ItemUpdate::SetLastPhaseCommit(sha.clone()));
                        }
                        updates
                    }
                    None => vec![ItemUpdate::SetBlocked(format!(
                        "Phase {} not found in pipeline phases",
                        current_phase
                    ))],
                }
            }
        }
    }
}

/// Find the next phase name after `current` in the given phase list.
fn next_phase_in_list(phases: &[PhaseConfig], current: &str) -> Option<String> {
    let idx = phases.iter().position(|p| p.name == current)?;
    phases.get(idx + 1).map(|p| p.name.clone())
}

// --- Guardrails ---

/// Check if an item passes all guardrail thresholds.
///
/// An item passes if all of its dimensions are within the configured maximums.
/// Missing dimensions are treated as passing (no data = no concern).
pub fn passes_guardrails(item: &PgItem, guardrails: &GuardrailsConfig) -> bool {
    let size_ok = match item.size() {
        Some(ref size) => size_level_value(size) <= size_level_value(&guardrails.max_size),
        None => true,
    };

    let complexity_ok = match item.complexity() {
        Some(ref complexity) => {
            dimension_level_value(complexity) <= dimension_level_value(&guardrails.max_complexity)
        }
        None => true,
    };

    let risk_ok = match item.risk() {
        Some(ref risk) => {
            dimension_level_value(risk) <= dimension_level_value(&guardrails.max_risk)
        }
        None => true,
    };

    size_ok && complexity_ok && risk_ok
}

fn size_level_value(level: &SizeLevel) -> u8 {
    match level {
        SizeLevel::Small => 1,
        SizeLevel::Medium => 2,
        SizeLevel::Large => 3,
    }
}

fn dimension_level_value(level: &DimensionLevel) -> u8 {
    match level {
        DimensionLevel::Low => 1,
        DimensionLevel::Medium => 2,
        DimensionLevel::High => 3,
    }
}

// --- Phase execution ---

/// Execute a single phase for a backlog item.
///
/// This is the core execution function that:
/// 1. Checks staleness (destructive phases only)
/// 2. Records phase start (captures HEAD SHA)
/// 3. Builds the prompt
/// 4. Runs workflows sequentially with retry
/// 5. Returns the execution result (caller applies transitions)
///
/// The executor does NOT apply transitions itself — it returns a
/// `PhaseExecutionResult` that the scheduler uses to drive coordinator updates.
#[allow(clippy::too_many_arguments)]
pub async fn execute_phase(
    item: &PgItem,
    phase_config: &PhaseConfig,
    config: &PhaseGolemConfig,
    coordinator: &CoordinatorHandle,
    runner: &impl AgentRunner,
    cancel: &CancellationToken,
    root: &Path,
    previous_summary: Option<&str>,
    config_base: &Path,
) -> PhaseExecutionResult {
    // 1. Staleness check (destructive phases only)
    if phase_config.is_destructive {
        match check_staleness(item, phase_config, coordinator).await {
            StalenessResult::Proceed => {}
            StalenessResult::Warn => {
                log_warn!(
                    "[{}][{}] Warning: prior phase artifacts may be stale",
                    item.id(),
                    phase_config.name.to_uppercase()
                );
            }
            StalenessResult::Block(reason) => {
                return PhaseExecutionResult::Blocked(reason);
            }
        }
    }

    // 2. Record phase start (capture HEAD SHA)
    let head_sha = match coordinator.get_head_sha().await {
        Ok(sha) => sha,
        Err(e) => return PhaseExecutionResult::Failed(format!("Failed to get HEAD SHA: {}", e)),
    };

    if let Err(e) = coordinator.record_phase_start(item.id(), &head_sha).await {
        return PhaseExecutionResult::Failed(format!("Failed to record phase start: {}", e));
    }

    // 3. Build prompt and paths
    let result_path = result_file_path(root, item.id(), &phase_config.name);
    let change_folder = match resolve_or_find_change_folder(root, item.id(), item.title()).await {
        Ok(path) => path,
        Err(e) => return PhaseExecutionResult::Failed(e),
    };

    let timeout = Duration::from_secs(config.execution.phase_timeout_minutes as u64 * 60);
    let max_attempts = config.execution.max_retries + 1;

    // 4. Log CLI tool and model for this phase
    log_info!(
        "[{}][{}] Using {} (model: {})",
        item.id(),
        phase_config.name.to_uppercase(),
        config.agent.cli.display_name(),
        config.agent.model.as_deref().unwrap_or("default")
    );

    // 5. Retry loop
    let mut failure_context: Option<String> = None;

    for attempt in 1..=max_attempts {
        if cancel.is_cancelled() {
            return PhaseExecutionResult::Cancelled;
        }

        log_info!(
            "[{}][{}] Starting phase (attempt {}/{})",
            item.id(),
            phase_config.name.to_uppercase(),
            attempt,
            max_attempts
        );

        let prompt = build_executor_prompt(
            &phase_config.name,
            phase_config,
            item,
            &result_path,
            &change_folder,
            previous_summary,
            item.unblock_context().as_deref(),
            failure_context.as_deref(),
            config_base,
        );

        // Currently workflows are encoded in the prompt, and a single agent run
        // executes them all. Multi-workflow phases run as a single agent invocation
        // (the prompt lists all workflow files).
        let workflow_result = tokio::select! {
            result = runner.run_agent(&prompt, &result_path, timeout) => result,
            _ = cancel.cancelled() => return PhaseExecutionResult::Cancelled,
        };

        match workflow_result {
            Ok(phase_result) => {
                // Validate result identity before processing — non-retryable on mismatch
                if let Err(e) =
                    validate_result_identity(&phase_result, item.id(), &phase_config.name)
                {
                    return PhaseExecutionResult::Failed(e);
                }

                match phase_result.result {
                    ResultCode::SubphaseComplete => {
                        return PhaseExecutionResult::SubphaseComplete(phase_result);
                    }
                    ResultCode::PhaseComplete => {
                        return PhaseExecutionResult::Success(phase_result);
                    }
                    ResultCode::Blocked => {
                        let reason = phase_result
                            .context
                            .as_deref()
                            .unwrap_or(&phase_result.summary)
                            .to_string();
                        return PhaseExecutionResult::Blocked(reason);
                    }
                    ResultCode::Failed => {
                        if attempt >= max_attempts {
                            return PhaseExecutionResult::Failed(format!(
                                "Phase {} failed after {} attempts. Last failure: {}",
                                phase_config.name, attempt, phase_result.summary
                            ));
                        }
                        log_info!(
                            "[{}][{}] Failed (attempt {}/{}): {}",
                            item.id(),
                            phase_config.name.to_uppercase(),
                            attempt,
                            max_attempts,
                            phase_result.summary
                        );
                        failure_context = Some(phase_result.summary);
                    }
                }
            }
            Err(e) => {
                if attempt >= max_attempts {
                    return PhaseExecutionResult::Failed(format!(
                        "Phase {} failed after {} attempts. Last error: {}",
                        phase_config.name, attempt, e
                    ));
                }
                log_info!(
                    "[{}][{}] Agent error (attempt {}/{}): {}",
                    item.id(),
                    phase_config.name.to_uppercase(),
                    attempt,
                    max_attempts,
                    e
                );
                failure_context = Some(e);
            }
        }
    }

    // Should not be reached due to loop logic, but safety fallback
    PhaseExecutionResult::Failed(format!(
        "Phase {} failed: retry loop exited unexpectedly",
        phase_config.name
    ))
}

// --- Prompt building ---

/// Build the prompt for executor-driven phase execution.
///
/// Uses the existing prompt infrastructure with the context preamble.
#[allow(clippy::too_many_arguments)]
fn build_executor_prompt(
    phase: &str,
    phase_config: &PhaseConfig,
    item: &PgItem,
    result_path: &Path,
    change_folder: &Path,
    previous_summary: Option<&str>,
    unblock_notes: Option<&str>,
    failure_context: Option<&str>,
    config_base: &Path,
) -> String {
    let params = prompt::PromptParams {
        phase,
        phase_config,
        item,
        result_path,
        change_folder,
        previous_summary,
        unblock_notes,
        failure_context,
        config_base,
    };
    prompt::build_prompt(&params)
}

// --- Path helpers ---

/// Generate the result file path for a phase.
pub fn result_file_path(root: &Path, item_id: &str, phase: &str) -> PathBuf {
    root.join(".phase-golem")
        .join(format!("phase_result_{}_{}.json", item_id, phase))
}

/// Resolve an existing change folder or create one if not found.
///
/// Searches the `changes/` directory for a folder prefixed with `{item_id}_`.
/// Falls back to creating `{item_id}_{slugified_title}` if none exists.
async fn resolve_or_find_change_folder(
    root: &Path,
    item_id: &str,
    title: &str,
) -> Result<PathBuf, String> {
    let changes_dir = root.join("changes");
    let prefix = format!("{}_", item_id);

    match tokio::fs::read_dir(&changes_dir).await {
        Ok(mut entries) => {
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| format!("Failed to read directory entry: {}", e))?
            {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&prefix)
                    && entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false)
                {
                    return Ok(entry.path());
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Directory doesn't exist yet — fall through to creation
        }
        Err(e) => {
            return Err(format!("Failed to read {}: {}", changes_dir.display(), e));
        }
    }

    // Create the directory if it doesn't exist
    let slug = slugify(title);
    let folder_name = format!("{}_{}", item_id, slug);
    let folder_path = changes_dir.join(folder_name);
    tokio::fs::create_dir_all(&folder_path)
        .await
        .map_err(|e| format!("Failed to create {}: {}", folder_path.display(), e))?;
    Ok(folder_path)
}

/// Convert a title to a URL-friendly slug.
pub fn slugify(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<&str>>()
        .join("-")
}
