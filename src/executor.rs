use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use task_golem::model::status::Status;
use tokio_util::sync::CancellationToken;

use crate::agent::{
    kill_all_children, run_shell_command, AgentRunner, TrustedExecutionRequest,
    TrustedExecutorAdapter, MAX_TRUSTED_RESULT_PAYLOAD_BYTES,
};
use crate::config::{
    GuardrailsConfig, PhaseConfig, PhaseGolemConfig, PipelineConfig, PublicExecutionPolicy,
    StalenessAction, TrustedExecutorProfile, VerificationPlan,
};
use crate::coordinator::{CoordinatorHandle, ExpectedExecutionSnapshot, WorkSnapshot};
use crate::pg_item::PgItem;
use crate::prompt;
use crate::types::{
    AttemptOutcome, DimensionLevel, ItemUpdate, PhaseExecutionResult, PhasePool, PhaseResult,
    ResultCode, SizeLevel, SupervisedAttempt, SupervisedOutcome, SupervisedTransition,
    TrustedExecutorResult, TrustedResultCode,
};
use crate::{log_info, log_warn};

const MAX_REJECTED_SUMMARY_BYTES: usize = 256;
const MAX_REJECTED_EVIDENCE_REFERENCES: usize = 4;
const MAX_REJECTED_EVIDENCE_REFERENCE_BYTES: usize = 128;
const OVERSIZED_RESULT_SUMMARY: &str =
    "Trusted executor result exceeds the PG attempt evidence budget";
const BOUNDED_REJECTION_SUMMARY: &str =
    "Rejected attempt details exceeded the PG attempt evidence budget";
const REDACTED_RUNTIME_CREDENTIAL: &str = "[REDACTED]";

#[derive(Clone, Debug, PartialEq, Eq)]
struct SupervisionContext {
    phase: String,
    executor_profile: String,
    policy: PublicExecutionPolicy,
    verification: VerificationPlan,
    profile: TrustedExecutorProfile,
    title: String,
}

impl SupervisionContext {
    fn expected_execution_snapshot(&self) -> ExpectedExecutionSnapshot {
        ExpectedExecutionSnapshot::new(
            self.title.clone(),
            self.phase.clone(),
            self.executor_profile.clone(),
            self.policy.clone(),
            self.verification.clone(),
        )
    }

    fn request(&self, item_id: &str, attempt: u32) -> TrustedExecutionRequest {
        TrustedExecutionRequest {
            item_id: item_id.to_string(),
            phase: self.phase.clone(),
            title: self.title.clone(),
            attempt,
            policy: self.policy.clone(),
        }
    }

    fn attempt(
        &self,
        request: &TrustedExecutionRequest,
        outcome: AttemptOutcome,
        summary: String,
        executor_evidence: Vec<String>,
        verification_evidence: Vec<String>,
    ) -> SupervisedAttempt {
        SupervisedAttempt {
            schema: "phase-golem/trusted-executor-attempt/v1".to_string(),
            item_id: request.item_id.clone(),
            phase: self.phase.clone(),
            executor_profile: self.executor_profile.clone(),
            attempt: request.attempt,
            max_attempts: self.policy.max_retries.saturating_add(1),
            public_policy: self.policy.clone(),
            outcome,
            summary,
            executor_evidence,
            verification_evidence,
        }
    }
}

fn prepare_supervision_context(
    snapshot: &WorkSnapshot,
    item_id: &str,
    config: &PhaseGolemConfig,
) -> Result<SupervisionContext, String> {
    let item = snapshot
        .iter()
        .find(|item| item.id() == item_id)
        .ok_or_else(|| format!("Item '{item_id}' is not active"))?;
    let phase = item
        .template_node_key()
        .filter(|phase| !phase.trim().is_empty())
        .ok_or_else(|| format!("Item '{item_id}' has no valid template node snapshot"))?;
    let executor_profile = item.executor_profile_snapshot()?;
    let policy = item.execution_policy_snapshot()?;
    let verification = item.verification_snapshot()?;
    if policy.timeout_minutes == 0 {
        return Err(format!(
            "Item '{item_id}' execution policy timeout_minutes must be greater than zero"
        ));
    }
    let profile = config
        .executor_profiles
        .get(&executor_profile)
        .ok_or_else(|| format!("Trusted executor profile '{executor_profile}' is not configured"))?
        .clone();
    if profile.command.trim().is_empty() {
        return Err(format!(
            "Trusted executor profile '{executor_profile}' has no command"
        ));
    }

    let context = SupervisionContext {
        phase,
        executor_profile,
        policy,
        verification,
        profile,
        title: item.title().to_string(),
    };
    let max_attempts = context.policy.max_retries.saturating_add(1);
    let rejection_probe = context.attempt(
        &context.request(item_id, max_attempts),
        AttemptOutcome::Rejected,
        BOUNDED_REJECTION_SUMMARY.to_string(),
        Vec::new(),
        Vec::new(),
    );
    rejection_probe.validated_note_text().map_err(|error| {
        format!("Item '{item_id}' snapshots cannot produce bounded attempt evidence: {error}")
    })?;
    Ok(context)
}

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

pub fn parse_trusted_executor_result(payload: &str) -> Result<TrustedExecutorResult, String> {
    let result = serde_json::from_str::<TrustedExecutorResult>(payload)
        .map_err(|error| format!("Malformed trusted executor result: {error}"))?;
    if result.summary.trim().is_empty() {
        return Err("Malformed trusted executor result: summary must not be empty".to_string());
    }
    Ok(result)
}

fn validate_trusted_result_identity(
    result: &TrustedExecutorResult,
    expected_item_id: &str,
    expected_phase: &str,
) -> Result<(), String> {
    if result.item_id == expected_item_id && result.phase == expected_phase {
        return Ok(());
    }
    Err(format!(
        "Trusted executor result identity mismatch: expected item '{}' phase '{}', got item '{}' phase '{}'",
        expected_item_id, expected_phase, result.item_id, result.phase
    ))
}

pub trait DeterministicVerifier: Send + Sync {
    fn verify(
        &self,
        plan: &VerificationPlan,
        request: &TrustedExecutionRequest,
        executor_evidence: &[String],
    ) -> impl std::future::Future<Output = Result<Vec<String>, String>> + Send;
}

pub struct ShellVerifier {
    root: PathBuf,
}

impl ShellVerifier {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl DeterministicVerifier for ShellVerifier {
    async fn verify(
        &self,
        plan: &VerificationPlan,
        request: &TrustedExecutionRequest,
        _executor_evidence: &[String],
    ) -> Result<Vec<String>, String> {
        let mut evidence = Vec::with_capacity(plan.required_checks.len());
        for check in &plan.required_checks {
            if check.trim().is_empty() {
                return Err("Verification plan contains an empty required check".to_string());
            }
            let timeout = Duration::from_secs(request.policy.timeout_minutes as u64 * 60);
            let output = run_shell_command(check, &self.root, timeout)
                .await
                .map_err(|error| format!("Verification check '{check}' could not run: {error}"))?;
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(format!(
                    "Verification check '{check}' failed with {}: {stderr}",
                    output.status
                ));
            }
            evidence.push(format!("verification:{check}"));
        }
        Ok(evidence)
    }
}

pub async fn supervise_trusted_executor(
    item_id: &str,
    config: &PhaseGolemConfig,
    coordinator: &CoordinatorHandle,
    adapter: &impl TrustedExecutorAdapter,
    verifier: &impl DeterministicVerifier,
) -> Result<SupervisedOutcome, String> {
    supervise_trusted_executor_with_cancellation(
        item_id,
        config,
        coordinator,
        adapter,
        verifier,
        &CancellationToken::new(),
    )
    .await
}

pub async fn supervise_trusted_executor_with_cancellation(
    item_id: &str,
    config: &PhaseGolemConfig,
    coordinator: &CoordinatorHandle,
    adapter: &impl TrustedExecutorAdapter,
    verifier: &impl DeterministicVerifier,
    cancel: &CancellationToken,
) -> Result<SupervisedOutcome, String> {
    if cancel.is_cancelled() {
        return Err("Shutdown requested".to_string());
    }
    let prepared =
        prepare_supervision_context(&coordinator.get_snapshot().await?, item_id, config)?;
    cancel_aware(
        cancel,
        coordinator.claim_item_with_expected_execution_snapshot(
            item_id,
            prepared.expected_execution_snapshot(),
        ),
    )
    .await?
    .map_err(String::from)?;

    let max_attempts = prepared.policy.max_retries.saturating_add(1);
    for attempt in 1..=max_attempts {
        let request = prepared.request(item_id, attempt);
        let payload =
            match cancel_aware(cancel, adapter.invoke(&prepared.profile, &request)).await? {
                Ok(payload) => payload,
                Err(error) => {
                    record_rejected_attempt(coordinator, &prepared, &request, &error, &[]).await?;
                    if attempt < max_attempts {
                        continue;
                    }
                    return Err("Trusted executor failed after all attempts".to_string());
                }
            };
        if payload.len() > MAX_TRUSTED_RESULT_PAYLOAD_BYTES {
            record_rejected_attempt(
                coordinator,
                &prepared,
                &request,
                OVERSIZED_RESULT_SUMMARY,
                &[],
            )
            .await?;
            return Err(format!(
                "Trusted executor result is {} bytes, exceeding the {}-byte protocol limit",
                payload.len(),
                MAX_TRUSTED_RESULT_PAYLOAD_BYTES
            ));
        }

        let result = match parse_trusted_executor_result(&payload) {
            Ok(result) => result,
            Err(error) => {
                record_rejected_attempt(coordinator, &prepared, &request, &error, &[]).await?;
                return Err(redact_runtime_credentials(&prepared.profile, &error));
            }
        };
        if let Err(error) = validate_trusted_result_identity(&result, item_id, &prepared.phase) {
            record_rejected_attempt(
                coordinator,
                &prepared,
                &request,
                &error,
                &result.evidence_references,
            )
            .await?;
            return Err(redact_runtime_credentials(&prepared.profile, &error));
        }
        let result_outcome = match &result.result {
            TrustedResultCode::Complete => AttemptOutcome::Complete,
            TrustedResultCode::Blocked => AttemptOutcome::Blocked,
        };
        let durable_summary = redact_runtime_credentials(&prepared.profile, &result.summary);
        let durable_executor_evidence =
            redact_runtime_credential_references(&prepared.profile, &result.evidence_references);
        let result_attempt = prepared.attempt(
            &request,
            result_outcome.clone(),
            durable_summary.clone(),
            durable_executor_evidence.clone(),
            Vec::new(),
        );
        if result_attempt.validated_note_text().is_err() {
            record_rejected_attempt(
                coordinator,
                &prepared,
                &request,
                OVERSIZED_RESULT_SUMMARY,
                &[],
            )
            .await?;
            return Err(OVERSIZED_RESULT_SUMMARY.to_string());
        }

        let verification_evidence = match cancel_aware(
            cancel,
            verifier.verify(
                &prepared.verification,
                &request,
                &result.evidence_references,
            ),
        )
        .await?
        {
            Ok(evidence) => evidence,
            Err(error) => {
                let error = redact_runtime_credentials(&prepared.profile, &error);
                record_rejected_attempt(
                    coordinator,
                    &prepared,
                    &request,
                    &error,
                    &result.evidence_references,
                )
                .await?;
                return Err(format!("Deterministic verification failed: {error}"));
            }
        };
        let verification_evidence =
            redact_runtime_credential_references(&prepared.profile, &verification_evidence);
        let (attempt_outcome, transition, outcome) = match result.result {
            TrustedResultCode::Complete => (
                AttemptOutcome::Complete,
                SupervisedTransition::Complete,
                SupervisedOutcome::Complete,
            ),
            TrustedResultCode::Blocked => (
                AttemptOutcome::Blocked,
                SupervisedTransition::Blocked,
                SupervisedOutcome::Blocked,
            ),
        };
        let completed_attempt = prepared.attempt(
            &request,
            attempt_outcome,
            durable_summary,
            durable_executor_evidence,
            verification_evidence,
        );
        if completed_attempt.validated_note_text().is_err() {
            record_rejected_attempt(
                coordinator,
                &prepared,
                &request,
                OVERSIZED_RESULT_SUMMARY,
                &[],
            )
            .await?;
            return Err(OVERSIZED_RESULT_SUMMARY.to_string());
        }
        coordinator
            .finalize_supervised_attempt(
                completed_attempt,
                prepared.expected_execution_snapshot(),
                Some(transition),
            )
            .await?;
        return Ok(outcome);
    }

    Err("Trusted executor attempt loop exited unexpectedly".to_string())
}

async fn cancel_aware<T>(
    cancel: &CancellationToken,
    operation: impl Future<Output = T>,
) -> Result<T, String> {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            let _ = tokio::task::spawn_blocking(kill_all_children).await;
            Err("Shutdown requested".to_string())
        }
        result = operation => Ok(result),
    }
}

async fn record_rejected_attempt(
    coordinator: &CoordinatorHandle,
    context: &SupervisionContext,
    request: &TrustedExecutionRequest,
    summary: &str,
    executor_evidence: &[String],
) -> Result<(), String> {
    let bounded_summary = truncate_utf8(
        &redact_runtime_credentials(&context.profile, summary),
        MAX_REJECTED_SUMMARY_BYTES,
    );
    let bounded_evidence =
        redact_runtime_credential_references(&context.profile, executor_evidence)
            .into_iter()
            .take(MAX_REJECTED_EVIDENCE_REFERENCES)
            .map(|reference| truncate_utf8(&reference, MAX_REJECTED_EVIDENCE_REFERENCE_BYTES))
            .collect();
    let detailed = context.attempt(
        request,
        AttemptOutcome::Rejected,
        bounded_summary,
        bounded_evidence,
        Vec::new(),
    );
    let attempt = if detailed.validated_note_text().is_ok() {
        detailed
    } else {
        context.attempt(
            request,
            AttemptOutcome::Rejected,
            BOUNDED_REJECTION_SUMMARY.to_string(),
            Vec::new(),
            Vec::new(),
        )
    };
    attempt.validated_note_text()?;
    coordinator
        .finalize_supervised_attempt(attempt, context.expected_execution_snapshot(), None)
        .await
        .map_err(String::from)
}

fn redact_runtime_credentials(profile: &TrustedExecutorProfile, value: &str) -> String {
    let mut credentials = profile
        .environment
        .values()
        .filter(|credential| !credential.is_empty())
        .collect::<Vec<_>>();
    credentials.sort_unstable_by_key(|credential| std::cmp::Reverse(credential.len()));
    credentials
        .into_iter()
        .fold(value.to_string(), |redacted, credential| {
            redacted.replace(credential, REDACTED_RUNTIME_CREDENTIAL)
        })
}

fn redact_runtime_credential_references(
    profile: &TrustedExecutorProfile,
    references: &[String],
) -> Vec<String> {
    references
        .iter()
        .map(|reference| redact_runtime_credentials(profile, reference))
        .collect()
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
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
/// - Last pre-phase completed: enter the first main phase or block on guardrails
/// - Last main phase completed: transition to Done
/// - Mid-pipeline: advance phase and record the last phase commit
/// - Failed phase or retry exhaustion: block with diagnostics
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
    if item.template_node_key().is_some() {
        return vec![ItemUpdate::TransitionStatus(Status::Done)];
    }

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
                // The last preliminary phase must pass guardrails before main execution.
                if !passes_guardrails(item, guardrails) {
                    return vec![ItemUpdate::SetBlocked(
                        "Exceeds autonomous guardrail thresholds".to_string(),
                    )];
                }

                match pipeline.phases.first() {
                    Some(first_phase) => vec![
                        ItemUpdate::SetPhase(first_phase.name.clone()),
                        ItemUpdate::SetPhasePool(PhasePool::Main),
                    ],
                    None => vec![ItemUpdate::SetBlocked(
                        "Pipeline has no main executor phase".to_string(),
                    )],
                }
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
                vec![ItemUpdate::TransitionStatus(Status::Done)]
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
