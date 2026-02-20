# Design: Clean Up Stale .phase-golem Result Files on Startup and Shutdown

**ID:** WRK-025
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-025_feature_PRD.md
**Tech Research:** ./WRK-025_feature_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add a single async function `cleanup_stale_result_files()` to `main.rs` that scans `.phase-golem/` for `phase_result_*.json` files and deletes them. Call it twice in `handle_run()`: once at startup (Must Have) after lock acquisition and before any agents spawn, and once at shutdown (Should Have) after the backlog commit and before the lock is released. This uses the existing `tokio::fs::read_dir` + string matching pattern already established in the codebase, requires no new dependencies, and follows the existing error-swallowing cleanup pattern from `agent.rs:351-360`.

**Relationship to existing per-execution cleanup:** The codebase already has per-execution cleanup in `agent.rs` — pre-spawn deletion (`agent.rs:188-202`) and post-read deletion (`agent.rs:351-360`). Those handle result files within a single agent lifecycle. This new startup/shutdown cleanup is a complementary defense-in-depth layer that handles files orphaned by crashes or abnormal termination. Both cleanup mechanisms remain; they cover different failure scenarios.

---

## System Design

### High-Level Architecture

No new components or modules. This change adds one function and two call sites, all within `main.rs`. The function is a self-contained cleanup utility that operates on the filesystem.

```
handle_run()
  ├── lock::try_acquire()           (line 346)
  ├── cleanup_stale_result_files()  ← NEW (startup cleanup)
  ├── git::check_preconditions()    (line 347-348)
  ├── ... scheduler runs ...
  ├── backlog commit                (lines 663-721)
  ├── cleanup_stale_result_files()  ← NEW (shutdown cleanup)
  └── print summary                 (lines 723-754)
```

### Component Breakdown

#### `cleanup_stale_result_files(runtime_dir: &Path)`

**Signature:** `async fn cleanup_stale_result_files(runtime_dir: &Path)` — private function in `main.rs`. Uses `tokio::fs` because `handle_run()` is async and all file I/O in the codebase uses tokio.

**Purpose:** Delete all `phase_result_*.json` files from the given directory.

**Responsibilities:**
- Read directory entries from `runtime_dir` using `tokio::fs::read_dir` with the `next_entry().await` async iterator pattern (consistent with `executor.rs:522-543`)
- Filter entries by calling `entry.file_name().to_string_lossy()` and checking `starts_with("phase_result_")` && `ends_with(".json")` (case-sensitive)
- Delete each matching file with `tokio::fs::remove_file()`
- Log a single info-level summary when files are found: `log_info!("[pre] Cleaned up {} stale result files from .phase-golem/", count)` — no log emitted when zero files are found
- Log warning-level messages for individual file deletion failures: `log_warn!("[pre] Warning: Failed to delete stale result file {}: {}", path, err)`
- Log warning on `read_dir` failure: `log_warn!("[pre] Warning: Failed to read .phase-golem/ for cleanup: {}", err)` and return early
- Never propagate errors — all failures are swallowed after logging

**Interfaces:**
- Input: `runtime_dir: &Path` — path to the `.phase-golem/` directory
- Output: None (returns `()`, never errors)

**Dependencies:** `tokio::fs` (already available; ensure `use tokio::fs;` is imported in `main.rs`)

### Data Flow

1. `handle_run()` calls `cleanup_stale_result_files(&runtime_dir)` after acquiring the lock
2. The function calls `tokio::fs::read_dir(runtime_dir)` to iterate directory entries
3. For each entry, it checks `file_name().starts_with("phase_result_")` and `file_name().ends_with(".json")`
4. Matching files are deleted with `tokio::fs::remove_file()`
5. A count of deleted files is tracked; if > 0, an info-level summary is logged
6. The same function is called again during shutdown after the backlog commit

### Key Flows

#### Flow: Startup Cleanup (Must Have)

> Delete stale result files before any agents run, ensuring a clean slate.

1. **Lock acquired** — `lock::try_acquire()` completes, guaranteeing exclusive access and that `.phase-golem/` exists
2. **Call cleanup** — `cleanup_stale_result_files(&runtime_dir)` is called
3. **Scan directory** — `tokio::fs::read_dir()` iterates entries
4. **Filter and delete** — Matching `phase_result_*.json` files are deleted one by one
5. **Log summary** — If any files were deleted, log "Cleaned up N stale result files from .phase-golem/"
6. **Continue startup** — Proceed to git preconditions check regardless of cleanup outcome

**Edge cases:**
- Directory read fails (permissions, I/O error) — Log warning, return early, startup continues
- Individual file deletion fails — Log warning per file, continue deleting remaining files
- No matching files found — No log emitted, silent no-op
- `.phase-golem/` doesn't exist — `read_dir` returns an error, caught and handled as no-op (though in practice, lock acquisition creates the directory first)

#### Flow: Shutdown Cleanup (Should Have)

> Delete result files left from the current run after all agents complete, for tidiness.

1. **Scheduler complete** — `scheduler::run_scheduler()` returns; all agents have finished executing and all results have been read by executors
2. **Children killed** — `kill_all_children()` ensures no child processes remain (lines 651-655)
3. **Coordinator shut down** — `coord_task.await` completes, guaranteeing `save_backlog()` is done (line 658)
4. **Backlog committed** — BACKLOG.yaml changes are committed to git (lines 663-721)
5. **Call cleanup** — `cleanup_stale_result_files(&runtime_dir)` runs at line ~722, after backlog commit, before run summary. The `_lock` variable (bound at line 346) is still in scope and held until `handle_run()` returns at line 760, guaranteeing mutual exclusion throughout cleanup.
6. **Continue to summary** — Print run summary and return

**Why this is safe:** By step 1, the scheduler has returned, meaning all executor tasks that read result files have completed. No agent can be running or writing result files at this point. Shutdown cleanup only catches files that were written but not read due to executor errors or unusual control flow.

**Edge cases:**
- Same as startup (directory/file errors are swallowed)
- Lock is still held during cleanup — no race condition with other instances
- If `.phase-golem/` becomes inaccessible between startup and shutdown, `read_dir` fails, warning is logged, shutdown continues normally

---

## Technical Decisions

### Key Decisions

#### Decision: Single function for both startup and shutdown

**Context:** Both call sites need the same behavior — scan and delete `phase_result_*.json` files.

**Decision:** Use one reusable function called from both locations.

**Rationale:** Eliminates code duplication. The cleanup semantics are identical at both call sites.

**Consequences:** Simple, DRY implementation.

#### Decision: Function lives in `main.rs` (not a new module)

**Context:** The function is ~20 lines and only called from `handle_run()`.

**Decision:** Define the function in `main.rs` alongside its call sites.

**Rationale:** Creating a `cleanup.rs` module for a single small function would be over-engineering. If cleanup concerns grow later, it can be extracted then.

**Consequences:** `main.rs` grows by ~25 lines.

#### Decision: Hand-rolled `read_dir` + string matching (no `glob` crate)

**Context:** Need to match files with pattern `phase_result_*.json`.

**Decision:** Use `tokio::fs::read_dir` with `starts_with("phase_result_")` and `ends_with(".json")` checks on the filename.

**Rationale:** Matches the existing pattern in `executor.rs:522-543`. No new dependency for a simple fixed pattern. The `glob` crate would be overkill.

**Consequences:** If the file naming convention ever becomes more complex, this code would need updating. But the naming convention is well-established and unlikely to change.

#### Decision: Swallow all errors, log warnings

**Context:** Cleanup is non-critical — failing to clean up stale files should never prevent the orchestrator from running.

**Decision:** Catch all errors (both `read_dir` failure and individual `remove_file` failures), log at warning level, and continue.

**Rationale:** Matches the established pattern in `agent.rs:351-360` (`cleanup_result_file`). Startup reliability is more important than cleanup success.

**Why error-swallowing instead of strict error propagation:** The codebase has two cleanup patterns in `agent.rs`: (1) pre-spawn cleanup (`agent.rs:188-202`) returns `Err` on deletion failure because the agent cannot safely proceed with a stale result file present — it would read stale data. (2) Post-read cleanup (`agent.rs:351-360`) swallows errors because the result has already been consumed — deletion failure is inconvenient but harmless. Startup/shutdown cleanup follows pattern (2) because failing to delete stale files is the same state as before this change — no regression, and the per-execution pre-spawn cleanup provides the safety guarantee for individual files.

**Consequences:** If cleanup silently fails, stale files persist — which is the same state as before this change. No regression. Operators can monitor for cleanup failures via warning-level log messages.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Silent failure | Cleanup errors don't alert the operator | Never blocking startup/shutdown | Stale files are low-impact; blocking startup is high-impact |
| No async parallelism | Files are deleted sequentially in a loop | Simpler code, no concurrency complexity | Expected file count is tiny (< 10); sequential deletion is fast enough |
| String matching | Less flexible than glob patterns | No new dependency, consistent with codebase | Pattern is fixed and well-established |

---

## Alternatives Considered

### Alternative: `glob` Crate for Pattern Matching

**Summary:** Use the `glob` crate to match `phase_result_*.json` files.

**How it would work:**
- Add `glob` to `Cargo.toml`
- Use `glob::glob(".phase-golem/phase_result_*.json")` to find matching files
- Delete each match

**Pros:**
- Familiar glob syntax
- More flexible for complex patterns

**Cons:**
- Adds an external dependency for a single use case
- `glob` is sync-only; would need `spawn_blocking` or mixing sync/async
- Inconsistent with the async `read_dir` pattern used elsewhere in the codebase

**Why not chosen:** The pattern is simple and fixed. Hand-rolled string matching is fewer lines, no new dependency, and matches existing code patterns.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Deleting a result file from the current run | Agent result lost, phase would fail | Very Low | Startup cleanup runs before agents spawn; shutdown cleanup runs after all agents complete |
| Race with concurrent orchestrator | Could delete another instance's active result | Very Low | Lock ensures mutual exclusion — only one instance can run at a time |
| `read_dir` or `remove_file` blocks unexpectedly | Delays startup/shutdown | Very Low | Async I/O via tokio; files are local and small |

---

## Integration Points

### Existing Code Touchpoints

- `src/main.rs:346-347` — Insert startup cleanup call between lock acquisition and git preconditions
- `src/main.rs:721-723` — Insert shutdown cleanup call between backlog commit and run summary
- `src/main.rs` (module level) — Add new `cleanup_stale_result_files()` async function

**Lock scope guarantee:** The `_lock` variable is bound at line 346 and remains in scope until `handle_run()` returns at line 760. Both cleanup calls execute within this scope, guaranteeing mutual exclusion with other orchestrator instances. Do not move cleanup calls outside this scope.

**Naming convention dependency:** The cleanup function's string matching (`phase_result_` prefix, `.json` suffix) depends on the naming convention defined in `executor::result_file_path()` (`src/executor.rs:505-507`). If the naming convention changes, both locations must be updated together.

### External Dependencies

- None — uses only `tokio::fs` which is already a dependency

---

## Open Questions

None — the design is straightforward with no unresolved decisions.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Initial design draft (light mode) | Single-function approach with two call sites in handle_run() |
| 2026-02-20 | Self-critique (7 agents) + auto-fixes | Clarified shutdown timing, error handling justification, log formats, lock scope, naming dependency, PRD priority |

## Assumptions

- Autonomous mode: no user Q&A conducted. All decisions documented above.
- Light design mode is appropriate given the small/low-complexity nature of this change.
- No PRODUCT_VISION.md exists in the project root, so no vision-based tradeoff evaluation was performed.
