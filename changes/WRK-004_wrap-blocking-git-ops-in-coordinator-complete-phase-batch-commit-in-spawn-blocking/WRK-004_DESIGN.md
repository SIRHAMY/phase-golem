# Design: Wrap Blocking Git Ops in spawn_blocking

**ID:** WRK-004
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-004_PRD.md
**Tech Research:** ./WRK-004_TECH_RESEARCH.md
**Mode:** Light

## Overview

Move the synchronous git calls in the `CompletePhase` and `BatchCommit` match arms of `run_coordinator` into `tokio::task::spawn_blocking` closures, following the existing `GetHeadSha`/`IsAncestor` pattern. State mutations (`pending_batch_phases.push`/`.clear()`) remain on the async executor thread, outside the closures. This is a straightforward pattern-replication change with no new abstractions.

---

## System Design

### High-Level Architecture

No architectural changes. The coordinator actor loop remains the same structure. The only change is *where* git subprocess calls execute: they move from the async executor thread to tokio's blocking thread pool via `spawn_blocking`.

```
Before:
  Actor Loop (async thread) → git::get_status / stage_paths / commit (blocks thread)

After:
  Actor Loop (async thread) → spawn_blocking → git::get_status / stage_paths / commit (blocking thread pool)
                             ← .await result  ← return Result
  Actor Loop (async thread) → state mutation (pending_batch_phases)
```

### Component Breakdown

#### `run_coordinator` Match Arms (Modified)

**Purpose:** Dispatch `CoordinatorCommand` variants to appropriate handlers.

**Responsibilities:**
- Clone owned data from `CoordinatorState` for the blocking closure
- Spawn blocking closure containing git I/O
- `.await` the `JoinHandle` and handle `JoinError`
- Perform state mutations on the async thread after `.await` returns `Ok`
- Send result through reply channel

**Interfaces:**
- Input: `CoordinatorCommand::CompletePhase` and `CoordinatorCommand::BatchCommit` variants
- Output: `Result<(), String>` sent via `reply` oneshot channel

**Dependencies:** `tokio::task::spawn_blocking`, `crate::git::*`, `CoordinatorState`

#### Helper Functions (Removed)

The existing `handle_complete_phase` and `handle_batch_commit` functions will be removed. Their git I/O logic moves into the `spawn_blocking` closures in the match arms. Their state mutation logic moves into the match arm code after the `.await`.

#### Git Module (Unchanged)

**Purpose:** Synchronous git operations via `std::process::Command`.

**Note:** No changes to this module. All functions (`get_status`, `stage_paths`, `commit`) remain synchronous.

### Data Flow

#### CompletePhase

1. Clone `project_root`, `backlog_path`, `output_paths` from state/args into owned values. Also clone `item_id` and `result.phase` if destructive (needed inside closure for commit message), or keep them on the async thread if non-destructive (needed for `pending_batch_phases.push`).
2. Spawn a single blocking closure that: calls `get_status` → `collect_orchestrator_paths` → `stage_paths`. If `is_destructive`, also calls `build_phase_commit_message` → `get_status` → `commit`. Returns `Result<(), String>`.
3. `.await` the `JoinHandle`, flatten `JoinError` into `Result<(), String>` via `.unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {}", e)))`
4. If non-destructive and result is `Ok(())`, push `(item_id, phase)` to `state.pending_batch_phases` on the async thread
5. Send result via reply channel

#### BatchCommit

1. Check if `pending_batch_phases` is empty — if so, return `Ok(())` immediately (no `spawn_blocking` needed)
2. Clone `project_root` and `pending_batch_phases` for the blocking closure
3. Spawn blocking closure that: calls `get_status` → if staged changes exist, builds batch commit message via `build_batch_commit_message` and calls `commit`; returns `Result<(), String>` where `Ok(())` means either no staged changes existed or commit succeeded
4. `.await` the `JoinHandle`, flatten `JoinError` via `.unwrap_or_else`
5. If result is `Ok(())`, clear `state.pending_batch_phases` on async thread. If result is `Err`, do NOT clear (so the user can retry).
6. Send result via reply channel

### Key Flows

#### Flow: CompletePhase (Destructive Path)

> Stage output files and commit immediately for a destructive phase.

1. **Clone data** — Clone `project_root`, `backlog_path`, `output_paths`, `item_id`, `result.phase` into owned values for the closure
2. **Spawn blocking** — Move all owned data into a single `spawn_blocking` closure
3. **Get status** — Call `git::get_status` to discover modified files
4. **Collect paths** — Call `collect_orchestrator_paths` with the status result to find orchestrator-managed paths
5. **Stage paths** — Call `git::stage_paths` with backlog path + output paths + orchestrator paths
6. **Build message** — Call `build_phase_commit_message` with item_id and phase
7. **Check staged** — Call `git::get_status` again, check `has_staged_changes`
8. **Commit** — If staged changes exist, call `git::commit`
9. **Return result** — Return `Result<(), String>` from closure
10. **Await + send** — `.await` the handle, flatten `JoinError`, send via reply

**Edge cases:**
- No files changed → `stage_paths` is a no-op, `has_staged_changes` returns false, no commit happens → returns `Ok(())`
- Git command fails → error propagated via `Result::Err` from closure
- `spawn_blocking` panics → caught by `.unwrap_or_else`, returns descriptive error string
- `stage_paths` fails after partial staging → error propagated, files may remain staged but no commit occurs

#### Flow: CompletePhase (Non-Destructive Path)

> Stage output files without committing, queue for later batch commit.

1. **Clone data for closure** — Clone `project_root`, `backlog_path`, `output_paths` into owned values for the closure. Keep `item_id` and `result.phase` on the async thread (they are NOT moved into the closure — they're needed for `pending_batch_phases.push`)
2. **Spawn blocking** — Move closure data into `spawn_blocking` closure
3. **Get status** — Call `git::get_status` to discover modified files
4. **Collect paths** — Call `collect_orchestrator_paths` with status result
5. **Stage paths** — Call `git::stage_paths` with backlog path + output paths + orchestrator paths
6. **Return result** — Return `Result<(), String>` from closure (staging result only)
7. **Await** — `.await` the handle, flatten `JoinError`
8. **Push to pending** — If result is `Ok(())`, push `(item_id, phase)` to `state.pending_batch_phases` on async thread. If result is `Err`, do NOT push.
9. **Send result** — Send via reply

**Edge cases:**
- Staging fails → error returned, `pending_batch_phases` not modified (correct behavior)

#### Flow: BatchCommit

> Commit all pending non-destructive phase outputs in a single commit.

1. **Early return** — If `pending_batch_phases` is empty, return `Ok(())` immediately (no `spawn_blocking` needed)
2. **Clone data** — Clone `project_root` and `pending_batch_phases` for the closure
3. **Spawn blocking** — Move owned data into `spawn_blocking` closure
4. **Get status** — Call `git::get_status` to check for staged changes
5. **Branch on staged** — If no staged changes, return `Ok(())`; if staged changes exist, call `build_batch_commit_message` with the cloned pending phases, then call `git::commit`
6. **Return result** — Return `Result<(), String>` from closure. `Ok(())` means either "no staged changes" or "commit succeeded"
7. **Await** — `.await` the handle, flatten `JoinError`
8. **Clear pending on success** — If result is `Ok(())`, clear `state.pending_batch_phases` on async thread. If result is `Err`, do NOT clear (preserves pending list so user can retry)
9. **Send result** — Send via reply

**Edge cases:**
- No staged changes → closure returns `Ok(())`, pending list cleared (phases were already staged by `complete_phase`, clearing is correct)
- Commit fails → closure returns `Err(msg)`, pending list NOT cleared, error sent via reply
- Empty pending list → immediate return before `spawn_blocking`, no git operations

---

## Technical Decisions

### Key Decisions

#### Decision: Inline spawn_blocking in match arms (remove handler functions)

**Context:** The existing `handle_complete_phase` and `handle_batch_commit` are synchronous helper functions called from match arms. We need to decide where to place the `spawn_blocking` boundary.

**Decision:** Place `spawn_blocking` directly in the match arms of `run_coordinator`, matching the `GetHeadSha`/`IsAncestor` pattern. Remove the `handle_complete_phase` and `handle_batch_commit` helper functions entirely — their git I/O logic moves into the closures, and state mutation logic stays in the match arm.

**Rationale:** Consistency with the existing codebase pattern. Having all `spawn_blocking` calls visible at the match arm level makes it immediately obvious which operations block and which don't.

**Consequences:** The match arms for `CompletePhase` and `BatchCommit` will grow in size (~20-30 lines each), but this is acceptable because the `GetHeadSha`/`IsAncestor` arms already set this precedent. The `handle_complete_phase` and `handle_batch_commit` functions are removed.

#### Decision: Single spawn_blocking closure per command, branch internally

**Context:** `handle_complete_phase` branches on `is_destructive` — destructive commits immediately, non-destructive just stages. We could use two separate closures or one closure that branches internally.

**Decision:** Use a single `spawn_blocking` closure for `CompletePhase` that branches on `is_destructive` inside the closure. The closure performs staging (shared logic), then conditionally commits (destructive only). Returns `Result<(), String>`.

**Rationale:** Avoids duplicating the staging logic (get_status → collect_orchestrator_paths → stage_paths) across two separate closures. A single closure keeps the shared logic in one place. The `is_destructive` boolean is moved into the closure alongside the other owned data.

**Consequences:** The closure is slightly longer due to the internal branch, but all staging logic exists in exactly one place. The match arm code after `.await` still branches on `is_destructive` for the state mutation (`pending_batch_phases.push` only on the non-destructive path).

#### Decision: No consolidated spawn_blocking helper

**Context:** The PRD lists a consolidated helper as "Nice to Have."

**Decision:** Skip the helper. Use the raw `spawn_blocking(...).await.unwrap_or_else(...)` pattern inline.

**Rationale:** Only 2 call sites. A helper would need generics for the return type and adds indirection for minimal deduplication. The 2-line boilerplate is clear and matches existing code.

**Consequences:** Minor code duplication (the `.unwrap_or_else` pattern appears 4 times total: GetHeadSha, IsAncestor, CompletePhase, BatchCommit). Acceptable at this scale.

#### Decision: Clear pending_batch_phases only on Ok in BatchCommit

**Context:** The current `handle_batch_commit` clears `pending_batch_phases` in two places: when there are no staged changes (early return) and after a successful commit. Both successful paths clear the list. On error, the current code returns `Err` before reaching the clear.

**Decision:** Move the `get_status` check into the `spawn_blocking` closure. The closure returns `Result<(), String>`. On the async thread: if the `.await` returns `Ok(())`, clear `pending_batch_phases`. If it returns `Err`, do NOT clear — preserving the pending list so the operation can be retried.

**Rationale:** Matches existing behavior exactly. In the current code, both the "no staged changes" and "commit succeeded" paths clear the list. The "commit failed" path returns `Err` before the clear statement executes.

**Consequences:** The state mutation is cleanly separated from the git I/O. The clearing contract is explicit: clear on success, preserve on error.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Larger match arms | `CompletePhase` and `BatchCommit` match arms grow to ~20-30 lines each | Consistency with `GetHeadSha`/`IsAncestor` pattern and visible spawn_blocking boundaries | All git operations in the actor loop follow the same structure |
| Data cloning | Extra `.clone()` calls for `PathBuf`, `String`, `Vec<PathBuf>` before each closure | Thread-safe owned data in `spawn_blocking` closures | Cloning small path/string data is negligible vs. subprocess I/O time |
| Non-cancellable git sequences | Once a `spawn_blocking` closure starts, the entire git sequence runs to completion even if the coordinator is dropped | Single closure simplicity, no multi-await error handling | Coordinator processes one command at a time; partial git operations (staged but not committed) are harmless |

---

## Alternatives Considered

### Alternative: Keep handler functions, make them async

**Summary:** Keep `handle_complete_phase` and `handle_batch_commit` as separate functions but convert them to `async fn` with internal `spawn_blocking`.

**How it would work:**
- Functions become `async fn handle_complete_phase(state: &mut CoordinatorState, ...) -> Result<(), String>`
- Each function internally uses `spawn_blocking` for its git calls
- Match arms remain simple: `let result = handle_complete_phase(&mut state, ...).await;`

**Pros:**
- Match arms stay concise
- Function encapsulation preserved

**Cons:**
- Inconsistent with `GetHeadSha`/`IsAncestor` which inline `spawn_blocking` in the match arm
- Hides the blocking boundary inside helper functions — less obvious which operations block
- Passing `&mut state` to an async function that also needs to clone data for `spawn_blocking` is awkward

**Why not chosen:** Consistency with the established pattern is more valuable than concise match arms. The tech research explicitly recommended the inline approach.

### Alternative: Two separate spawn_blocking closures for destructive/non-destructive

**Summary:** Branch on `is_destructive` before spawning, with separate closures for each path.

**How it would work:**
- `if is_destructive { spawn_blocking(stage + commit) } else { spawn_blocking(stage only) }`
- Each closure contains its own copy of the staging logic

**Pros:**
- Each closure is simple and self-contained
- No internal branching in closures

**Cons:**
- Duplicates staging logic (get_status → collect_orchestrator_paths → stage_paths) across two closures
- Maintenance risk: changes to staging must be applied in two places

**Why not chosen:** A single closure with internal branching on `is_destructive` avoids the duplication while remaining straightforward.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Clone overhead on very large path lists | Negligible performance impact | Very Low | Path lists are small (handful of files per phase); clone cost is dwarfed by git subprocess I/O |
| Coordinator dropped during spawn_blocking | Git operation completes but state mutation skipped | Very Low | Coordinator shutdown saves final state; incomplete phase can be retried |

---

## Integration Points

### Existing Code Touchpoints

- `orchestrator/src/coordinator.rs` — `run_coordinator` function: modify `CompletePhase` and `BatchCommit` match arms; remove `handle_complete_phase` and `handle_batch_commit` helper functions
- No other files modified

### External Dependencies

- `tokio::task::spawn_blocking` — Already used in the same file. No new dependency.

---

## Open Questions

_(None — design is straightforward pattern replication.)_

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

- **No human available:** Running autonomously. Decisions made based on PRD requirements and tech research findings.
- **Mode selection:** Using `light` mode — this is a straightforward pattern replication with one clear approach and no significant alternatives worth deep analysis.
- **ROI accepted at PRD level:** The PRD established that this change is warranted for consistency and correctness. The design does not re-evaluate the fundamental need — it focuses on how to implement the accepted change.
- **Existing behavior preserved:** Error semantics, state mutation behavior, and recovery paths match the current implementation exactly. No behavioral changes.

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Initial design draft | Full design following established spawn_blocking pattern, light mode |
| 2026-02-12 | Self-critique (7 agents) | Auto-fixed: BatchCommit clearing semantics clarified (clear only on Ok); single-closure approach adopted to avoid staging duplication; handler function fate made explicit (removed); non-destructive flow data ownership clarified; cancellation safety tradeoff documented |
