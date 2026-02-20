# SPEC: Clean Up Stale .phase-golem Result Files on Startup and Shutdown

**ID:** WRK-025
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-025_feature_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

Phase execution writes ephemeral result JSON files to `.phase-golem/` (e.g., `phase_result_WRK-001_build.json`). These are cleaned up per-execution in `agent.rs` — pre-spawn deletion (`agent.rs:188-202`) and post-read deletion (`agent.rs:351-360`). If the orchestrator crashes or is killed between write and read, stale files persist. Over many runs these accumulate, and a stale file from a previous run could be mistakenly read as a current result.

This change adds a complementary defense-in-depth layer: delete all `phase_result_*.json` files at startup (before agents spawn) and at shutdown (after all agents complete). This guarantees a clean slate at the beginning of each run and a tidy directory at the end.

## Approach

Add a single private async function `cleanup_stale_result_files(runtime_dir: &Path, context: &str)` to `src/main.rs` that scans the `.phase-golem/` directory for `phase_result_*.json` files and deletes them. The `context` parameter controls the log prefix tag (e.g., `"pre"` or `"post"`). Call it from two locations in `handle_run()`:

1. **Startup** (Must Have) — Immediately after `let _lock = lock::try_acquire(&runtime_dir)?;`, before `log_info!("[pre] Checking git preconditions...")`. Called with `context = "pre"`. This is the earliest safe point where the directory exists (lock acquisition creates it via `create_dir_all`) and exclusive access is guaranteed.
2. **Shutdown** (Should Have) — After the entire `if let Err(err) = coord_task.await { ... } else { ... }` block, before `log_info!("\n--- Run Summary ---")`. Called with `context = "post"`. At this point all agents have completed, all results have been read, the coordinator has shut down, and the lock is still held (the `_lock` variable remains in scope until `handle_run()` returns).

The function uses `tokio::fs::read_dir` with `starts_with("phase_result_")` and `ends_with(".json")` string matching. The loop structure follows the async `read_dir` + `next_entry().await` pattern from `executor.rs:522-543`. **Error handling diverges from that pattern:** `executor.rs` propagates errors, but this function swallows all errors after logging, following `cleanup_result_file()` in `agent.rs:351-360`.

**Patterns to follow:**

- `src/executor.rs:522-543` — async `read_dir` + `next_entry().await` loop structure **only** (not error handling)
- `src/agent.rs:351-360` — error-swallowing cleanup pattern (log warning, never propagate) — **follow this for error handling**
- `src/agent.rs:188-202` — `NotFound` error kind handling for file deletion
- `src/agent.rs:189,353` — inline `tokio::fs::remove_file(...)` usage without a `use tokio::fs` import (full path style)

**Implementation boundaries:**

- Do not modify: `src/agent.rs` — existing per-execution cleanup remains unchanged
- Do not modify: `src/executor.rs` — result file naming convention is read-only context
- Do not modify: `Cargo.toml` — no new dependencies needed (`tokio::fs` is already available via `tokio = { features = ["full"] }`, confirmed by existing usage in `executor.rs:522` and `agent.rs:189,353`; `tempfile` is already a dev-dependency for tests)
- Do not refactor: existing cleanup patterns in `agent.rs`; this is additive only

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Implement cleanup function and call sites | Low | Add `cleanup_stale_result_files()` to `main.rs` with startup and shutdown call sites, plus unit tests |

**Ordering rationale:** This is a single-phase change — one function, two call sites, and tests. There are no dependencies to sequence across phases.

---

## Phases

### Phase 1: Implement cleanup function and call sites

> Add `cleanup_stale_result_files()` to `main.rs` with startup and shutdown call sites, plus unit tests

**Phase Status:** complete

**Complexity:** Low

**Goal:** Implement the `cleanup_stale_result_files` function, wire it into `handle_run()` at both startup and shutdown, and add unit tests verifying correct behavior.

**Files:**

- `src/main.rs` — modify — add ~30 lines: new async function + 2 call site insertions + `#[cfg(test)]` module with tests

**Patterns:**

- Follow `src/executor.rs:522-543` for `tokio::fs::read_dir` + `next_entry().await` loop structure
- Follow `src/agent.rs:351-360` for error-swallowing cleanup (`log_warn!` on failure, never propagate) — **use this error handling pattern, not the propagation pattern from executor.rs**
- Use full-path `tokio::fs::read_dir(...)` / `tokio::fs::remove_file(...)` style (no `use tokio::fs;` import), matching `agent.rs:189,353`

**Tasks:**

- [x] Add `async fn cleanup_stale_result_files(runtime_dir: &Path, context: &str)` to `src/main.rs`:
  - Call `tokio::fs::read_dir(runtime_dir).await`; on error, `log_warn!("[{}] Failed to read .phase-golem/ for cleanup: {}", context, err)` and return
  - Iterate entries with `next_entry().await`; on iteration error, `log_warn!` and break
  - For each entry, check `file_name().to_string_lossy()` with `starts_with("phase_result_")` && `ends_with(".json")`
  - Delete matching files with `tokio::fs::remove_file(entry.path()).await`; on error, `log_warn!("[{}] Failed to delete stale result file {}: {}", context, path, err)` — **continue to next file, do not break or return**
  - Track count of deleted files; if count > 0, `log_info!("[{}] Cleaned up {} stale result file(s) from .phase-golem/", context, count)`. When count == 0, emit no log (silent no-op).
  - Function signature returns `()` — never propagates errors
- [x] Insert startup call: add `cleanup_stale_result_files(&runtime_dir, "pre").await;` immediately after `let _lock = lock::try_acquire(&runtime_dir)?;` and before `log_info!("[pre] Checking git preconditions...")` (semantic anchors; currently lines 346–347, but use the surrounding code as the anchor if line numbers have shifted)
- [x] Insert shutdown call: add `cleanup_stale_result_files(&runtime_dir, "post").await;` after the closing `}` of the entire `if let Err(err) = coord_task.await { ... } else { ... }` block, before `log_info!("\n--- Run Summary ---")` (semantic anchors; currently after line 721, before line 723). **Important:** the call must be outside both branches of the if/else, so cleanup runs regardless of whether the coordinator completed normally or panicked.
- [x] Add `#[cfg(test)]` module in `src/main.rs` with unit tests (all tests use `#[tokio::test]`, not `#[test]`, since the function is async):
  - `cleanup_deletes_matching_files` — create temp dir (via `tempfile::tempdir()`) with `phase_result_WRK-001_build.json` and `phase_result_WRK-002_prd.json`, call cleanup, assert files are deleted
  - `cleanup_ignores_non_matching_files` — create temp dir with `phase-golem.lock`, `other.json`, `phase_result_WRK-001_build.txt`, call cleanup, assert non-matching files still exist
  - `cleanup_handles_missing_directory` — call cleanup on a non-existent path, assert no panic
  - `cleanup_handles_empty_directory` — create empty temp dir, call cleanup, assert no panic
  - `cleanup_continues_after_partial_failure` — create temp dir with: a regular file `phase_result_WRK-001_build.json`, a subdirectory named `phase_result_stuck.json` (which `remove_file` will fail on with `EISDIR`), and a second regular file `phase_result_WRK-002_prd.json`. Call cleanup, assert both regular files are deleted and the subdirectory still exists, confirming the function iterates past failures.
  - `cleanup_handles_directory_entry_with_matching_name` — create a subdirectory named `phase_result_WRK-003_test.json` alongside a regular matching file, run cleanup, assert no panic and the regular file is deleted

**Verification:**

- [x] `cargo build` succeeds without warnings
- [x] `cargo test --all` passes (all existing tests + new tests)
- [x] `cargo clippy` produces no new warnings
- [x] Code review confirms: `cleanup_stale_result_files` call appears after `lock::try_acquire` and before `log_info!("[pre] Checking git preconditions...")` (startup ordering)
- [x] Code review confirms: shutdown call is outside the `if let Err(err) = coord_task.await` block, before `log_info!("\n--- Run Summary ---")`
- [x] Code review confirms: both cleanup calls execute while `_lock` is in scope (before `handle_run()` returns)
- [x] Manual verification (startup): create dummy `phase_result_test_build.json` in `.phase-golem/`, run `phase-golem run`, confirm file is deleted at startup (visible in `[pre]` info-level log) — verified by unit tests; manual run skipped (autonomous mode, no running environment)
- [x] Manual verification (shutdown): after a normal run with at least one phase execution, confirm no `phase_result_*.json` files remain in `.phase-golem/` (visible in `[post]` info-level log if any were present) — verified by unit tests; manual run skipped (autonomous mode)
- [x] Performance: cleanup completes in under 100ms for typical file counts (< 100 files) — met by design (single sequential `read_dir` over local files)
- [x] Code review passes

**Commit:** `[WRK-025][P1] Feature: startup/shutdown cleanup of stale result files`

**Notes:**

- The function is private (`async fn`, not `pub`), so tests must live in a `#[cfg(test)]` module inside `main.rs`. This is the standard Rust pattern for testing private functions in binary crates.
- `runtime_dir` is defined at line 345 as `let runtime_dir = root.join(".phase-golem")`, confirmed to resolve to the `.phase-golem/` directory. It is available at both call sites in `handle_run()`.
- `lock::try_acquire()` (in `lock.rs:44-94`) calls `create_dir_all` on the runtime directory, guaranteeing `.phase-golem/` exists before the startup cleanup call. If `.phase-golem/` somehow doesn't exist, `read_dir` returns an error that is swallowed.
- The `_lock` variable stays in scope until `handle_run()` returns (line 760), so both cleanup calls execute under the lock. The lock is exclusive — `lock::try_acquire` returns `Err` if another instance holds it, guaranteeing no concurrent orchestrator can be running during cleanup.
- The naming convention (`phase_result_` prefix, `.json` suffix) is a semantic dependency on `executor::result_file_path()` at `executor.rs:505-507`. Add a `// NOTE: must match executor::result_file_path() naming convention` inline comment at the string literal sites. See Followups for extracting shared constants.
- `tokio::fs` is available without `Cargo.toml` changes — confirmed by existing usage in `agent.rs:189,353` and `executor.rs:522`. `tempfile` is already a dev-dependency.
- The logging macros (`log_info!`, `log_warn!`) are safe to call in test context — they write to stderr and do not require initialization. Tests may produce log output but will not panic.

**Followups:**

- [ ] [Low] Extract `"phase_result_"` prefix and `".json"` suffix into shared constants (e.g., `RESULT_FILE_PREFIX` / `RESULT_FILE_SUFFIX` in `executor.rs`) to create compile-time coupling between `executor::result_file_path()` and `cleanup_stale_result_files()`. Currently the naming convention is a string-literal semantic dependency with no automated enforcement. Deferred because it requires modifying `executor.rs` which is out of scope for this change.

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met:
  - [x] Startup: all `phase_result_*.json` deleted before agents spawn
  - [x] Startup: cleanup runs after lock acquisition, before coordinator starts
  - [x] Startup: each file deletion failure logged as warning, startup continues
  - [x] Startup: if `.phase-golem/` unreadable, warning logged, no abort
  - [x] Shutdown: all `phase_result_*.json` deleted while lock held, after agents complete and coordinator has shut down
  - [x] Logging: info-level summary when stale files found, no log when zero files
- [x] Tests pass
- [x] No regressions introduced
- [x] Code reviewed

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | complete | pending | Added cleanup function, startup/shutdown call sites, and 6 unit tests. Code review passed. |

## Followups Summary

### Critical

### High

### Medium

### Low

- [ ] Extract naming convention constants — `"phase_result_"` prefix and `".json"` suffix should be shared constants between `executor.rs` and `main.rs` for compile-time coupling
- [ ] Add symlink check before deletion in `cleanup_stale_result_files` and existing cleanup paths in `agent.rs` — low priority, requires write access to locked runtime directory to exploit

## Design Details

### Key Types

No new types introduced. The function signature is:

```rust
async fn cleanup_stale_result_files(runtime_dir: &Path, context: &str)
```

The `context` parameter is a log prefix tag (`"pre"` for startup, `"post"` for shutdown) used in all log messages emitted by the function.

### Architecture Details

The function is a self-contained cleanup utility with no dependencies beyond `tokio::fs` and the logging macros. It operates on the filesystem only and has no interaction with the orchestrator's internal state.

```
handle_run()
  ├── lock::try_acquire()                    (line 346)
  ├── cleanup_stale_result_files("pre")      ← NEW (startup)
  ├── git::check_preconditions()             (line 347-348)
  ├── ... config, preflight, scheduler ...
  ├── kill_all_children()                    (lines 651-655)
  ├── coord_task.await + backlog commit      (lines 658-721)
  ├── cleanup_stale_result_files("post")     ← NEW (shutdown, outside if/else)
  └── print summary                          (lines 723-754)
```

### Design Rationale

- **Single function, two call sites:** Eliminates duplication — cleanup semantics are identical at startup and shutdown, with only the log prefix differing.
- **`context` parameter for log prefix:** The function is called from both startup (`[pre]`) and shutdown (`[post]`) contexts. A parameter avoids hardcoding a misleading `[pre]` tag in shutdown logs.
- **Shutdown placement outside if/else:** The shutdown call is placed after the entire `coord_task.await` if/else block so cleanup runs regardless of whether the coordinator completed normally or panicked. In both cases, all agents have finished and no result files are actively in use.
- **In `main.rs`, not a new module:** The function is ~20 lines and only called from `handle_run()`. A separate module would be over-engineering.
- **Hand-rolled string matching, no `glob` crate:** The pattern is simple and fixed (`phase_result_*.json`). Matches existing `read_dir` patterns in the codebase.
- **Swallow all errors:** Cleanup is non-critical. Failing to clean up is the same state as before this change — no regression. The per-execution pre-spawn cleanup in `agent.rs:188-202` provides the safety guarantee against reading stale data.

---

## Self-Critique Summary

This SPEC was refined through an automated self-critique cycle (6 critique agents, triage, auto-fix).

**Auto-fixed (16):**
- Added `context: &str` parameter to function signature for accurate log prefixes at startup (`[pre]`) and shutdown (`[post]`)
- Fixed shutdown call site placement: outside the `if let Err(err) = coord_task.await` if/else block, not inside the `else` branch
- Specified `#[tokio::test]` annotation requirement for all async test functions
- Added `cleanup_continues_after_partial_failure` test case (PRD requires iterating past individual deletion failures)
- Added `cleanup_handles_directory_entry_with_matching_name` test case
- Replaced raw line number references with semantic anchors (line numbers retained as hints)
- Clarified `executor.rs:522-543` is referenced for loop structure only; error handling follows `agent.rs:351-360`
- Added citation for `runtime_dir = root.join(".phase-golem")` at line 345
- Added note that `lock::try_acquire` creates `.phase-golem/` via `create_dir_all`
- Documented lock exclusivity guarantees (no concurrent orchestrator during cleanup)
- Added `tokio::fs` availability citation
- Added shutdown manual verification step
- Added explicit code review checks for call-site ordering
- Added `cargo test --all` instead of `cargo test`
- Added performance NFR acknowledgment (met by design)
- Added naming convention coupling followup item

**Quality items (not auto-fixed — noted for awareness):**
- Log capture testing: the logging macros do not have a test-capture mechanism, so the "no log when zero files" invariant is verified by code inspection rather than automated assertion
- Call-site integration test: verifying that `handle_run()` continues after cleanup failure requires a full integration harness; covered by code review confirming the function returns `()` and call sites don't wrap it in `?`
- Deletion count assertion: the exact count logged is not asserted in tests; verified by code inspection

## Assumptions

- Autonomous mode: no user Q&A conducted. All decisions documented in design artifacts.
- Single-phase SPEC is appropriate for this small, low-complexity change.
- Light mode used given the simplicity (one function, two call sites, unit tests).
- The `context: &str` parameter approach was chosen over removing the prefix entirely, because the `[pre]`/`[post]` tags match the existing log tagging convention in the codebase and provide useful context for operators reading logs.
