# SPEC: Snapshot Caching for Backlog Snapshot Optimization

**ID:** WRK-024
**Status:** Ready
**Created:** 2026-02-20
**PRD:** ./WRK-024_investigate-using-immutable-data-structures-for-backlog-snapshots-to-avoid-cloning_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

The coordinator actor deep-clones the entire `BacklogFile` on every `get_snapshot()` call. The scheduler currently calls `get_snapshot()` independently in multiple handler functions, creating redundant clones within a single tick. The Design phase evaluated three approaches (snapshot caching, `Arc<Vec>`, `imbl::Vector`) and recommended snapshot caching as the only cost-justified optimization at current scale (5-30 items).

This SPEC implements the Design's recommendation: cache one snapshot per tick in the main scheduler loop and pass it by reference to handler functions, reducing redundant `get_snapshot()` calls. The change is entirely within `src/scheduler.rs`. No data structures, external APIs, or other modules change.

## Approach

The implementation follows a two-level caching strategy:

1. **Main loop level:** The existing snapshot at line 607 (`let snapshot = coordinator.get_snapshot().await?;`) is already cached for action selection and filtering. Extend its use to `handle_promote()` by passing `&BacklogFile` instead of having the handler fetch its own copy.

2. **Task completion level:** Pre-fetch one snapshot at the top of `handle_task_completion()` and pass it to sub-handlers that read before mutating. Handlers that mutate-then-read (Type C) keep their own internal `get_snapshot()` calls at mutation boundaries.

### Handler Classification

Each handler is classified by its snapshot usage pattern:

| Type | Pattern | Snapshot Strategy | Handlers |
|------|---------|-------------------|----------|
| A | Read-before-mutate | Use passed `&BacklogFile` for initial read; handler mutates afterward but never reads back | `handle_subphase_complete`, `handle_phase_failed`, `handle_phase_blocked`, Cancelled branch |
| B | Read-then-mutate (no re-read) | Use passed `&BacklogFile` for initial read | `handle_promote` (Phase 1) |
| C | Mutate-then-read | Keep internal `get_snapshot()` at mutation boundaries | `handle_phase_success`, `handle_triage_success` (worklog read uses passed snapshot; post-mutation reads re-fetch), `process_merges`, `apply_triage_result` |
| N/A | Executor spawn | Independent `get_snapshot()` inside async task | `spawn_triage` (line 1585), phase executor (line 873) |

**Note:** Type A and B handlers both read the snapshot before any mutations. The distinction is that Type B (`handle_promote`) is handled in Phase 1 (main loop path), while Type A handlers are in Phase 2 (task completion path). In all cases, the passed `&BacklogFile` is only read before any `coordinator.*` mutation calls, so the snapshot is guaranteed fresh for those reads.

**Patterns to follow:**

- `src/scheduler.rs:149-282` — `select_actions(snapshot: &BacklogFile, ...)` demonstrates the existing reference-passing pattern for snapshot consumers
- `src/scheduler.rs:287-388` — Pure functions like `sorted_ready_items(items: &[BacklogItem])` show the convention for slice-based read-only access
- `src/filter.rs` — `apply_filters(criteria: &[FilterCriterion], backlog: &BacklogFile)` accepts `&BacklogFile` by reference

**Implementation boundaries:**

- Do not modify: `src/coordinator.rs`, `src/types.rs`, `src/backlog.rs`, `src/filter.rs`, `src/prompt.rs`, `src/executor.rs`, `src/preflight.rs`
- Do not change: `CoordinatorHandle` API, `BacklogFile` struct, `BacklogItem` struct
- Do not modify: Executor spawn closures (lines 871-943, 1584-1618) — executors need independent snapshots

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Main Loop & Promote Caching | Low | Pass tick snapshot by reference to `handle_promote()`, removing its redundant `get_snapshot()` call |
| 2 | Task Completion Snapshot Sharing | Low | Pre-fetch one snapshot in `handle_task_completion()` and pass to Type A/B handlers, removing their redundant `get_snapshot()` calls |

**Ordering rationale:** Phase 1 targets the main loop dispatch path (simplest change, no mutation concerns). Phase 2 targets the task completion path (requires handler classification audit, but still mechanical signature changes).

---

## Phases

---

### Phase 1: Main Loop & Promote Caching

> Pass tick snapshot by reference to `handle_promote()`, removing its redundant `get_snapshot()` call

**Phase Status:** complete

**Complexity:** Low

**Goal:** Eliminate the redundant `get_snapshot()` clone inside `handle_promote()` by passing the main loop's existing cached snapshot by reference.

**Files:**

- `src/scheduler.rs` — modify — Add `snapshot: &BacklogFile` parameter to `handle_promote()`; remove internal `get_snapshot()` call; update call site at line 809

**Patterns:**

- Follow `select_actions(snapshot: &BacklogFile, ...)` at line 149 for the parameter-first convention

**Tasks:**

- [x] Add `snapshot: &BacklogFile` as first parameter to `handle_promote()` (line 1513)
- [x] Remove `let snapshot = coordinator.get_snapshot().await?;` from `handle_promote()` body (line 1518)
- [x] Update `handle_promote()` call site at line 809 to pass `&snapshot`
- [x] Verify `handle_promote()` compiles — the existing `snapshot.items.iter().find(...)` at line 1519 should work unchanged since it already borrows the local `snapshot`
- [x] Run `cargo build` to confirm no compile errors
- [x] Run `cargo test` to confirm no regressions

**Verification:**

- [x] `cargo build` succeeds
- [x] `cargo test` passes (all existing tests — 177 passed, 0 failed)
- [x] `handle_promote()` signature includes `snapshot: &BacklogFile` parameter
- [x] No `get_snapshot()` call remains inside `handle_promote()` body
- [x] Executor spawn closures (lines 871-943, 1584-1618) are unchanged
- [x] Code review passes — Ready to Merge verdict, no critical/high issues

**Commit:** `[WRK-024][P1] Clean: pass tick snapshot to handle_promote by reference`

**Notes:**

`handle_promote()` is a Type B handler (read-then-mutate): it reads the item's pipeline info from the snapshot, then issues three `coordinator.update_item()` calls. It never reads back post-mutation state, so the cached tick snapshot is always valid for its reads. The borrow of `&snapshot` does not cross any await points that conflict with coordinator mutations — the snapshot is read at line 1519, then only `coordinator` is used for mutations afterward.

**Followups:**

---

### Phase 2: Task Completion Snapshot Sharing

> Pre-fetch one snapshot in `handle_task_completion()` and pass to Type A/B handlers, removing their redundant `get_snapshot()` calls

**Phase Status:** complete

**Complexity:** Low

**Goal:** Pre-fetch a single snapshot at the top of `handle_task_completion()` and pass it to handlers that read before mutating (Type A/B), eliminating up to 4 redundant `get_snapshot()` clones per task completion.

**Files:**

- `src/scheduler.rs` — modify — Add snapshot pre-fetch in `handle_task_completion()`; add `snapshot: &BacklogFile` parameter to Type A/B handlers; remove their internal `get_snapshot()` calls; add `snapshot: &BacklogFile` parameter to `handle_triage_success()` for its initial worklog read; add snapshot freshness contract comments

**Tasks:**

- [x] Add `let snapshot = coordinator.get_snapshot().await?;` at the top of `handle_task_completion()` (line 1067, before the `match`)
- [x] Add `snapshot: &BacklogFile` parameter to `handle_subphase_complete()` (line 1228)
- [x] Remove `let snapshot = coordinator.get_snapshot().await?;` from `handle_subphase_complete()` body (line 1247)
- [x] Update `handle_subphase_complete()` call site in `handle_task_completion()` to pass `&snapshot`
- [x] Add `snapshot: &BacklogFile` parameter to `handle_phase_failed()` (line 1283)
- [x] Remove `let snapshot = coordinator.get_snapshot().await?;` from `handle_phase_failed()` body (line 1293)
- [x] Update `handle_phase_failed()` call site in `handle_task_completion()` to pass `&snapshot`
- [x] Add `snapshot: &BacklogFile` parameter to `handle_phase_blocked()` (line 1312)
- [x] Remove `let snapshot = coordinator.get_snapshot().await?;` from `handle_phase_blocked()` body (line 1322)
- [x] Update `handle_phase_blocked()` call site in `handle_task_completion()` to pass `&snapshot`
- [x] Replace inline `coordinator.get_snapshot().await?` in the Cancelled branch (line 1103) with the pre-fetched `snapshot`
- [x] Add `snapshot: &BacklogFile` parameter to `handle_triage_success()` (line 1444) for its initial worklog read at line 1464
- [x] Remove the first `let snapshot = coordinator.get_snapshot().await?;` from `handle_triage_success()` (line 1464) — use passed snapshot for worklog
- [x] Keep the second `let snapshot = coordinator.get_snapshot().await?;` in `handle_triage_success()` (line 1501) — this reads post-mutation state after `apply_triage_result()`
- [x] Update `handle_triage_success()` call site in `handle_task_completion()` to pass `&snapshot`
- [x] Do NOT add snapshot parameter to `handle_phase_success()` — it mutates (assessments, follow-ups) before its first read (line 1156), so the pre-fetched snapshot would be useless
- [x] Do NOT modify `process_merges()` or `apply_triage_result()` — they have internal mutate-then-read loops that require fresh snapshots
- [x] Verify `drain_join_set()` still compiles — it also calls `handle_task_completion()`, but since the pre-fetch is added inside the function body (not changing its external signature), no call-site updates are needed. Confirm this explicitly.
- [x] Add comment block at top of `handle_task_completion()` documenting the snapshot freshness contract (updated from SPEC template to accurately reflect hybrid triage_success pattern per code review)
- [x] Run `cargo build` to confirm no compile errors
- [x] Run `cargo test` to confirm no regressions

**Verification:**

- [x] `cargo build` succeeds
- [x] `cargo test` passes (all existing tests — 660 passed, 0 failed)
- [x] `handle_task_completion()` has exactly one `coordinator.get_snapshot().await?` call at its top
- [x] `handle_subphase_complete()`, `handle_phase_failed()`, `handle_phase_blocked()` have zero `get_snapshot()` calls in their bodies
- [x] `handle_triage_success()` has exactly one internal `get_snapshot()` call (line 1526 for post-mutation check), down from two
- [x] `handle_phase_success()` signature is unchanged (still fetches its own snapshot at line 1181)
- [x] `process_merges()` and `apply_triage_result()` are unchanged
- [x] Executor spawn closures (lines 871-943, 1610-1618) are unchanged
- [x] Snapshot freshness contract comment is present at top of `handle_task_completion()` and accurately lists all handlers as implemented
- [x] `drain_join_set()` compiles without changes (it calls `handle_task_completion()` whose external signature is unchanged)
- [x] Total `get_snapshot()` calls in `scheduler.rs` reduced from 12 to 8 (main loop, executor spawn x2, handle_task_completion pre-fetch, handle_phase_success, handle_triage_success post-mutation, process_merges, apply_triage_result)
- [x] Code review passes — Ready to Merge verdict after fixing freshness contract comment accuracy

**Commit:** `[WRK-024][P2] Clean: share pre-fetched snapshot across task completion handlers`

**Notes:**

Handler audit results:

| Handler | Line | Read Before Mutate? | Mutate Then Read? | Strategy |
|---------|------|--------------------|--------------------|----------|
| `handle_subphase_complete` | 1228 | Yes (worklog at 1247) | No | Use passed `&snapshot` |
| `handle_phase_failed` | 1283 | Yes (worklog at 1293) | No | Use passed `&snapshot` |
| `handle_phase_blocked` | 1312 | Yes (worklog at 1322) | No | Use passed `&snapshot` |
| Cancelled branch | 1101 | Yes (worklog at 1103) | No | Use pre-fetched snapshot |
| `handle_triage_success` | 1444 | Yes (worklog at 1464) | Yes (post-triage at 1501) | Passed for worklog; re-fetch at 1501 |
| `handle_phase_success` | 1123 | No | Yes (transition at 1156) | Unchanged — manages own snapshot |

**Design audit closure:** The Design flagged `apply_triage_result()` (line 1676) as "Needs audit — may need fresh data after triage mutations." Audit result: confirmed as Type C. `apply_triage_result()` calls `coordinator.update_item()` for assessments, description, and pipeline_type (lines 1630-1671), then calls `get_snapshot()` at line 1676 to read the post-mutation item state for routing decisions (size/risk checks, pipeline pre-phase detection). This is a necessary post-mutation re-fetch and must remain.

**Wasted pre-fetch tradeoff:** The snapshot pre-fetch at the top of `handle_task_completion()` runs unconditionally, including on the `handle_phase_success` path which does not use it. This is an accepted tradeoff: at 5-30 items, one wasted clone costs ~2-5us, and the alternative (moving the pre-fetch inside individual match arms) would add code complexity for negligible savings. The net clone reduction remains positive across all scenarios.

**`handle_promote` cross-reference:** `handle_promote()` is handled in Phase 1, not in this phase. It is called from the main loop action dispatch (line 809), not from `handle_task_completion()`.

Lifetime safety: The `&snapshot` borrow in Type A handlers does not cross any problematic await boundaries. In each handler, the snapshot is read synchronously (`.items.iter().find()`), the found item is `.clone()`d for the worklog call, and then the borrow is no longer active when `coordinator.write_worklog().await` executes.

**Followups:**

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met (recommendation delivered via Design doc; this SPEC implements the recommended approach)
- [x] Tests pass (660 passed, 0 failed)
- [x] No regressions introduced
- [x] Code reviewed — Ready to Merge verdict

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| Phase 1: Main Loop & Promote Caching | complete | `[WRK-024][P1] Clean: pass tick snapshot to handle_promote by reference` | All tasks done, code review passed (Ready to Merge) |
| Phase 2: Task Completion Snapshot Sharing | complete | `[WRK-024][P2] Clean: share pre-fetched snapshot across task completion handlers` | All tasks done, code review fixed comment accuracy, Ready to Merge |

## Followups Summary

### Critical

### High

### Medium

- Re-evaluate snapshot strategy if backlog size exceeds 100 items or executor spawn frequency exceeds 8/tick. At that scale, consider layering `Arc<Vec<BacklogItem>>` for O(1) snapshot distribution. See Design doc "Tradeoffs Accepted" table for re-evaluation triggers.
- Add unit tests for individual handler functions (currently untested in isolation). Handler functions are private, so testing requires either `pub(crate)` visibility or testing through `run_scheduler()` integration tests. Key coverage gaps: `handle_promote()`, `handle_subphase_complete()`, `handle_phase_failed()`, `handle_phase_blocked()`, and the Cancelled branch in `handle_task_completion()`. The Cancelled branch in particular has zero test coverage.

### Low

## Design Details

### Architecture Details

**Snapshot flow after implementation:**

```
Scheduler tick
  -> get_snapshot() [clone #1, stored as tick_snapshot]
  -> apply_filters(&tick_snapshot)       [zero-cost borrow]
  -> select_actions(&tick_snapshot)      [zero-cost borrow]
  -> handle_promote(&tick_snapshot, ...) [zero-cost borrow, no internal clone]
  -> spawn_triage/spawn_phase(...)      [executor gets independent clone]
  -> join_set.join_next()
     -> handle_task_completion()
        -> get_snapshot() [clone #2, pre-fetched for handlers]
        -> handler(&snapshot, ...)      [zero-cost borrow, no internal clone]
           -> [Type C only] get_snapshot() [clone #3, post-mutation re-fetch]
```

**Clone count comparison per tick:**

| Scenario | Before | After | Savings |
|----------|--------|-------|---------|
| Tick with promote, no task completion | 2 (main + promote) | 1 (main) | 1 clone |
| Tick with task completion (Type A/B handler) | 2-3 (main + promote + handler) | 2 (main + completion pre-fetch) | 1 clone |
| Tick with task completion (Type C handler) | 3-5 (main + promote + handler + re-fetches) | 2-4 (main + completion pre-fetch + re-fetches) | 1 clone |
| Tick with triage completion | 4-6 (main + promote + worklog + merges + post-triage + routing) | 3-5 (main + completion pre-fetch + merges + post-triage + routing) | 1 clone |

Net savings: 1-2 redundant `get_snapshot()` clones eliminated per tick.

### Design Rationale

**Why not pass tick_snapshot to task completion handlers?**

The tick_snapshot (line 607) is taken at the start of the loop iteration, before executors complete. By the time `handle_task_completion()` runs, the backlog may have been mutated by `handle_promote()` or by the executor itself (via coordinator commands). The pre-fetched snapshot in `handle_task_completion()` reflects the latest state at the moment the handler runs, which is what Type A/B handlers need for accurate worklog entries.

**Why not change handle_phase_success?**

`handle_phase_success()` does not read from any snapshot before its mutations. Its first action is `coordinator.update_item()` for assessment updates, followed by `ingest_follow_ups()`. Only then does it call `get_snapshot()` (line 1156) to read the post-mutation item state for transition resolution. A pre-fetched snapshot would be immediately stale after the first mutation, so passing it would add parameter complexity without eliminating any clone.

**Why not change process_merges or apply_triage_result?**

Both functions have internal mutate-then-read loops. `process_merges()` calls `get_snapshot()` inside a loop (line 1364) to validate each duplicate before merging. `apply_triage_result()` calls `get_snapshot()` at line 1676 to read post-mutation item state for routing decisions. These internal re-fetches are necessary for correctness and cannot be replaced with a cached snapshot.

## Assumptions

*Decisions made without human input during autonomous SPEC creation:*

- **Mode selection:** Used `light` mode (2 phases, minimal iteration) based on item assessments (small size, low complexity, low risk). The Design already resolved all architectural questions; the SPEC is a mechanical translation of the Design's recommendations into implementation tasks.
- **Handler classification audit:** Classified all handlers by reading the source code. The Design deferred this audit to the SPEC phase ("The SPEC phase must audit each handler to confirm this"). Classifications are documented in Phase 2 notes.
- **handle_phase_success excluded:** Decided not to pass a snapshot to `handle_phase_success()` because it mutates before its first read. This deviates from the Design's call site catalog which marked line 1156 as "Can Use Cached: Yes" — but the code audit shows the handler needs post-mutation data at that point.
- **Two-level caching:** The Design describes a single cached snapshot per tick. The SPEC implements two cache levels (tick_snapshot for main loop + completion_snapshot for task completion) because the tick_snapshot is stale by the time task completion handlers run.
- **No new tests added:** The existing test suite covers `select_actions()` and related pure functions. The signature changes in this SPEC are mechanical and verified by compilation + existing tests. All handler functions are private (`async fn`, not `pub async fn`), so they can only be tested indirectly through `run_scheduler()` integration tests. Adding handler-level unit tests is tracked as a medium-priority follow-up.
- **PRD criteria traceability:** Several PRD success criteria were resolved at the Design phase, not the SPEC phase: (a) Serde gate — dismissed as not applicable since no data structure changes are made (Design Open Questions section); (b) Full cross-codebase call site catalog — narrowed to scheduler.rs since the recommended approach only affects snapshot consumers, not `Vec<BacklogItem>` mutations or slice APIs; (c) Benchmarks (Should Have) — the Design uses estimates rather than measured benchmarks; actual measurement remains a potential follow-up if scale increases. The Design document serves as the "written recommendation document" required by the PRD.

---

## Retrospective

### What worked well?

- Handler classification audit in SPEC design phase made implementation mechanical — each handler's strategy was pre-determined, so coding was just following the plan.
- Two-phase approach kept each change small and independently verifiable.
- Code review caught an inaccurate comment that would have misled future readers about the `handle_triage_success` hybrid pattern.

### What was harder than expected?

- Nothing was unexpectedly difficult. The implementation was straightforward mechanical refactoring as predicted by the SPEC.

### What would we do differently next time?

- The SPEC's freshness contract comment template should have accounted for the hybrid `handle_triage_success` pattern from the start, rather than oversimplifying into two clean categories. The handler audit table already documented the hybrid behavior, but the comment template didn't reflect it.
