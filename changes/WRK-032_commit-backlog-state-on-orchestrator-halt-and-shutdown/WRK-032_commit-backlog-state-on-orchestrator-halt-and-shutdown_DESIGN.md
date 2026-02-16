# Design: Commit backlog state on orchestrator halt and shutdown

**ID:** WRK-032
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-032_commit-backlog-state-on-orchestrator-halt-and-shutdown_PRD.md
**Tech Research:** ./WRK-032_commit-backlog-state-on-orchestrator-halt-and-shutdown_TECH_RESEARCH.md
**Mode:** Light

## Overview

When the orchestrator halts, BACKLOG.yaml may have uncommitted changes from status transitions, block reasons, or follow-up ingestion that occurred outside phase completions. This design adds a final git commit step to `handle_run()` that runs after the coordinator actor completes `save_backlog()`, committing BACKLOG.yaml if dirty. The approach modifies `spawn_coordinator()` to return its `JoinHandle` so the shutdown sequence can await coordinator completion before committing, keeping git concerns out of the coordinator actor and enabling the halt reason to be included in the commit message.

---

## System Design

### High-Level Architecture

The change is scoped to two functions and affects no new components. The existing shutdown sequence in `handle_run()` gains an additional step between the scheduler returning and the summary being printed.

```
Current:  run_scheduler() → kill_children → print summary → return
Proposed: run_scheduler() → kill_children → await coordinator JoinHandle → commit BACKLOG.yaml if dirty → print summary → return
```

### Component Breakdown

#### spawn_coordinator (modified)

**Purpose:** Spawns the coordinator actor task and returns handles for communication and lifecycle management.

**Change:** Return `(CoordinatorHandle, JoinHandle<()>)` instead of just `CoordinatorHandle`.

**Interfaces:**
- Input: Unchanged (backlog, paths, prefix)
- Output: Tuple of `(CoordinatorHandle, JoinHandle<()>)` — the sender handle for commands, and the task handle for awaiting completion

**Dependencies:** Unchanged

#### handle_run (modified)

**Purpose:** Top-level orchestrator run handler. Owns the full lifecycle from setup to shutdown.

**Change:** After `run_scheduler()` returns (which drops the `CoordinatorHandle`, triggering coordinator shutdown), await the `JoinHandle` to ensure `save_backlog()` has completed, then check if BACKLOG.yaml is dirty and commit if so.

**Interfaces:**
- Input: Unchanged
- Output: Unchanged (`Result<(), String>`)

**Dependencies:** `crate::git::{get_status, stage_paths, commit}`

### Data Flow

1. `run_scheduler()` returns `RunSummary` (which includes `halt_reason`). The `CoordinatorHandle` is consumed/dropped by the scheduler.
2. `kill_all_children()` runs (unchanged).
3. `handle_run()` awaits the coordinator `JoinHandle`. If the handle returns `Err(JoinError)` (coordinator panicked), log a warning and skip to step 7 — the on-disk state may be inconsistent. If `Ok(())`, proceed.
4. `handle_run()` calls `git::get_status(Some(&root))` and filters entries for `BACKLOG.yaml`. The filter checks whether `entry.path` equals the backlog filename (`BACKLOG.yaml`) or, for git-quoted paths, whether the unquoted path matches. Any status code (staged or unstaged) qualifies as dirty.
5. If BACKLOG.yaml is dirty, `git::stage_paths(&[&backlog_path], Some(&root))` stages it and `git::commit(&message, Some(&root))` creates the commit with message `format!("[orchestrator] Save backlog state on halt ({:?})", summary.halt_reason)`.
6. Git errors (from `get_status`, `stage_paths`, or `commit`) are logged with `log_warn!` and do not prevent `handle_run()` from returning `Ok(())`. If `stage_paths` succeeds but `commit` fails, BACKLOG.yaml remains staged — this is acceptable since the next orchestrator run or manual commit will pick it up.
7. Summary is printed (unchanged).

### Key Flows

#### Flow: Successful backlog commit on halt

> Orchestrator halts with dirty BACKLOG.yaml; the file is committed to git automatically.

1. **Scheduler halts** — `run_scheduler()` returns `RunSummary` with a `HaltReason` (e.g., `CapReached`). The `CoordinatorHandle` is dropped.
2. **Kill children** — `kill_all_children()` terminates lingering subprocesses.
3. **Await coordinator** — `handle_run()` awaits `JoinHandle<()>`. If `Err(JoinError)`, log `log_warn!("Coordinator task panicked, skipping backlog commit: {:?}", err)` and skip to step 7. If `Ok(())`, the coordinator has finished `save_backlog()`.
4. **Check dirty** — `git::get_status(Some(&root))` returns status entries. Filter for entries where `entry.path == "BACKLOG.yaml"` (or the path unquoted if git quoted it). Any status code (staged `M `, unstaged ` M`, untracked `??`, etc.) qualifies.
5. **Stage and commit** — `git::stage_paths(&[&backlog_path], Some(&root))` followed by `git::commit(&message, Some(&root))` with message `format!("[orchestrator] Save backlog state on halt ({:?})", summary.halt_reason)` — e.g., `[orchestrator] Save backlog state on halt (CapReached)`.
6. **Log** — `log_info!` records the commit SHA.
7. **Print summary** — Normal summary output (unchanged).

**Edge cases:**
- BACKLOG.yaml is clean — `get_status()` returns no matching entries; commit is skipped. No log noise.
- Git operation fails — `log_warn!` with the error message; `handle_run()` continues to print summary and return `Ok(())`. If `stage_paths()` succeeded but `commit()` failed, BACKLOG.yaml is left staged — acceptable since the next run or manual commit picks it up.
- Coordinator panics — `JoinHandle` returns `Err(JoinError)`. Log warning: `"Coordinator task panicked, skipping backlog commit"`. Skip directly to summary. BACKLOG.yaml may or may not be saved to disk; we skip the commit since on-disk state may be inconsistent.
- `save_backlog()` fails inside coordinator — The coordinator logs this internally (`let _ = state.save_backlog()`). Because `save_backlog()` uses atomic write (temp file + fsync + rename), a failure leaves the previous BACKLOG.yaml intact. If the previous state was already committed, `get_status()` shows no changes and the commit is skipped. If the previous state had uncommitted changes, those (stale) changes would be committed — this is acceptable since they represent the last known-good state.
- `spawn_blocking` fails (JoinError from the blocking task) — treated the same as a git operation failure: log warning, skip commit, continue to summary.

#### Flow: Clean BACKLOG.yaml on halt (no-op)

> Orchestrator halts but all backlog changes were already committed by phase commits.

1. **Scheduler halts** — Same as above.
2. **Await coordinator** — Same as above.
3. **Check dirty** — `get_status()` finds no entries for BACKLOG.yaml.
4. **Skip** — No stage, no commit, no log.
5. **Print summary** — Normal output.

---

## Technical Decisions

### Key Decisions

#### Decision: Commit from handle_run() rather than inside the coordinator actor

**Context:** The commit could happen inside the coordinator's shutdown hook (after `save_backlog()`) or from `handle_run()` after awaiting the coordinator.

**Decision:** Commit from `handle_run()`.

**Rationale:**
- The halt reason (needed for the commit message) is only available in `handle_run()` from the `RunSummary`
- Git commits are an infrastructure concern that belongs at the orchestrator level, not inside the coordinator actor whose responsibility is state management
- Errors are observable by the caller
- The coordinator remains a pure state manager without git knowledge

**Consequences:** Requires `spawn_coordinator()` to return the `JoinHandle` so `handle_run()` can await it.

#### Decision: Use spawn_blocking for git operations in the shutdown path

**Context:** Git operations (`std::process::Command`) are blocking. In `handle_run()`, we're in an async context.

**Decision:** Use `tokio::task::spawn_blocking` for the git dirty-check and commit, matching the existing `batch_commit` pattern.

**Rationale:**
- Consistent with the existing pattern in `batch_commit()` (coordinator.rs:601)
- Avoids blocking a tokio worker thread
- The runtime is still alive at this point (we're inside `handle_run()` which is called from `main()`)

**Consequences:** Minor additional complexity from spawn_blocking error handling (JoinError), but the pattern is already established.

#### Decision: Filter get_status() for BACKLOG.yaml only

**Context:** `get_status()` returns all dirty files. We only care about BACKLOG.yaml.

**Decision:** Call `get_status()`, then filter the returned `Vec<StatusEntry>` for entries matching the backlog filename.

**Matching logic:** Compare `entry.path` against the backlog filename (`"BACKLOG.yaml"`). Git `--porcelain` returns paths relative to the repo root. Since BACKLOG.yaml sits at the repo root, the path is just the filename. For git-quoted paths (paths containing special characters are wrapped in `""`), strip surrounding quotes before comparing. Any status code qualifies as dirty — both staged and unstaged changes trigger the commit (per PRD: "staged or unstaged changes").

**Rationale:**
- Reuses the existing `get_status()` function without modification
- Ensures we never accidentally stage or commit other dirty files
- Simple string comparison on the path field; BACKLOG.yaml at repo root avoids subdirectory ambiguity

**Consequences:** Slightly more data fetched than strictly necessary, but `git status --porcelain` is fast and the output is small.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| API change | `spawn_coordinator()` return type changes from `CoordinatorHandle` to `(CoordinatorHandle, JoinHandle<()>)` | Explicit lifecycle management; caller can await coordinator completion | Small, targeted change; only one call site (`handle_run()`). Future callers must await or explicitly drop the JoinHandle. |
| Warn-and-continue errors | Git commit failure on shutdown is a warning, not an error | Clean process exit regardless of git state | BACKLOG.yaml is already saved to disk; git commit is a durability bonus |
| spawn_blocking complexity | Adds JoinError handling layer around git operations | Consistency with existing `batch_commit` pattern; avoids blocking tokio worker thread | The shutdown path has no other async work, so blocking would be harmless in practice. spawn_blocking is chosen for consistency, not necessity. |

---

## Alternatives Considered

### Alternative: Commit inside the coordinator shutdown hook

**Summary:** Add git stage + commit calls directly after `save_backlog()` in the coordinator's shutdown path (coordinator.rs:683).

**How it would work:**
- After `save_backlog()` succeeds, call `git::stage_paths()` and `git::commit()` synchronously
- No changes needed to `spawn_coordinator()` return type

**Pros:**
- No API changes — `spawn_coordinator()` stays the same
- Ordering is trivially correct (runs immediately after `save_backlog()`)
- Self-contained — all callers benefit automatically

**Cons:**
- Cannot include the halt reason in the commit message (coordinator doesn't know why the scheduler halted)
- Mixes git infrastructure concerns into the coordinator actor
- Errors during commit are not observable by the caller (`JoinHandle` is discarded)
- Blocking git calls on a tokio worker thread (could use `spawn_blocking` but adds complexity inside the actor)

**Why not chosen:** The PRD requires including the halt reason in the commit message, which is only available in `handle_run()`. Additionally, the separation of concerns is cleaner with the recommended approach.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Coordinator task panics, `JoinHandle` returns `JoinError` | Commit is skipped; BACKLOG.yaml may not be saved to disk | Low — coordinator has been stable | Check JoinHandle result; if `Err`, log warning and skip commit. Disk state is best-effort. |
| `spawn_blocking` during late shutdown may not execute (tokio#7499) | Git commit doesn't happen | Very Low — runtime is alive when we reach this code | Accepted risk. If observed in practice, replace `spawn_blocking` with synchronous git calls — safe in shutdown since no other async work is running. |
| Second SIGTERM/SIGKILL during commit leaves `.git/index.lock` | Manual cleanup needed | Very Low — narrow time window | Standard git recovery (`rm .git/index.lock`); documented in PRD risks |
| `stage_paths` succeeds but `commit` fails | BACKLOG.yaml left staged in git index | Very Low — commit failures are rare | Log warning; next orchestrator run or manual commit resolves. Acceptable since disk state is preserved. |

---

## Integration Points

### Existing Code Touchpoints

- `orchestrator/src/coordinator.rs` — `spawn_coordinator()`: change return type from `CoordinatorHandle` to `(CoordinatorHandle, tokio::task::JoinHandle<()>)`. Store the `JoinHandle` from `tokio::spawn` in a local variable and return it alongside the handle. (~3 lines changed)
- `orchestrator/src/main.rs` — `handle_run()`: destructure the tuple from `spawn_coordinator()` as `let (coord_handle, coord_task) = ...`. Pass only `coord_handle` to `run_scheduler()`. After `run_scheduler()` returns, await `coord_task`, then run dirty check + commit logic.
- `orchestrator/src/git.rs` — No changes needed; existing `get_status()`, `stage_paths()`, and `commit()` are used as-is

Note: Line numbers are approximate and based on the codebase at design time.

### External Dependencies

- None — all functionality uses existing in-codebase git helpers wrapping `std::process::Command`

---

## Open Questions

- None — all questions from the PRD and tech research have been resolved.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain

---

## Assumptions

Decisions made without human input (autonomous mode):

- **Mode selection:** Light mode, matching the tech research mode and the item's low complexity/small size assessments.
- **No product vision file exists** — no PRODUCT_VISION.md was found, so design decisions are guided by the PRD and tech research alone.
- **Backlog path matching:** The dirty check compares `entry.path` from `get_status()` against the literal filename `"BACKLOG.yaml"` (stripping git quotes if present). This works because BACKLOG.yaml sits at the repo root and git porcelain returns repo-relative paths.
- **spawn_blocking vs synchronous git:** Self-critique flagged spawn_blocking as potentially unnecessary in the shutdown path (no other async work running). Chose to keep spawn_blocking for consistency with the existing `batch_commit` pattern, but acknowledged in tradeoffs that synchronous calls would also be acceptable here. The spec/build phase may simplify to synchronous if it proves cleaner.
- **Approach B (handle_run) over Approach A (coordinator hook):** Self-critique's simplicity agent advocated for Approach A. Chose Approach B because the PRD "should have" explicitly requests halt reason in the commit message, which is only available in `handle_run()`. The API change is small (one call site).

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Initial design draft (autonomous) | Full design with recommended approach (commit from handle_run), one alternative (commit inside coordinator), flows and decisions documented |
| 2026-02-13 | Self-critique (7 agents) + auto-fix | Added explicit JoinError handling to data flow, clarified path matching logic, added staged-but-uncommitted risk, promoted coordinator panic to risks table, clarified spawn_blocking tradeoff, documented directional decisions in Assumptions |
