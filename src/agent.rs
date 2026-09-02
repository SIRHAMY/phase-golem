use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use nix::unistd::Pid;
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::config::{CliTool, PublicExecutionPolicy, TrustedExecutorProfile};
use crate::pg_item::PgItem;
use crate::types::PhaseResult;
use crate::{log_debug, log_warn};

pub const MAX_TRUSTED_RESULT_PAYLOAD_BYTES: usize = 16 * 1024;

/// Maximum time to wait for graceful shutdown after SIGTERM before sending SIGKILL.
const SIGTERM_GRACE_PERIOD_SECONDS: u64 = 5;

/// Polling interval when waiting for a process group to exit after SIGTERM.
const KILL_POLL_INTERVAL_MS: u64 = 100;

/// Global shutdown flag shared with signal handlers.
fn shutdown_flag() -> &'static Arc<AtomicBool> {
    static FLAG: OnceLock<Arc<AtomicBool>> = OnceLock::new();
    FLAG.get_or_init(|| Arc::new(AtomicBool::new(false)))
}

/// Check if a shutdown has been requested via signal.
pub fn is_shutdown_requested() -> bool {
    shutdown_flag().load(Ordering::Relaxed)
}

/// Install signal handlers for SIGTERM and SIGINT that set the shutdown flag.
///
/// Call once at program startup. Subsequent calls are safe (re-registers handlers).
pub fn install_signal_handlers() -> Result<(), String> {
    let flag = Arc::clone(shutdown_flag());
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&flag))
        .map_err(|e| format!("Failed to register SIGTERM handler: {}", e))?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, flag)
        .map_err(|e| format!("Failed to register SIGINT handler: {}", e))?;
    Ok(())
}

// --- Process Registry ---

/// Global registry of active child process group IDs.
///
/// Uses `std::sync::Mutex` (not tokio's) because operations are fast
/// (insert/remove/iterate) with no I/O under the lock.
fn process_registry() -> &'static Arc<std::sync::Mutex<HashSet<Pid>>> {
    static REGISTRY: OnceLock<Arc<std::sync::Mutex<HashSet<Pid>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Arc::new(std::sync::Mutex::new(HashSet::new())))
}

/// Register a child process group in the global registry.
pub fn register_child(pgid: Pid) {
    if let Ok(mut registry) = process_registry().lock() {
        registry.insert(pgid);
    }
}

/// Unregister a child process group from the global registry.
pub fn unregister_child(pgid: Pid) {
    if let Ok(mut registry) = process_registry().lock() {
        registry.remove(&pgid);
    }
}

/// Kill all registered child process groups.
///
/// Sends SIGTERM to all registered PGIDs, waits for the grace period,
/// then SIGKILLs any survivors. Clears the registry when done.
pub fn kill_all_children() {
    use nix::sys::signal::{killpg, Signal};

    let pgids: Vec<Pid> = {
        let Ok(registry) = process_registry().lock() else {
            return;
        };
        registry.iter().copied().collect()
    };

    if pgids.is_empty() {
        return;
    }

    // SIGTERM all
    for &pgid in &pgids {
        let _ = killpg(pgid, Signal::SIGTERM);
    }

    // Wait grace period
    let deadline = std::time::Instant::now() + Duration::from_secs(SIGTERM_GRACE_PERIOD_SECONDS);
    let poll_interval = Duration::from_millis(KILL_POLL_INTERVAL_MS);

    while std::time::Instant::now() < deadline {
        let all_gone = pgids
            .iter()
            .all(|&pgid| matches!(killpg(pgid, None), Err(nix::errno::Errno::ESRCH)));
        if all_gone {
            break;
        }
        std::thread::sleep(poll_interval);
    }

    // SIGKILL survivors
    for &pgid in &pgids {
        let _ = killpg(pgid, Signal::SIGKILL);
    }

    // Clear registry
    if let Ok(mut registry) = process_registry().lock() {
        registry.clear();
    }
}

/// Trait for running agents. Enables mocking in pipeline tests.
pub trait AgentRunner: Send + Sync {
    fn run_agent(
        &self,
        prompt: &str,
        result_path: &Path,
        timeout: Duration,
    ) -> impl std::future::Future<Output = Result<PhaseResult, String>> + Send;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TrustedExecutionRequest {
    pub item_id: String,
    pub phase: String,
    pub title: String,
    pub attempt: u32,
    pub policy: PublicExecutionPolicy,
}

pub trait TrustedExecutorAdapter: Send + Sync {
    fn invoke(
        &self,
        profile: &TrustedExecutorProfile,
        request: &TrustedExecutionRequest,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
}

pub struct LocalProcessExecutorAdapter {
    root: PathBuf,
}

impl LocalProcessExecutorAdapter {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl TrustedExecutorAdapter for LocalProcessExecutorAdapter {
    async fn invoke(
        &self,
        profile: &TrustedExecutorProfile,
        request: &TrustedExecutionRequest,
    ) -> Result<String, String> {
        let result_path = self.root.join(".phase-golem").join(format!(
            "trusted_result_{}_{}.json",
            request.item_id, request.phase
        ));
        let request_json = serde_json::to_string_pretty(request)
            .map_err(|error| format!("Failed to serialize executor request: {error}"))?;
        let prompt = format!(
            "Execute this trusted Phase Golem request:\n{request_json}\n\nWrite one JSON result to {} with exactly: item_id, phase, result (complete or blocked), summary, and evidence_references.",
            result_path.display()
        );
        let timeout = Duration::from_secs(request.policy.timeout_minutes as u64 * 60);
        let mut command = tokio::process::Command::new(&profile.command);
        command.args(&profile.args).arg(prompt);
        command.current_dir(&self.root);
        command.env_clear();
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        command.envs(&profile.environment);

        run_subprocess_json_with_result_payload_limit::<serde_json::Value>(
            command,
            &result_path,
            timeout,
            MAX_TRUSTED_RESULT_PAYLOAD_BYTES,
        )
        .await
        .map(|value| value.to_string())
    }
}

pub async fn run_agent_for_claimed_pg_item(
    item: &PgItem,
    runner: &impl AgentRunner,
    prompt: &str,
    result_path: &Path,
    timeout: Duration,
) -> Option<Result<PhaseResult, String>> {
    if !item.is_claimed_for_pg_execution() {
        return None;
    }

    Some(runner.run_agent(prompt, result_path, timeout).await)
}

/// Real implementation that spawns a CLI agent as a subprocess.
pub struct CliAgentRunner {
    pub tool: CliTool,
    pub model: Option<String>,
}

impl CliAgentRunner {
    pub fn new(tool: CliTool, model: Option<String>) -> Self {
        Self { tool, model }
    }

    /// Verify that the configured CLI tool is available on PATH.
    pub fn verify_cli_available(&self) -> Result<(), String> {
        let output = std::process::Command::new(self.tool.binary_name())
            .args(self.tool.version_args())
            .output()
            .map_err(|e| {
                format!(
                    "{} not found on PATH. {} ({})",
                    self.tool.display_name(),
                    self.tool.install_hint(),
                    e
                )
            })?;

        if !output.status.success() {
            return Err(format!(
                "{} found but `{} {}` failed",
                self.tool.display_name(),
                self.tool.binary_name(),
                self.tool.version_args().join(" ")
            ));
        }

        Ok(())
    }
}

impl AgentRunner for CliAgentRunner {
    async fn run_agent(
        &self,
        prompt: &str,
        result_path: &Path,
        timeout: Duration,
    ) -> Result<PhaseResult, String> {
        let mut cmd = tokio::process::Command::new(self.tool.binary_name());
        cmd.args(self.tool.build_args(prompt, self.model.as_deref()));
        run_subprocess_agent(cmd, result_path, timeout).await
    }
}

/// Spawn a subprocess agent, enforce timeout, read result file.
///
/// This is the shared implementation used by both `CliAgentRunner` and test runners.
/// The caller configures the `Command` (program, args, env); this function handles
/// process group isolation, timeout, signal checking, and result parsing.
///
/// Note: checks the global `shutdown_flag()` after subprocess completion.
pub async fn run_subprocess_agent(
    cmd: tokio::process::Command,
    result_path: &Path,
    timeout: Duration,
) -> Result<PhaseResult, String> {
    run_subprocess_json(cmd, result_path, timeout).await
}

pub async fn run_subprocess_json<T: DeserializeOwned>(
    cmd: tokio::process::Command,
    result_path: &Path,
    timeout: Duration,
) -> Result<T, String> {
    run_subprocess_json_with_result_payload_limit(cmd, result_path, timeout, usize::MAX).await
}

/// Run a shell verification command in a hermetic project-root process group.
pub async fn run_shell_command(
    command_text: &str,
    root: &Path,
    timeout: Duration,
) -> Result<std::process::Output, String> {
    let mut command = tokio::process::Command::new("sh");
    command.args(["-c", command_text]).current_dir(root);
    command.stdin(std::process::Stdio::null());
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    command.kill_on_drop(true);

    // SAFETY: setpgid is async-signal-safe and runs between fork and exec.
    unsafe {
        command.pre_exec(|| {
            nix::unistd::setpgid(nix::unistd::Pid::from_raw(0), nix::unistd::Pid::from_raw(0))
                .map_err(std::io::Error::other)?;
            Ok(())
        });
    }

    let child = command
        .spawn()
        .map_err(|error| format!("Failed to spawn shell verification command: {error}"))?;
    let child_pid = child
        .id()
        .ok_or_else(|| "Failed to get shell verification process PID".to_string())?
        as i32;
    let pgid = Pid::from_raw(child_pid);
    register_child(pgid);

    let wait_for_output = child.wait_with_output();
    tokio::pin!(wait_for_output);
    let timeout_sleep = tokio::time::sleep(timeout);
    tokio::pin!(timeout_sleep);
    let shutdown = wait_for_shutdown();
    tokio::pin!(shutdown);

    let result = tokio::select! {
        result = &mut wait_for_output => result
            .map_err(|error| format!("Error waiting for shell verification command: {error}")),
        _ = &mut timeout_sleep => Err(format!(
            "Shell verification command timed out after {} seconds",
            timeout.as_secs()
        )),
        _ = &mut shutdown => Err("Shutdown requested".to_string()),
    };

    match result {
        Ok(output) => {
            if is_shutdown_requested() {
                kill_process_group(child_pid).await;
                unregister_child(pgid);
                return Err("Shutdown requested".to_string());
            }
            unregister_child(pgid);
            Ok(output)
        }
        Err(error) if error == "Shutdown requested" || error.contains("timed out") => {
            kill_process_group(child_pid).await;
            unregister_child(pgid);
            Err(error)
        }
        Err(error) => {
            unregister_child(pgid);
            Err(error)
        }
    }
}

async fn wait_for_shutdown() {
    while !is_shutdown_requested() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn run_subprocess_json_with_result_payload_limit<T: DeserializeOwned>(
    mut cmd: tokio::process::Command,
    result_path: &Path,
    timeout: Duration,
    max_result_payload_bytes: usize,
) -> Result<T, String> {
    // Delete stale result file if it exists (unconditional to avoid TOCTOU)
    match tokio::fs::remove_file(result_path).await {
        Ok(()) => log_warn!(
            "Warning: Stale result file found at {}, deleted",
            result_path.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // expected
        Err(e) => {
            return Err(format!(
                "Failed to remove stale result file {}: {}",
                result_path.display(),
                e
            ))
        }
    }

    // Configure stdio and process group
    // stdin MUST be null — with setpgid the child is in a background process group,
    // and any attempt to read from the terminal would cause SIGTTIN (silent stop).
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());
    cmd.kill_on_drop(true);

    // SAFETY: pre_exec runs between fork() and exec() where only async-signal-safe
    // functions are permitted. setpgid is async-signal-safe per POSIX.
    unsafe {
        cmd.pre_exec(|| {
            nix::unistd::setpgid(nix::unistd::Pid::from_raw(0), nix::unistd::Pid::from_raw(0))
                .map_err(std::io::Error::other)?;
            Ok(())
        });
    }

    log_debug!("[agent] Spawning subprocess...");
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn subprocess: {}", e))?;

    let child_pid = child
        .id()
        .ok_or_else(|| "Failed to get child PID".to_string())? as i32;
    let pgid = Pid::from_raw(child_pid);
    log_debug!("[agent] Subprocess spawned (pid={})", child_pid);

    // Register in process registry
    register_child(pgid);

    // Wait with timeout
    log_debug!("[agent] Waiting (timeout={}s)...", timeout.as_secs());
    let wait_result = tokio::time::timeout(timeout, child.wait()).await;

    match wait_result {
        Err(_) => {
            // Timeout — kill the process group
            log_debug!(
                "[agent] TIMEOUT after {}s — killing process group",
                timeout.as_secs()
            );
            kill_process_group(child_pid).await;
            let _ = child.wait().await;
            unregister_child(pgid);
            Err(format!(
                "Agent timed out after {} seconds",
                timeout.as_secs()
            ))
        }
        Ok(wait_result) => {
            let exit_status =
                wait_result.map_err(|e| format!("Error waiting for subprocess: {}", e))?;
            log_debug!(
                "[agent] Subprocess exited (status={:?})",
                exit_status.code()
            );

            // Check for shutdown signal
            if is_shutdown_requested() {
                kill_process_group(child_pid).await;
                let _ = child.wait().await;
                unregister_child(pgid);
                return Err("Shutdown requested".to_string());
            }

            unregister_child(pgid);

            let parsed_result =
                read_json_result_file_with_payload_limit(result_path, max_result_payload_bytes)
                    .await;

            match (exit_status.success(), parsed_result) {
                (true, Ok(result)) => {
                    cleanup_result_file(result_path).await;
                    Ok(result)
                }
                (false, Ok(result)) => {
                    log_warn!(
                        "Warning: Agent exited with non-zero status but produced valid result"
                    );
                    cleanup_result_file(result_path).await;
                    Ok(result)
                }
                (_, Err(e)) => {
                    let exit_info = if exit_status.success() {
                        "zero exit".to_string()
                    } else {
                        format!("exit code {:?}", exit_status.code())
                    };
                    Err(format!("Agent failed ({}): {}", exit_info, e))
                }
            }
        }
    }
}

/// Kill a process group by PID. Sends SIGTERM, polls for exit, then SIGKILL if needed.
///
/// The blocking poll-and-sleep loop runs on the tokio blocking thread pool
/// via `spawn_blocking` to avoid stalling async worker threads.
async fn kill_process_group(pgid: i32) {
    tokio::task::spawn_blocking(move || {
        use nix::sys::signal::{killpg, Signal};

        let pgid = Pid::from_raw(pgid);

        // SIGTERM first
        if let Err(nix::errno::Errno::ESRCH) = killpg(pgid, Signal::SIGTERM) {
            return; // already gone
        }

        // Poll for process group exit with short intervals
        let deadline =
            std::time::Instant::now() + Duration::from_secs(SIGTERM_GRACE_PERIOD_SECONDS);
        let poll_interval = Duration::from_millis(KILL_POLL_INTERVAL_MS);

        while std::time::Instant::now() < deadline {
            // Signal 0 checks if the process group exists without sending a signal
            match killpg(pgid, None) {
                Err(nix::errno::Errno::ESRCH) => return, // process group exited
                _ => std::thread::sleep(poll_interval),
            }
        }

        // Still alive after grace period — force kill
        let _ = killpg(pgid, Signal::SIGKILL);
    })
    .await
    .unwrap_or_else(|e| log_warn!("kill_process_group task panicked: {}", e));
}

/// Read and validate a phase result JSON file.
pub async fn read_result_file(path: &Path) -> Result<PhaseResult, String> {
    read_json_result_file(path).await
}

pub async fn read_trusted_executor_result_file(path: &Path) -> Result<String, String> {
    read_json_result_file_with_payload_limit::<serde_json::Value>(
        path,
        MAX_TRUSTED_RESULT_PAYLOAD_BYTES,
    )
    .await
    .map(|value| value.to_string())
}

async fn read_json_result_file<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    read_json_result_file_with_payload_limit(path, usize::MAX).await
}

async fn read_json_result_file_with_payload_limit<T: DeserializeOwned>(
    path: &Path,
    max_payload_bytes: usize,
) -> Result<T, String> {
    use tokio::io::AsyncReadExt;

    let file = tokio::fs::File::open(path).await.map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!("Result file not found: {}", path.display())
        } else {
            format!("Failed to read result file {}: {}", path.display(), error)
        }
    })?;
    let mut contents = Vec::with_capacity(max_payload_bytes.min(16 * 1024));
    file.take(max_payload_bytes.saturating_add(1) as u64)
        .read_to_end(&mut contents)
        .await
        .map_err(|error| format!("Failed to read result file {}: {}", path.display(), error))?;
    if contents.len() > max_payload_bytes {
        return Err(format!(
            "Result file {} exceeds the {}-byte protocol limit",
            path.display(),
            max_payload_bytes
        ));
    }
    let contents = String::from_utf8(contents)
        .map_err(|error| format!("Failed to read result file {}: {}", path.display(), error))?;

    let result = serde_json::from_str(&contents).map_err(|error| {
        format!(
            "Failed to parse result JSON from {}: {}",
            path.display(),
            error
        )
    })?;

    Ok(result)
}

/// Delete a result file after successful read.
async fn cleanup_result_file(path: &Path) {
    if let Err(e) = tokio::fs::remove_file(path).await {
        log_warn!(
            "Warning: Failed to clean up result file {}: {}",
            path.display(),
            e
        );
    }
}

/// Mock agent runner for pipeline tests.
///
/// Returns predefined PhaseResult values from a configurable sequence.
/// Each call to `run_agent` returns the next result in the sequence.
pub struct MockAgentRunner {
    results: tokio::sync::Mutex<Vec<Result<PhaseResult, String>>>,
}

impl MockAgentRunner {
    /// Create a new mock with a sequence of results to return.
    ///
    /// Results are returned in order (first call gets first result, etc.).
    pub fn new(results: Vec<Result<PhaseResult, String>>) -> Self {
        let mut reversed = results;
        reversed.reverse();
        Self {
            results: tokio::sync::Mutex::new(reversed),
        }
    }
}

impl AgentRunner for MockAgentRunner {
    async fn run_agent(
        &self,
        _prompt: &str,
        _result_path: &Path,
        _timeout: Duration,
    ) -> Result<PhaseResult, String> {
        let mut results = self.results.lock().await;
        results
            .pop()
            .unwrap_or_else(|| Err("MockAgentRunner: no more results in sequence".to_string()))
    }
}

/// Set the shutdown flag for testing. Only available in test builds.
// Relaxed is safe: .await on subprocess wait() ensures visibility before flag check
#[cfg(test)]
fn set_shutdown_flag_for_testing(value: bool) {
    shutdown_flag().store(value, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::Duration;
    use tempfile::TempDir;

    #[tokio::test]
    async fn shutdown_flag_returns_error_after_subprocess_exits() {
        let dir = TempDir::new().unwrap();
        let result_path = dir.path().join("result.json");

        set_shutdown_flag_for_testing(true);

        let fixture_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_agent_success.sh");
        let mut cmd = tokio::process::Command::new("bash");
        cmd.arg(&fixture_path).arg(&result_path);

        let result = run_subprocess_agent(cmd, &result_path, Duration::from_secs(30)).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("Shutdown requested"),
            "Expected 'Shutdown requested' in: {}",
            err
        );

        set_shutdown_flag_for_testing(false);
    }
}
