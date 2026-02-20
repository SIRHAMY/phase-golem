# SPEC: Pre-build HashMap for O(1) Dependency Lookups in Scheduler

**ID:** WRK-035
**Status:** Ready
**Created:** 2026-02-20
**PRD:** ./WRK-035_pre-build-hashmap-for-o-1-dependency-lookups-in-scheduler_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

The scheduler's dependency-checking and target-lookup functions resolve item IDs via linear scans (`.iter().find()`). While functional at current backlog sizes, this is O(n) per lookup and scales quadratically across all candidate evaluations in `select_actions`. This change replaces those scans with O(1) HashMap lookups, built once per scheduling cycle from the snapshot's items slice.

The codebase already uses HashMap patterns extensively (e.g., `&HashMap<String, PipelineConfig>` threading through scheduler functions) and has established lifetime annotation patterns in the `sorted_*_items` helpers, making this a mechanical signature refactoring.

## Approach

Add a single new helper function `build_item_lookup` that constructs a `HashMap<&str, &BacklogItem>` from an items slice. Thread this map through 6 existing function signatures, replacing `.iter().find()` with `.get()` at each lookup site. Update test call sites to construct and pass the map.

The change follows the existing codebase pattern of building utility values from the snapshot at the top of scheduling functions and passing them by reference to callees. Each top-level dispatch function (`select_actions`, `select_targeted_actions`) builds its own map internally. The targets branch in `run_scheduler` builds a separate map for `advance_to_next_active_target`, and the diagnostic logging block builds another (only on idle cycles). The dispatch functions are in a three-way dispatch, so at most one dispatch map is built per cycle; combined with the targets-branch and diagnostic maps, at most three maps are built per cycle.

**Patterns to follow:**

- `src/scheduler.rs:304` — `sorted_in_progress_items<'a>()` for lifetime annotation pattern on borrowed slice helpers
- `src/scheduler.rs:287` — `sorted_ready_items()` for slice-processing utility placement
- `src/scheduler.rs:153` — `&HashMap<String, PipelineConfig>` parameter threading pattern

**Implementation boundaries:**

- Do not modify: task-completion handlers (`handle_task_completion`, `handle_phase_success`, etc.) — they operate on freshly re-loaded snapshots, not the scheduling-cycle snapshot
- Do not modify: status-filtering scans in `sorted_*_items` helpers (these filter by status, not by ID)
- Do not add: convenience wrappers or backward-compatible overloads for the changed signatures

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | HashMap lookup refactoring | Low | Add `build_item_lookup` helper, update all 6 function signatures, update all call sites and tests |

**Ordering rationale:** All changes are in a single phase because the signature change to `unmet_dep_summary` (a `pub` function) makes intermediate states non-compiling. The function is called from both internal callers and `tests/scheduler_test.rs`, so all changes must land atomically.

---

## Phases

### Phase 1: HashMap lookup refactoring

> Add `build_item_lookup` helper, refactor all linear-scan lookups to use HashMap, update tests

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Replace all in-scope `.iter().find()` ID lookups with `HashMap::get()`, built from a shared helper function, while maintaining identical behavior.

**Files:**

- `src/scheduler.rs` — modify — Add `build_item_lookup` helper (~line 348, after sorting helpers); change signatures and bodies of `unmet_dep_summary` (line 358), `skip_for_unmet_deps` (line 382), `advance_to_next_active_target` (line 469, replacing `snapshot` param with `item_lookup`); update callers in `select_actions` (line 149), `select_targeted_actions` (line 1000), diagnostic logging block (line 772), and `advance_to_next_active_target` call site (line 657)
- `tests/scheduler_test.rs` — modify — Add `build_item_lookup` to import; update 4 `unmet_dep_summary` test call sites (~lines 1798, 1808, 1824, 1852); update 8 `advance_to_next_active_target` test call sites (~lines 1873, 1891, 1905, 1922, 1933, 1950, 1965, 1984) replacing `&snapshot` with `&item_lookup`; 3 `select_targeted_actions` test call sites require NO updates

**Patterns:**

- Follow `src/scheduler.rs:304` (`sorted_in_progress_items<'a>`) for the `<'a>` lifetime annotation on the helper
- Follow existing `&HashMap<String, PipelineConfig>` parameter threading for how to pass the map

**Tasks:**

- [ ] Add `pub fn build_item_lookup<'a>(items: &'a [BacklogItem]) -> HashMap<&'a str, &'a BacklogItem>` after the sorting helpers in `scheduler.rs`, using `items.iter().map(|i| (i.id.as_str(), i)).collect()`
- [ ] Change `unmet_dep_summary` signature: `all_items: &[BacklogItem]` → `item_lookup: &HashMap<&str, &BacklogItem>`; replace `all_items.iter().find(|i| i.id == *dep_id)` with `item_lookup.get(dep_id.as_str())`
- [ ] Change `skip_for_unmet_deps` signature: same substitution; update pass-through call to `unmet_dep_summary`
- [ ] In `select_actions`: add `let item_lookup = build_item_lookup(&snapshot.items);` immediately before `let mut actions: Vec<SchedulerAction> = Vec::new();` (after both early returns at lines ~157 and ~165); replace `&snapshot.items` with `&item_lookup` in all 4 `skip_for_unmet_deps` calls (lines ~186, 204, 218, 232)
- [ ] In `select_targeted_actions`: add `let item_lookup = build_item_lookup(&snapshot.items);`; replace `snapshot.items.iter().find(|i| i.id == target_id)` with `item_lookup.get(target_id).copied()`; replace `&snapshot.items` with `&item_lookup` in `skip_for_unmet_deps` call
- [ ] Change `advance_to_next_active_target` signature: replace `snapshot: &BacklogFile` parameter with `item_lookup: &HashMap<&str, &BacklogItem>`; replace `snapshot.items.iter().find(|i| i.id == *target)` with `item_lookup.get(target.as_str()).copied()`
- [ ] Update `advance_to_next_active_target` call site in `run_scheduler` (~line 657): build `let item_lookup = build_item_lookup(&snapshot.items);` immediately before the call (scoped locally to the targets branch), and pass `&item_lookup` in place of `&snapshot`
- [ ] In diagnostic logging block (~line 772): add `let item_lookup = build_item_lookup(&snapshot.items);` **inside** the `if actions.is_empty() && running.is_empty()` conditional block, before the `.filter_map()` closure; replace `unmet_dep_summary(i, &snapshot.items)` with `unmet_dep_summary(i, &item_lookup)`. Note: the closure captures `&item_lookup` while `.iter()` borrows `&snapshot.items` — this is safe because both are shared (`&`) borrows
- [ ] Update `tests/scheduler_test.rs` import to add `build_item_lookup`
- [ ] Update 4 `unmet_dep_summary` test call sites: extract items into a named `Vec`, build `let lookup = build_item_lookup(&items);`, pass `&lookup` instead of `&[...]`
- [ ] Update 8 `advance_to_next_active_target` test call sites: build `let item_lookup = build_item_lookup(&snapshot.items);` from existing fixture snapshot, pass `&item_lookup` in place of `&snapshot` (the `snapshot` parameter is replaced, not added alongside)
- [ ] Confirm: the 3 `select_targeted_actions` test call sites (~lines 1742, 1761, 1779) require NO updates — that function's external signature is unchanged (the map is built internally)

**Verification:**

- [ ] `cargo build` succeeds with no errors or warnings
- [ ] `cargo test` passes — all existing scheduler tests pass with identical behavior
- [ ] `cargo clippy` reports no new warnings
- [ ] Grep confirms no remaining `.iter().find(|i| i.id ==` patterns within the bodies of the 4 in-scope functions (`unmet_dep_summary`, `skip_for_unmet_deps`, `advance_to_next_active_target`, `select_targeted_actions`). Note: ~10 matches in out-of-scope task-completion handlers (lines ~885, 1130, 1274–1620) are expected and correct — do not modify those
- [ ] Confirm `build_item_lookup` appears in the `use phase_golem::scheduler::{...}` import in `tests/scheduler_test.rs`

**Commit:** `[WRK-035][P1] Clean: Replace linear-scan ID lookups with HashMap in scheduler`

**Notes:**

- `HashMap::get()` returns `Option<&&BacklogItem>` when the value type is `&BacklogItem`. Use `.copied()` at call sites that need `Option<&BacklogItem>` (e.g., `select_targeted_actions` target lookup, `advance_to_next_active_target` target lookup). In `unmet_dep_summary`, the `match` on `.get()` binds `Some(dep_item)` where `dep_item: &&BacklogItem` — field access like `dep_item.status` works via Rust's auto-deref, so no `.copied()` is needed there.
- **Build order:** Complete all production code changes in `scheduler.rs` first (verify with `cargo build`) before updating tests. This creates one natural checkpoint since intermediate states with only some signatures changed will not compile.
- The diagnostic logging HashMap must be built from the full `snapshot.items`, not `filtered_snapshot`, since dependency resolution needs visibility into all items regardless of filter status.
- `build_item_lookup` must be `pub` (not just `pub(crate)`) because `tests/scheduler_test.rs` is an integration test (in the `tests/` directory), which sits outside the crate.

**Followups:**

(None)

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

(None)

### High

(None)

### Medium

(None)

### Low

(None)

## Design Details

### Key Types

```rust
/// Build a borrowed HashMap from an items slice for O(1) ID lookups.
pub fn build_item_lookup<'a>(items: &'a [BacklogItem]) -> HashMap<&'a str, &'a BacklogItem> {
    items.iter().map(|i| (i.id.as_str(), i)).collect()
}
```

No new types are introduced. The change uses `HashMap<&str, &BacklogItem>` with lifetimes tied to the source items slice.

### Architecture Details

The HashMap is built at the top of each top-level scheduling function and threaded downward by shared reference:

```
build_item_lookup(&snapshot.items) -> HashMap<&str, &BacklogItem>
    │
    ├─ select_actions() builds map, passes &map to:
    │       skip_for_unmet_deps() → unmet_dep_summary()
    │
    ├─ select_targeted_actions() builds map, uses map.get() + passes &map to:
    │       skip_for_unmet_deps() → unmet_dep_summary()
    │
    ├─ run_scheduler() targets branch builds map, passes &map to:
    │       advance_to_next_active_target() uses map.get()
    │
    └─ run_scheduler() diagnostic logging builds map, passes &map to:
            unmet_dep_summary()
```

At most three maps are built per cycle: one from the dispatch branch (`select_actions` or `select_targeted_actions`), one for `advance_to_next_active_target` in the targets branch (only when targets are active), and one from the diagnostic logging block (only on idle cycles). In practice, the targets branch and dispatch are in the same code path, so 2–3 maps are built depending on the scheduling mode.

### Design Rationale

- **Direct references over indices:** `HashMap<&str, &BacklogItem>` avoids extra indirection and allocation vs. `HashMap<String, usize>`. Lifetime propagation is minimal since no function returns borrowed data from the map.
- **`.collect()` over explicit loop:** More concise and idiomatic. Capacity optimization is negligible at current backlog sizes (tens of items).
- **Single helper function:** Avoids duplicating map construction across 3 build sites. Groups naturally with existing sorting helpers.
- **`pub` visibility:** Required for integration test access. Only external caller is `tests/scheduler_test.rs`.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
