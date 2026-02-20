# SPEC: Wrap kill_all_children in spawn_blocking for async call sites

**ID:** WRK-017
**Status:** Complete
**Created:** 2026-02-19
**PRD:** ./WRK-017_wrap-kill-all-children-in-spawn-blocking-for-async-call-sites_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

`kill_all_children()` is a synchronous function in `src/agent.rs` that sends SIGTERM to all registered child process groups, polls with `std::thread::sleep` (up to 5 seconds), then SIGKILLs survivors. It is called directly from two async functions — `handle_run` (main.rs:635) and `handle_triage` (main.rs:851) — blocking a tokio worker thread for the duration.

The codebase already has an established pattern for this: `kill_process_group()` (agent.rs:304-333) wraps the same SIGTERM-poll-SIGKILL logic in `tokio::task::spawn_blocking` with `.await.unwrap_or_else(|e| log_warn!(...))` error handling. The fix is to apply this same pattern at both `kill_all_children()` call sites.

## Approach

Replace each direct `kill_all_children()` call in async context with a `spawn_blocking` wrapper that moves the blocking work to the tokio blocking thread pool. The wrapper uses `move || { ... }` closure syntax and `.await.unwrap_or_else(|e| log_warn!(...))` for panic handling, matching the established `kill_process_group()` pattern exactly.

The function signature and body of `kill_all_children()` are not modified — it must remain synchronous for shutdown-path safety (WRK-001 constraint).

**Patterns to follow:**

- `src/agent.rs:304-333` — `kill_process_group()`: the gold standard `spawn_blocking` + `unwrap_or_else` + `log_warn!` pattern for fire-and-forget blocking cleanup
- `src/main.rs:649-...` — `handle_run` commit block: additional `spawn_blocking` usage in the same function

**Implementation boundaries:**

- Do not modify: `src/agent.rs` (the `kill_all_children()` function definition)
- Do not add: New test files (PRD explicitly excludes; change is mechanical and covered by existing integration tests)

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Wrap call sites in spawn_blocking | Low | Wrap both async call sites of `kill_all_children()` in `tokio::task::spawn_blocking` with panic-logging error handling |

**Ordering rationale:** Single phase — both call sites are independent, the change is identical at each site, and all dependencies (tokio, log_warn!, kill_all_children import) are already in scope.

---

## Phases

### Phase 1: Wrap call sites in spawn_blocking

> Wrap both async call sites of `kill_all_children()` in `tokio::task::spawn_blocking` with panic-logging error handling

**Phase Status:** complete

**Complexity:** Low

**Goal:** Move the blocking `kill_all_children()` calls off tokio worker threads by wrapping them in `spawn_blocking`, matching the established `kill_process_group()` pattern.

**Files:**

- `src/main.rs` — modify — Wrap `kill_all_children()` calls in `handle_run` and `handle_triage` in `spawn_blocking`

**Patterns:**

- Follow `src/agent.rs:304-333` (`kill_process_group`) for the exact `spawn_blocking` + `.await` + `.unwrap_or_else` + `log_warn!` pattern

**Tasks:**

- [x] Search `src/main.rs` for all `kill_all_children()` calls — confirm exactly 2 exist, both in async functions (`handle_run` and `handle_triage`)
- [x] In `handle_run`, replace the `kill_all_children();` call with:
  ```rust
  tokio::task::spawn_blocking(move || {
      kill_all_children();
  })
  .await
  .unwrap_or_else(|e| log_warn!("kill_all_children task panicked: {}", e));
  ```
- [x] In `handle_triage`, replace the `kill_all_children();` call with the same `spawn_blocking` wrapper (identical code)
- [x] Verify `tokio::task::spawn_blocking` is already accessible (it is — used elsewhere in `handle_run`)
- [x] Verify `log_warn!` macro is already imported (it is — imported at the top of main.rs)

**Verification:**

- [x] `cargo build` succeeds without warnings
- [x] `cargo test` passes (all existing tests pass without modification)
- [x] `cargo clippy` reports no new warnings
- [x] Both call sites use `move || { kill_all_children(); }` closure syntax, matching `kill_process_group()` at agent.rs:305
- [x] Both call sites use `.unwrap_or_else(|e| log_warn!("kill_all_children task panicked: {}", e))` — exact log message matches this pattern
- [x] No other `kill_all_children()` calls exist in async contexts without `spawn_blocking`
- [x] `kill_all_children()` function signature and body in `src/agent.rs` are unchanged

**Commit:** `[WRK-017][P1] Fix: Wrap kill_all_children in spawn_blocking at async call sites`

**Notes:**

- The `move` keyword on the closure is technically unnecessary (nothing is captured) but is included for consistency with the `kill_process_group()` pattern at agent.rs:305.
- This changes panic semantics at both call sites: previously a panic in `kill_all_children()` would crash the process; now it is caught by the `JoinHandle` and logged. This is a behavioral improvement for cleanup paths. If `kill_all_children()` panics mid-execution (e.g., during registry clearing), the registry may remain uncleared — this is acceptable for a cleanup path where the process is about to exit.
- Task descriptions use function names (`handle_run`, `handle_triage`) rather than line numbers since line numbers may shift if other changes land first.

**Followups:**

- [ ] [Low] Audit other blocking operations in async functions for consistent `spawn_blocking` usage — tracked separately as WRK-019 but worth verifying scope alignment

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met:
  - [x] `handle_run` calls `kill_all_children()` via `spawn_blocking` and `.await`s the result
  - [x] `handle_triage` calls `kill_all_children()` via `spawn_blocking` and `.await`s the result
  - [x] Error handling uses `.await.unwrap_or_else(|e| ...)` with `log_warn!`, matching `kill_process_group()` pattern
  - [x] Errors from JoinHandle are logged but not propagated
  - [x] `kill_all_children()` function signature and body are not modified
  - [x] Existing tests pass without modification
- [x] Tests pass
- [x] No regressions introduced
- [x] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | complete | `[WRK-017][P1] Fix: Wrap kill_all_children in spawn_blocking at async call sites` | Both call sites wrapped, all verification passed, code review clean |

## Followups Summary

### Critical

### High

### Medium

### Low

- Audit other blocking operations in async functions for consistent `spawn_blocking` usage — may overlap with WRK-019 scope

## Design Details

### Design Rationale

The `spawn_blocking` wrapper pattern is the simplest correct approach. It is already established in the codebase (`kill_process_group` at agent.rs:304-333), requires no new dependencies, and preserves the synchronous function signature required for shutdown-path safety.

The alternative (`block_in_place`) was rejected in the design phase: it is not used for this pattern in the codebase, hurts cache locality, and is incompatible with `current_thread` runtimes.

---

## Retrospective

### What worked well?

- Mechanical change with clear precedent made implementation straightforward
- All dependencies (tokio, log_warn!) already in scope — zero friction
- Single-phase SPEC was appropriate for this scope

### What was harder than expected?

- Nothing — this was as straightforward as expected for a two-line mechanical change

### What would we do differently next time?

- Nothing — the light-mode SPEC approach was well-matched to this task's complexity

## Assumptions

- Light mode SPEC was appropriate given single-phase, two-line mechanical change with clear codebase precedent.
- No open questions — the approach is fully specified in the design doc and validated by tech research.
- Task descriptions use function names rather than line numbers to be resilient to codebase changes.
