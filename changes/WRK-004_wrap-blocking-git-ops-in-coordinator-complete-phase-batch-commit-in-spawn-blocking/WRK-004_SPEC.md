# SPEC: Wrap Blocking Git Ops in spawn_blocking

**ID:** WRK-004
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-004_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

The coordinator actor loop (`run_coordinator` in `coordinator.rs`) processes commands sequentially. Two commands — `CompletePhase` and `BatchCommit` — call synchronous git functions (`get_status`, `stage_paths`, `commit`) directly on the async executor thread, blocking the tokio runtime. Two other commands — `GetHeadSha` and `IsAncestor` — already correctly wrap their git calls in `tokio::task::spawn_blocking`. This change brings the remaining two commands into alignment with the established pattern.

## Approach

Inline the git I/O from `handle_complete_phase` and `handle_batch_commit` into `spawn_blocking` closures directly in the `CompletePhase` and `BatchCommit` match arms of `run_coordinator`, following the exact pattern used by `GetHeadSha` (lines 556-563) and `IsAncestor` (lines 565-572). Remove the `handle_complete_phase` and `handle_batch_commit` helper functions entirely. State mutations (`pending_batch_phases.push`/`.clear()`) remain on the async executor thread, outside the closures, after the `.await`.

The `CompletePhase` closure uses a single `spawn_blocking` call that branches internally on `is_destructive`: both paths share the staging logic (get_status → collect_orchestrator_paths → stage_paths), and the destructive path additionally builds a commit message and commits. The `BatchCommit` closure handles get_status → optional commit in one `spawn_blocking` call.

**Patterns to follow:**

- `orchestrator/src/coordinator.rs` lines 556-563 (`GetHeadSha` match arm) — the exact `spawn_blocking` + `.await` + `.unwrap_or_else` + reply pattern to replicate
- `orchestrator/src/coordinator.rs` lines 565-572 (`IsAncestor` match arm) — same pattern with additional data cloned into closure

**Implementation boundaries:**

- Do not modify: `orchestrator/src/git.rs` — all git functions remain synchronous
- Do not modify: `CoordinatorHandle` public API — all method signatures unchanged
- Do not modify: test files — tests interact via the async API and should pass unchanged
- Do not create: a consolidated `spawn_blocking` helper function (only 2 call sites, not worth the abstraction)

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Wrap CompletePhase and BatchCommit in spawn_blocking | Low | Move git I/O into spawn_blocking closures, remove helper functions, keep state mutations on async thread |

**Ordering rationale:** Single phase — the entire change is in one file, affects two adjacent match arms, and removing the helper functions is tightly coupled to inlining their logic. Splitting into multiple phases would create intermediate states where some logic exists in both places.

---

## Phases

### Phase 1: Wrap CompletePhase and BatchCommit in spawn_blocking

> Move git I/O into spawn_blocking closures in the match arms, remove handler functions, keep state mutations on async thread

**Phase Status:** complete

**Complexity:** Low

**Goal:** All synchronous git operations in the coordinator actor loop use `spawn_blocking`, matching the existing `GetHeadSha`/`IsAncestor` pattern. Helper functions `handle_complete_phase` and `handle_batch_commit` are removed.

**Files:**

- `orchestrator/src/coordinator.rs` — modify — refactor `CompletePhase` and `BatchCommit` match arms, delete `handle_complete_phase` and `handle_batch_commit` functions

**Patterns:**

- Follow `coordinator.rs` lines 556-563 (`GetHeadSha`) for the spawn_blocking + await + unwrap_or_else pattern
- Follow `coordinator.rs` lines 565-572 (`IsAncestor`) for cloning multiple values into the closure

**Tasks:**

- [x] Refactor `CompletePhase` match arm (lines 536-551):
  - Clone all owned data before spawning: `project_root`, `backlog_path`, `output_paths`, `is_destructive`, plus clone `item_id` and `result.phase` twice — one copy for the closure, one for potential `pending_batch_phases.push` after `.await`
  - Use a single `spawn_blocking` closure that:
    1. Calls `git::get_status` → `collect_orchestrator_paths` → `git::stage_paths`
    2. If `is_destructive`: calls `build_phase_commit_message` → `git::get_status` (re-check for staged changes after staging, preserving existing behavior) → conditionally `git::commit`
    3. Returns `Result<(), String>`
  - `.await` the JoinHandle with `.unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {}", e)))`
  - After `.await`: if `!is_destructive && result.is_ok()`, push `(item_id, phase)` to `state.pending_batch_phases`
  - Send result via reply channel
- [x] Refactor `BatchCommit` match arm (lines 552-554):
  - Early return `Ok(())` if `state.pending_batch_phases.is_empty()` (no `spawn_blocking` needed)
  - Clone `project_root` and `pending_batch_phases` for the closure
  - Use a single `spawn_blocking` closure that:
    1. Calls `git::get_status`
    2. If `has_staged_changes`: builds batch commit message via `build_batch_commit_message`, calls `git::commit`
    3. Returns `Result<(), String>` — `Ok(())` for both "no staged changes" and "commit succeeded"
  - `.await` the JoinHandle with `.unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {}", e)))`
  - After `.await`: if result is `Ok(())`, clear `state.pending_batch_phases`. If `Err`, do NOT clear (preserves for retry)
  - Send result via reply channel
- [x] Delete `handle_complete_phase` function (lines 381-417)
- [x] Delete `handle_batch_commit` function (lines 419-435)
- [x] Run `cargo build --all-targets` — verify compilation succeeds with no errors or warnings
- [x] Run `cargo test` — verify all existing tests pass unchanged

**Verification:**

- [x] `cargo build --all-targets` succeeds with no errors or warnings
- [x] `cargo test` passes — all existing tests pass unchanged (specifically: `complete_phase_destructive_commits_immediately`, `complete_phase_non_destructive_stages_only`, `batch_commit_commits_staged_phases`, `batch_commit_noop_when_nothing_staged`)
- [x] No new `unwrap()` calls — all `JoinError` handling uses `.unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {}", e)))`
- [x] `handle_complete_phase` and `handle_batch_commit` functions no longer exist
- [x] `CompletePhase` and `BatchCommit` match arms follow the same structural pattern as `GetHeadSha` and `IsAncestor`
- [x] State mutations (`pending_batch_phases.push`/`.clear()`) are NOT inside any `spawn_blocking` closure
- [x] Error results (from git failures or `spawn_blocking` panics) are sent through the reply channel, not silently dropped
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-004][P1] Fix: Wrap CompletePhase and BatchCommit git ops in spawn_blocking`

**Notes:**

Key implementation detail for `CompletePhase`: The `item_id` and `result.phase` values are needed in two different contexts depending on `is_destructive`:
- **Destructive:** Needed inside the closure for `build_phase_commit_message` — must be cloned/moved into the closure
- **Non-destructive:** Needed outside the closure for `pending_batch_phases.push` — must stay on the async thread

Since the closure uses `move`, the simplest approach is to clone `item_id` and `result.phase` before spawning: one copy goes into the closure (for the commit message), the other stays for the potential push. On the non-destructive path, the closure doesn't use the copies, but the cloning cost is negligible (short strings, dwarfed by subprocess I/O).

**Followups:**

_(None)_

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
| 1 | Complete | `[WRK-004][P1] Fix: Wrap CompletePhase and BatchCommit git ops in spawn_blocking` | All 322 tests pass, code review passed |

## Followups Summary

### Critical

_(None)_

### High

_(None)_

### Medium

_(None)_

### Low

_(None)_

## Design Details

### Architecture Details

No architectural changes. The coordinator actor loop structure remains identical. The only change is where git subprocess calls execute:

```
Before:
  Actor Loop (async thread) → git::get_status / stage_paths / commit (blocks async thread)

After:
  Actor Loop (async thread) → spawn_blocking → git operations (blocking thread pool)
                             ← .await result  ← return Result
  Actor Loop (async thread) → state mutation (pending_batch_phases)
```

### Design Rationale

- **Single closure per command:** Avoids duplicating staging logic across two closures for the destructive/non-destructive branch. One closure branches internally on `is_destructive`.
- **Inline in match arms:** Matches `GetHeadSha`/`IsAncestor` pattern. Makes it visually obvious at the match level which operations block and which don't.
- **No consolidated helper:** Only 2 call sites. A helper would need generics and adds indirection for minimal deduplication (the `.unwrap_or_else` pattern is 2 lines).
- **Clone data, don't share:** `spawn_blocking` closures must own their data. Cloning `PathBuf`/`String` values is negligible vs. git subprocess I/O time.

## Assumptions

- **No human available:** Running autonomously as part of the orchestrated changes workflow.
- **Mode selection:** Using `light` mode — this is a straightforward pattern replication with one file to modify and a clear pattern to follow.
- **Existing test coverage sufficient:** The existing tests for `complete_phase` and `batch_commit` exercise the relevant code paths via the async `CoordinatorHandle` API. No new tests are needed since the behavioral contract is unchanged.

## Retrospective

### What worked well?

- Clear SPEC with exact line references and pattern examples made implementation straightforward
- Single-file change with well-defined boundaries minimized risk
- Existing test suite provided immediate confidence — all 322 tests passed without modification
- The established `GetHeadSha`/`IsAncestor` pattern was easy to replicate

### What was harder than expected?

- Nothing — this was a clean pattern replication as predicted by the SPEC

### What would we do differently next time?

- Nothing — the SPEC's assessment of low complexity was accurate
