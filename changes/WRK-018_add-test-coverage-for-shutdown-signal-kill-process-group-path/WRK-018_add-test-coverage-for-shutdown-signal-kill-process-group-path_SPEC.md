# SPEC: Add test coverage for shutdown signal kill_process_group path

**ID:** WRK-018
**Status:** Ready
**Created:** 2026-02-20
**PRD:** ./WRK-018_add-test-coverage-for-shutdown-signal-kill-process-group-path_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

The `run_subprocess_agent` function in `src/agent.rs` has a shutdown signal code path (lines 265-270) that checks `is_shutdown_requested()` after a subprocess exits normally. If true, it calls `kill_process_group(child_pid)` and returns `Err("Shutdown requested")`. This path has no test coverage. The timeout path (line 247) that also calls `kill_process_group` is covered by existing integration tests in `tests/agent_test.rs`.

This task adds a `#[cfg(test)]` setter function and a unit test inside `src/agent.rs` to exercise the shutdown signal path. The test lives in `src/agent.rs` (not `tests/agent_test.rs`) because `#[cfg(test)]` functions are invisible to integration test crates — this is a scope divergence from the PRD, documented in the Design's Technical Decisions section.

## Approach

Add two `#[cfg(test)]`-gated items to the end of `src/agent.rs`:

1. **Setter function** `set_shutdown_flag_for_testing(value: bool)` — stores `value` to the global `AtomicBool` via `shutdown_flag().store(value, Ordering::Relaxed)`. Placed immediately before the test module.

2. **Test module** `#[cfg(test)] mod tests {}` containing one async test that sets the shutdown flag to `true`, spawns a subprocess via `run_subprocess_agent` with `mock_agent_success.sh`, and asserts the function returns `Err` containing `"Shutdown requested"`. The test resets the flag to `false` after assertions.

Zero production code changes. The setter and test module compile only during `cargo test`.

**Patterns to follow:**

- `src/coordinator.rs:735` — `#[cfg(test)] mod tests {}` block structure and `use super::*;` pattern
- `src/scheduler.rs:1825` — another `#[cfg(test)] mod tests {}` example
- `tests/agent_test.rs:103-124` — `subprocess_success_writes_valid_result` for `TempDir` + `Command` + `run_subprocess_agent` pattern

**Implementation boundaries:**

- Do not modify: production code in `src/agent.rs` (lines 1-395)
- Do not modify: `tests/agent_test.rs` (existing integration tests)
- Do not modify: `Cargo.toml` (all dependencies already present)
- Do not refactor: the shutdown flag from global state to dependency injection

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Add shutdown flag test | Low | Add `#[cfg(test)]` setter and unit test for shutdown signal path |

**Ordering rationale:** Single phase — the setter and test are a single logical unit with no intermediate verification boundary.

---

## Phases

### Phase 1: Add shutdown flag test

> Add `#[cfg(test)]` setter function and unit test for the shutdown signal path in `src/agent.rs`

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Exercise the shutdown signal code path (lines 265-270) with a unit test that verifies `run_subprocess_agent` returns `Err("Shutdown requested")` when the shutdown flag is set.

**Files:**

- `src/agent.rs` — modify — Add `#[cfg(test)]` setter function and `#[cfg(test)] mod tests {}` block with one async test

**Patterns:**

- Follow `src/coordinator.rs:735-740` for `#[cfg(test)] mod tests { use super::*; }` structure
- Follow `tests/agent_test.rs:103-124` for `TempDir` + `Command` + `run_subprocess_agent` + assertion pattern

**Tasks:**

- [ ] Add `#[cfg(test)] fn set_shutdown_flag_for_testing(value: bool)` at the end of `src/agent.rs`, before the test module. Implementation: `shutdown_flag().store(value, Ordering::Relaxed)`. Include inline comment: `// Relaxed is safe: .await on subprocess wait() ensures visibility before flag check`
- [ ] Add `#[cfg(test)] mod tests {}` block at the end of `src/agent.rs` with `use super::*;` and necessary imports (`tempfile::TempDir`, `std::path::Path`, `std::time::Duration`)
- [ ] Implement `#[tokio::test] async fn shutdown_flag_returns_error_after_subprocess_exits()`:
  - Create `TempDir`, build result path as `dir.path().join("result.json")`
  - Call `set_shutdown_flag_for_testing(true)`
  - Build `tokio::process::Command` for `bash` with `.arg(fixture_path).arg(&result_path)` where fixture path is `Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_agent_success.sh")`
  - Call `run_subprocess_agent(cmd, &result_path, Duration::from_secs(30)).await`
  - Assert using existing pattern: `assert!(result.is_err()); let err = result.unwrap_err(); assert!(err.contains("Shutdown requested"), "Expected 'Shutdown requested' in: {}", err);`
  - Call `set_shutdown_flag_for_testing(false)` for cleanup

**Verification:**

- [ ] `cargo test -p phase_golem tests::shutdown_flag_returns_error_after_subprocess_exits` passes
- [ ] `cargo test` (full suite) passes with no regressions
- [ ] `cargo clippy` produces no new warnings

**Commit:** `[WRK-018][P1] Feature: Add unit test for shutdown signal kill_process_group path`

**Notes:**

- The test sets the shutdown flag *before* spawning the subprocess. This is deterministic — the flag is already `true` when the shutdown check at line 266 executes after the subprocess exits.
- `kill_process_group` receives ESRCH because the process already exited — this is expected and handled gracefully by existing code (line 311-312).
- Explicit flag reset is not panic-safe. If the test panics before the reset line, the flag leaks to subsequent tests. This is an accepted risk given the test is straightforward. Escalation path: add `serial_test` as a dev-dependency and annotate with `#[serial]` if flaky behavior is observed.
- `Ordering::Relaxed` is correct because the `.await` on subprocess completion establishes a happens-before relationship, ensuring the flag value is visible when checked at line 266.

**Followups:**

---

## Final Verification

- [ ] All phases complete
- [ ] All PRD success criteria met
- [ ] Tests pass
- [ ] No regressions introduced
- [ ] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|

## Followups Summary

### Critical

### High

### Medium

### Low

- [ ] [Low] Consider adding a test variant that spawns a subprocess with child processes to verify `kill_process_group` terminates the entire process group during shutdown (PRD Nice-to-Have). Deferred because `kill_process_group` behavior is already tested via the timeout path tests in `tests/agent_test.rs:327-354`.

## Design Details

### Key Types

No new types introduced. The implementation uses existing types:

- `AtomicBool` / `Arc` / `Ordering` — from `std::sync::atomic` (already imported in `agent.rs`)
- `TempDir` — from `tempfile` (already a dev-dependency)

### Architecture Details

No architectural changes. The setter function and test module are `#[cfg(test)]`-gated additions that compile only during testing.

Data flow:
1. Test calls `set_shutdown_flag_for_testing(true)` → stores to global `AtomicBool`
2. Test calls `run_subprocess_agent(cmd, result_path, timeout)`
3. Subprocess spawns, writes result JSON, exits with code 0
4. `run_subprocess_agent` checks `is_shutdown_requested()` → returns `true`
5. Calls `kill_process_group(child_pid)` → ESRCH (process already gone)
6. Returns `Err("Shutdown requested".to_string())`
7. Test asserts error message and resets flag to `false`

### Design Rationale

- **Unit test in `src/agent.rs`** over integration test in `tests/agent_test.rs`: `#[cfg(test)]` functions are invisible to integration test crates. The setter needs access to the private `shutdown_flag()` function. This follows existing patterns in `coordinator.rs`, `scheduler.rs`, `log.rs`, and `lock.rs`.
- **Explicit reset** over `#[serial]`: Only one shutdown test exists, so parallel interference is unlikely. Avoids adding a dev-dependency for a single test. Escalation path documented.
- **Pre-set flag** over mid-execution flag: Deterministic — no timing dependency. Tests the flag-checking logic, not signal delivery.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
