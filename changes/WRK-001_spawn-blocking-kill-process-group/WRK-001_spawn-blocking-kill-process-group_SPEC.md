# SPEC: Wrap kill_process_group in spawn_blocking

**ID:** WRK-001
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-001_spawn-blocking-kill-process-group_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

The `kill_process_group` function in `agent.rs` uses `std::thread::sleep()` in a polling loop that blocks a tokio worker thread for up to 5 seconds. Both call sites are in the async function `run_subprocess_agent`. This violates the async contract and can interfere with signal checking, timeout tracking, and future concurrent execution. The fix is mechanical: wrap the blocking body in `tokio::task::spawn_blocking` following the established pattern from `coordinator.rs`.

## Approach

Make `kill_process_group` an `async fn` that wraps its entire synchronous body inside `tokio::task::spawn_blocking`. The two call sites in `run_subprocess_agent` add `.await`. The blocking poll-and-sleep logic (SIGTERM → poll every 100ms → SIGKILL after 5s) is moved verbatim into the closure — no logic changes.

Panic handling uses `.unwrap_or_else(|e| log_warn!(...))`, adapted from the coordinator.rs pattern. The coordinator uses `.unwrap_or_else(|e| Err(...))` because those calls return `Result`; here the function returns `()`, so logging is the appropriate equivalent.

**Patterns to follow:**

- `.claude/skills/changes/orchestrator/src/coordinator.rs:556-572` — `spawn_blocking` with `.unwrap_or_else` panic handling for blocking operations

**Implementation boundaries:**

- Do not modify: `kill_all_children` (must remain synchronous — called during shutdown when tokio runtime may be unavailable)
- Do not refactor: The polling logic itself (no `tokio::time::sleep` rewrite, no timing changes)
- Do not change: Constants (`SIGTERM_GRACE_PERIOD_SECONDS`, `KILL_POLL_INTERVAL_MS`)
- Do not add: New tests beyond verifying existing tests pass

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Wrap kill_process_group in spawn_blocking | Low | Make `kill_process_group` async, wrap body in `spawn_blocking`, add `.await` at both call sites |

**Ordering rationale:** Single phase — all changes are in one file and must be applied atomically for the code to compile.

---

## Phases

### Phase 1: Wrap kill_process_group in spawn_blocking

> Make `kill_process_group` async, wrap body in `spawn_blocking`, add `.await` at both call sites

**Phase Status:** complete

**Complexity:** Low

**Goal:** Move the blocking poll-and-sleep loop in `kill_process_group` off tokio worker threads onto the blocking thread pool, preserving exact SIGTERM → poll → SIGKILL semantics.

**Files:**

- `.claude/skills/changes/orchestrator/src/agent.rs` — modify — Change `kill_process_group` signature to `async fn`, wrap body in `spawn_blocking` closure, add `.await` at two call sites (lines ~237 and ~254)

**Patterns:**

- Follow `coordinator.rs:556-572` for `spawn_blocking` + `.unwrap_or_else` pattern. Note: coordinator returns `Result` so uses `Err(format!(...))` in the unwrap handler; here the function returns `()` so use `log_warn!()` instead.

**Tasks:**

- [x] Change `kill_process_group` signature from `fn kill_process_group(pgid: i32)` to `async fn kill_process_group(pgid: i32)`
- [x] Wrap entire function body in `tokio::task::spawn_blocking(move || { ... })` closure
- [x] Add `.await.unwrap_or_else(|e| log_warn!("kill_process_group task panicked: {}", e))` after the `spawn_blocking` call
- [x] Update timeout-path call site (~line 237): `kill_process_group(child_pid)` → `kill_process_group(child_pid).await`
- [x] Update shutdown-path call site (~line 254): `kill_process_group(child_pid)` → `kill_process_group(child_pid).await`
- [x] Verify `use nix::sys::signal::{killpg, Signal}` import inside the function body works correctly inside the closure (it should — it's a local `use` statement)

**Verification:**

- [x] `cargo clippy --all-targets` produces no new warnings
- [x] `cargo test` — all existing tests pass without modification
- [x] `subprocess_timeout_kills_process` test passes (exercises timeout → kill path)
- [x] `process_group_kill_cleans_up_subprocess` test passes (exercises process group cleanup)
- [x] No `std::thread::sleep` calls remain outside of `spawn_blocking` closures in `kill_process_group`
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-001][P1] Fix: Wrap kill_process_group sleep in spawn_blocking`

**Notes:**

- `pgid: i32` is `Copy`, so it moves into the `spawn_blocking` closure trivially — no cloning needed.
- The `Pid::from_raw(pgid)` call and the `use nix::sys::signal::{killpg, Signal}` import both work inside closures.
- If the blocking task panics (extremely unlikely — simple signal/sleep operations), `.unwrap_or_else` converts it to `()` and execution continues normally. At the timeout-path call site, `unregister_child(pgid)` runs on the next line. At the shutdown-path call site, `child.wait().await` and the error return follow. In both cases, cleanup proceeds. The process group also remains in the registry, so `kill_all_children` at shutdown provides an additional safety net.

**Followups:**

- Wrap blocking filesystem ops (`fs::remove_file`, `fs::read_to_string`) in `run_subprocess_agent` with `tokio::fs` or `spawn_blocking`
- Consider wrapping `kill_all_children` in `spawn_blocking` when called from async contexts (currently excluded per SPEC boundary)
- Add test coverage for the shutdown signal → `kill_process_group` path (`is_shutdown_requested()` branch)

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met:
  - [x] `kill_process_group` body runs inside `tokio::task::spawn_blocking`
  - [x] The calling async code awaits the `spawn_blocking` result
  - [x] No `std::thread::sleep` calls execute on tokio worker threads
  - [x] SIGTERM → poll → SIGKILL behavior is preserved exactly
  - [x] Existing tests pass without modification
  - [x] `cargo clippy` produces no new warnings
  - [x] Follows existing `spawn_blocking` pattern from `coordinator.rs`
- [x] Tests pass
- [x] No regressions introduced
- [x] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | complete | 7ada2c3 | All tasks done, all verification passed, code review passed |

## Followups Summary

### Critical

### High

### Medium

- Wrap blocking filesystem ops in `run_subprocess_agent` (`fs::remove_file`, `fs::read_to_string`, `fs::remove_file`) with async equivalents to avoid blocking tokio workers
- Consider wrapping `kill_all_children` in `spawn_blocking` when called from async contexts
- Add test coverage for the shutdown signal → `kill_process_group` path

### Low

## Design Details

### Key Types

No new types introduced. The only type change is the function signature:

```rust
// Before
fn kill_process_group(pgid: i32)

// After
async fn kill_process_group(pgid: i32)
```

### Architecture Details

```
Before:
  run_subprocess_agent (async) ──sync call──> kill_process_group (sync, blocks worker thread)

After:
  run_subprocess_agent (async) ──.await──> kill_process_group (async, delegates to blocking pool)
                                                └──spawn_blocking──> closure (sync body, runs on blocking thread)
```

### Design Rationale

- **spawn_blocking over tokio::time::sleep:** Keeps the change minimal (body moves verbatim into closure), matches existing codebase pattern from coordinator.rs, preserves exact polling semantics.
- **async fn over wrapping at call sites:** Encapsulates the async concern in the function itself, avoids duplicating wrapping + panic-handling at two call sites.
- **log_warn! over unwrap/panic:** Both call sites are already on error/cleanup paths — crashing the caller is worse than logging and continuing. The process remains registered so `kill_all_children` at shutdown provides a safety net.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
