# SPEC: Wrap blocking filesystem ops in run_subprocess_agent with async equivalents

**ID:** WRK-016
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-016_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

The `run_subprocess_agent` function in `agent.rs` is an `async fn` running on tokio's async runtime, but it makes 3 blocking `std::fs` calls that execute directly on async worker threads. This was identified during WRK-001 build, which fixed the analogous issue in `kill_process_group` using `spawn_blocking`. The 3 remaining filesystem calls are the last blocking operations in the async agent execution path.

This is a mechanical conversion: swap `std::fs` calls with `tokio::fs` equivalents, add `.await`, convert helper function signatures to `async fn`, and update all callers including 4 test functions. No behavioral changes.

## Approach

Replace 3 blocking `std::fs` calls with `tokio::fs` async equivalents:

1. Convert `cleanup_result_file` from `fn` to `async fn`, replacing `fs::remove_file` with `tokio::fs::remove_file(...).await`
2. Convert `read_result_file` from `pub fn` to `pub async fn`, replacing `fs::read_to_string` with `tokio::fs::read_to_string(...).await`
3. In `run_subprocess_agent`, replace the inline `fs::remove_file` with `tokio::fs::remove_file(...).await` and add `.await` to calls to `read_result_file` and `cleanup_result_file`
4. Convert 4 test functions from `#[test] fn` to `#[tokio::test] async fn` and add `.await` to their `read_result_file` calls
5. Remove the now-unused `use std::fs;` import from `agent.rs`

All error handling behavior is preserved identically — `tokio::fs` returns `std::io::Result<T>` with the same `io::Error` and `ErrorKind` values because it delegates to `std::fs` internally via `spawn_blocking`. All `tokio::fs` calls use fully-qualified paths (`tokio::fs::remove_file`, `tokio::fs::read_to_string`) — no new import statement is needed.

**Patterns to follow:**

- `.claude/skills/changes/orchestrator/src/agent.rs` lines 112-165 — existing `#[tokio::test] async fn` tests in the same file demonstrate the pattern for async test conversion
- `.claude/skills/changes/orchestrator/src/coordinator.rs` lines 547+ — established `spawn_blocking` pattern for git ops (we use `tokio::fs` instead because our calls are isolated and interspersed with async logic)

**Implementation boundaries:**

- Do not modify: `executor.rs`, `backlog.rs`, `config.rs`, `main.rs`, `worklog.rs`, `lock.rs`, `git.rs`, `migration.rs`, `preflight.rs` (out of scope — other modules' blocking I/O is tracked separately)
- Do not modify: `fs::write` calls in test fixture setup code in `agent_test.rs` (intentionally left synchronous per design decision)
- Do not modify: `kill_all_children` or `kill_process_group` (already correctly handled)
- Do not modify: `CliAgentRunner::verify_cli_available()` (startup-only, not in async execution path)

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Async filesystem conversion | Low | Convert all 3 `std::fs` calls to `tokio::fs`, update function signatures, update callers, update tests |

**Ordering rationale:** This is a single atomic change — all edits must land together because changing `read_result_file` to `async fn` without updating its callers (and vice versa) produces compile errors. One phase is the natural and correct structure.

---

## Phases

### Phase 1: Async filesystem conversion

> Convert all 3 `std::fs` calls to `tokio::fs` equivalents and update all callers

**Phase Status:** complete

**Complexity:** Low

**Goal:** Eliminate all blocking `std::fs` calls from the async agent execution path in `agent.rs` while preserving identical observable behavior.

**Files:**

- `.claude/skills/changes/orchestrator/src/agent.rs` — modify — convert `read_result_file` and `cleanup_result_file` to `async fn`, replace 3 `std::fs` calls with `tokio::fs`, add `.await` at call sites, remove `use std::fs;` import
- `.claude/skills/changes/orchestrator/tests/agent_test.rs` — modify — convert 4 test functions from `#[test] fn` to `#[tokio::test] async fn`, add `.await` to `read_result_file` calls

**Tasks:**

- [x] Convert `cleanup_result_file` (line 339): change `fn` to `async fn`, replace `fs::remove_file(path)` with `tokio::fs::remove_file(path).await`
- [x] Convert `read_result_file` (line 323): change `pub fn` to `pub async fn`, replace `fs::read_to_string(path)` with `tokio::fs::read_to_string(path).await`
- [x] In `run_subprocess_agent` (line 179): replace `fs::remove_file(result_path)` with `tokio::fs::remove_file(result_path).await`
- [x] In `run_subprocess_agent` (line 260): add `.await` to `read_result_file(result_path)` call (note: this is inside a `match` expression — ensure both match arms still work correctly)
- [x] In `run_subprocess_agent` (lines 264, 271): add `.await` to both `cleanup_result_file(result_path)` calls
- [x] Convert 4 test functions in `agent_test.rs` from `#[test] fn` to `#[tokio::test] async fn`: `read_result_file_valid_json` (line 51), `read_result_file_missing_file` (line 65), `read_result_file_invalid_json` (line 80), `read_result_file_missing_required_fields` (line 96)
- [x] Add `.await` to `read_result_file(...)` calls in each of the 4 converted test functions
- [x] Remove `use std::fs;` import (line 2) — do this last, after all 3 `std::fs` calls are converted to `tokio::fs`

**Verification:**

- [x] `cargo build` succeeds with no errors or warnings about unused `Future`
- [x] `cargo test` passes — all existing tests (including the 4 converted tests and 9+ already-async tests) pass
- [x] `cargo clippy` produces no new warnings
- [x] `grep -n 'std::fs' src/agent.rs` returns no results (all `std::fs` usage removed from this file)
- [x] `grep -n 'use std::fs' src/agent.rs` returns no results (import removed)
- [x] Verified by existing test `read_result_file_missing_file`: error handling in `read_result_file` differentiates `NotFound` from other `io::Error` kinds
- [x] Manual inspection: stale file cleanup in `run_subprocess_agent` still silently ignores `NotFound` and returns `Err` for any other `io::Error` (e.g., permission errors)
- [x] Manual inspection: `cleanup_result_file` still logs warning on failure without propagating the error
- [x] Manual inspection: `read_result_file` retains `pub` visibility (`pub async fn`)
- [x] Manual inspection: `cleanup_result_file` remains private (no `pub`)

**Commit:** `[WRK-016][P1] Fix: Replace blocking std::fs calls with async tokio::fs in agent.rs`

**Notes:**

The Rust compiler provides a strong safety net here: any missed `.await` produces a "unused implementor of `Future`" warning, and any attempt to call a sync function with `.await` is a type error. If `cargo build` succeeds, the mechanical conversion is correct.

Test fixture setup code (`fs::write` for creating test JSON files) remains synchronous — this is intentional. The fixture writes are to temp files on local disk and complete instantly; they are not in the production async path.

All tasks in this phase are atomic — they must all land together in a single commit. The compiler enforces this: converting function signatures without updating callers produces compile errors. The task list ordering reflects logical dependency but all changes are applied before any verification.

**Followups:**

None.

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met
- [x] Tests pass
- [x] No regressions introduced
- [x] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | complete | `[WRK-016][P1] Fix: Replace blocking std::fs calls with async tokio::fs in agent.rs` | All 8 tasks done, all 11 verification items passed, code review passed (Ready to Merge) |

## Followups Summary

### Critical

None.

### High

None.

### Medium

None.

### Low

None.

## Design Details

### Design Rationale

**`tokio::fs` over `spawn_blocking`:** Each of the 3 filesystem calls is independent and interspersed with async logic (subprocess spawn, timeout wait, status matching). `tokio::fs` is a drop-in replacement with identical error types, less boilerplate than manual `spawn_blocking` closures, and internally uses `spawn_blocking` anyway. The codebase reserves explicit `spawn_blocking` for batching multiple related operations (e.g., `coordinator.rs` git ops). Using `tokio::fs` for isolated calls is consistent with tokio ecosystem conventions.

**Single phase:** All changes must compile together — changing `read_result_file` to `async fn` without updating callers is a compile error, and vice versa. Splitting into multiple phases would leave the codebase in a non-compiling state between phases, violating the SPEC principle that each phase should leave the codebase functional.

**Test fixture setup stays sync:** `fs::write` calls in tests that create JSON fixture files remain blocking. They run before the async operation under test, complete instantly on local disk, and converting them to async would add noise without benefit.

## Assumptions

- **Error type parity is guaranteed:** `tokio::fs` delegates to `std::fs` via `spawn_blocking`, returning identical `std::io::Error` values with the same `ErrorKind` variants. This is confirmed in tokio's documentation and source code.
- **No callers beyond verified set:** `read_result_file` is called from exactly 5 sites: 1 in `run_subprocess_agent` (agent.rs) and 4 test functions (agent_test.rs). This was verified via grep and the compiler will catch any misses.
- **`tokio::fs` is available:** The `tokio` dependency uses `features = ["full"]`, which includes `tokio::fs`. No Cargo.toml changes needed.

---

## Retrospective

### What worked well?

Mechanical conversion was straightforward. The Rust compiler validated correctness — any missed `.await` would produce a warning, and any incorrect async/sync mismatch would be a type error. All 3 assumptions held: error type parity, no callers beyond the verified set, and tokio::fs availability.

### What was harder than expected?

Nothing — this was as simple as expected for a low-complexity change.

### What would we do differently next time?

Nothing to change. Single-phase atomic approach was the right structure for tightly coupled changes.
