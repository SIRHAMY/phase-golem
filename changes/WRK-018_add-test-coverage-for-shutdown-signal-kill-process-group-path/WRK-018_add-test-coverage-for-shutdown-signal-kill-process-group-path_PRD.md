# Change: Add test coverage for shutdown signal kill_process_group path

**Status:** Proposed
**Created:** 2026-02-20
**Author:** Claude (autonomous)

## Problem Statement

The `run_subprocess_agent` function in `src/agent.rs` has two code paths that call `kill_process_group`:

1. **Timeout path** (line 247): When a subprocess exceeds its timeout. This path is covered by `subprocess_timeout_kills_process` and `process_group_kill_cleans_up_subprocess` tests.
2. **Shutdown signal path** (lines 265-270): After a subprocess exits normally, the function checks `is_shutdown_requested()`. If true, it calls `kill_process_group(child_pid)` and returns `Err("Shutdown requested")`. **This path has no test coverage.**

The shutdown signal path ensures clean process group cleanup when the orchestrator receives SIGTERM/SIGINT while a subprocess is executing. Without test coverage, regressions could silently cause process leaks during graceful shutdown — preventing clean termination in environments managed by process supervisors (systemd, Kubernetes) where Pod/service rollover depends on the orchestrator exiting cleanly.

This gap was identified during WRK-001 (which wrapped `kill_process_group`'s blocking poll loop in `spawn_blocking`) and logged as a follow-up.

## User Stories / Personas

- **Orchestrator Developer** — Wants confidence that the shutdown signal path correctly kills subprocess process groups and returns the expected error, especially when modifying signal handling or shutdown logic.

## Desired Outcome

When this change is complete, the shutdown signal path in `run_subprocess_agent` (lines 265-270) has automated test coverage that verifies:

1. When `is_shutdown_requested()` returns true after a subprocess exits normally, `run_subprocess_agent` returns `Err` containing `"Shutdown requested"`.
2. The error return confirms the shutdown code path was executed — `kill_process_group` is called on this path before the error is returned, and its behavior is separately tested via the timeout path tests.

A test-only setter function exists (gated behind `#[cfg(test)]`) to control the shutdown flag without sending real OS signals.

## Success Criteria

### Must Have

- [ ] At least one test exercises the shutdown signal path (lines 265-270 of `agent.rs`)
- [ ] Test verifies `run_subprocess_agent` returns `Err(e)` where `e` contains `"Shutdown requested"` (substring match, consistent with existing test patterns)
- [ ] A `#[cfg(test)]` setter function is added to `agent.rs` with signature: `pub fn set_shutdown_flag_for_testing(value: bool)` — stores `value` to the existing `shutdown_flag()` via `Ordering::Relaxed`
- [ ] Each test that sets the shutdown flag explicitly resets it to `false` after assertions (cleanup)
- [ ] All existing tests continue to pass
- [ ] `cargo clippy` produces no new warnings

### Should Have

- [ ] Test uses the existing test patterns (mock shell scripts, `TempDir`, `tokio::test`)
- [ ] Test subprocess uses `mock_agent_success.sh` (exits with code 0, writes valid result JSON) — the subprocess must exit normally so the shutdown check at line 266 is reached

### Nice to Have

- [ ] Additional test variant verifying behavior when subprocess has spawned child processes during shutdown (confirming the entire process group is killed)

## Scope

### In Scope

- Adding a `#[cfg(test)]` public setter function to `agent.rs` for the shutdown flag
- Adding test(s) to `tests/agent_test.rs` that exercise the shutdown signal path
- Adding a mock shell script to `tests/fixtures/` if needed (likely reuses existing `mock_agent_success.sh`)

### Out of Scope

- Refactoring the shutdown flag to use dependency injection
- Modifying `kill_process_group` implementation
- Testing actual SIGTERM/SIGINT signal delivery (this test verifies the flag-checking logic, not signal delivery)
- Testing `kill_all_children` (separate synchronous shutdown function)
- Changes to production code behavior
- Orphaned result file cleanup in the shutdown path (production behavior concern, not a test coverage gap)
- Integration tests or end-to-end scenarios

## Non-Functional Requirements

- **Performance:** Tests should complete in under 2 seconds (subprocess exits immediately; no long timeouts or grace period delays needed)

## Constraints

- The shutdown flag is a static global `OnceLock<Arc<AtomicBool>>` — it can be set/reset via `Ordering::Relaxed` store, but the `Arc<AtomicBool>` itself is initialized once per process. The `#[cfg(test)]` setter works within this constraint.
- Since `cargo test` runs tests in parallel within a single process, tests that mutate the global shutdown flag can interfere with each other. The test must explicitly reset the flag to `false` after assertions. If parallel interference is observed during implementation, the test should be annotated with `#[serial]` from the `serial_test` crate (adding it as a dev-dependency).
- The shutdown check at line 266 happens after `unregister_child(pgid)` at line 263 — the process is already removed from the registry before `kill_process_group` is called. Calling `kill_process_group` on an already-exited process group produces ESRCH, which `kill_process_group` handles gracefully (returns immediately).

## Dependencies

- **Depends On:** Nothing — standalone test addition
- **Blocks:** Nothing directly

## Risks

- [ ] **Test isolation:** The global shutdown flag could leak state between parallel tests if not properly reset. Mitigation: each test explicitly resets the flag to `false` after assertions. Escalation: add `#[serial]` if parallel interference occurs.

## Assumptions

- A `#[cfg(test)]` public setter is the right approach for controlling the shutdown flag in tests. This avoids the complexity of sending real signals and the invasiveness of refactoring to dependency injection, while being an idiomatic Rust pattern for exposing test-only control over private globals.
- The test sets the shutdown flag *before* spawning the subprocess. This exercises the code path deterministically — the flag is already true when the shutdown check at line 266 executes. This tests the defensive behavior of the shutdown path, not the race condition of a signal arriving mid-execution.
- The existing `mock_agent_success.sh` script is suitable because it exits with code 0 and writes a valid result JSON, ensuring the subprocess exits normally and reaches the shutdown check (line 266) rather than taking the timeout path (line 241) or the error path (line 287).

## Open Questions

(None — all resolved during self-critique.)

## References

- `src/agent.rs:265-270` — untested shutdown signal path
- `src/agent.rs:300-333` — `kill_process_group` implementation
- `src/agent.rs:20-28` — shutdown flag and `is_shutdown_requested()`
- `tests/agent_test.rs` — existing agent tests
- `tests/fixtures/mock_agent_success.sh` — mock script for normal subprocess exit
- `changes/WRK-001_spawn-blocking-kill-process-group/` — origin work item
