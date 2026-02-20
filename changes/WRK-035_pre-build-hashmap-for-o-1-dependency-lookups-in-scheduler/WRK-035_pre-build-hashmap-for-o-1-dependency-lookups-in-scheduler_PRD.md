# Change: Pre-build HashMap for O(1) Dependency Lookups in Scheduler

**Status:** Proposed
**Created:** 2026-02-20
**Author:** phase-golem (autonomous)

## Problem Statement

The scheduler's dependency-checking functions (`unmet_dep_summary`, `skip_for_unmet_deps`) resolve dependency IDs by linearly scanning `all_items` with `.iter().find()` — an O(n) lookup per dependency. In `select_actions`, this scan runs for every candidate item across four loops (Ready promotion, InProgress phases, Scoping phases, New triage), making the overall cost O(items * deps * items) per scheduling cycle.

Similarly, `advance_to_next_active_target` and `select_targeted_actions` do linear scans to find items by ID.

While no performance issue has been observed at current backlog sizes (tens of items), the quadratic scaling is unnecessary. As backlogs grow, this becomes a latent bottleneck. The primary motivation is algorithmic correctness and future-proofing rather than addressing a measured performance problem.

## User Stories / Personas

- **Phase-golem operator** - Runs the scheduler with backlogs that may grow over time. Expects scheduling decisions to remain fast regardless of backlog size.

## Desired Outcome

When the scheduler evaluates dependency status or looks up items by ID, it uses O(1) lookups via a pre-built `HashMap<&str, &BacklogItem>` instead of scanning the full items slice. The HashMap is built once per scheduling cycle from the snapshot's items vec (the `BacklogFile` instance representing the current backlog state), then passed to functions that need item-by-ID lookup.

The concrete type is `HashMap<&'a str, &'a BacklogItem>` where both the key (borrowed from `item.id.as_str()`) and value borrow from the same `&'a [BacklogItem]` slice.

## Success Criteria

### Must Have

- [ ] Dependency lookups in `unmet_dep_summary` use a HashMap instead of linear scan
- [ ] `skip_for_unmet_deps` receives the HashMap and passes it through
- [ ] `select_actions` builds the HashMap once and reuses it across all candidate evaluations
- [ ] `select_targeted_actions` uses the HashMap for both the target-item lookup and dependency checking via `skip_for_unmet_deps`
- [ ] All existing scheduler tests pass with behavioral equivalence (test updates for signature changes are permitted)
- [ ] `advance_to_next_active_target` uses the HashMap for target item lookups
- [ ] Diagnostic logging in `run_scheduler` that calls `unmet_dep_summary` to report dependency-blocked items uses the HashMap (this call site must compile after the signature change)
- [ ] The HashMap borrows from the snapshot — no cloning of item data. Concrete type: `HashMap<&str, &BacklogItem>`

### Should Have

- [ ] The HashMap is built from a shared helper function (e.g., `build_item_lookup(items: &[BacklogItem]) -> HashMap<&str, &BacklogItem>`) to avoid duplication across call sites

### Nice to Have

- [ ] (None currently)

## Scope

### In Scope

- Refactoring `unmet_dep_summary` signature to accept a lookup map
- Refactoring `skip_for_unmet_deps` signature to accept a lookup map
- Updating `select_actions` and `select_targeted_actions` to build and pass the map (including the target-item lookup in `select_targeted_actions`)
- Updating `advance_to_next_active_target` to accept and use a map
- Updating the diagnostic logging call site in `run_scheduler` that calls `unmet_dep_summary`
- Updating `tests/scheduler_test.rs` for any signature changes to `unmet_dep_summary`

### Out of Scope

- Changing the `BacklogFile` struct to store a pre-built index (the HashMap is rebuilt each scheduling cycle and discarded afterward)
- Optimizing the ~10 other `.iter().find()` by-ID lookups in task-completion handlers (`handle_task_completion`, `handle_phase_success`, `handle_triage_success`, `spawn_triage`, etc.) — these operate on freshly re-loaded snapshots after async task completion, not the scheduling-cycle snapshot
- Performance benchmarking — the improvement is from O(n) to O(1) per lookup, which does not require empirical validation
- Optimizing status-filtering scans in `sorted_*_items` helpers (these filter by status, not by ID)

## Non-Functional Requirements

- **Performance:** Item-by-ID lookups go from O(n) to O(1). HashMap construction is O(n) once per cycle, amortized over all lookups.

## Constraints

- `unmet_dep_summary` is `pub` and called from `tests/scheduler_test.rs` — its signature will change, and tests will be updated in the same change.
- The HashMap must borrow from the items slice (lifetime-tied to the snapshot) to avoid unnecessary allocations.

## Dependencies

- **Depends On:** None
- **Blocks:** None

## Risks

- [ ] Signature change to `unmet_dep_summary` is a public API change — mitigated by updating tests in the same change. The only external caller is `tests/scheduler_test.rs`.

## Open Questions

(None — all questions resolved during drafting.)

## Assumptions

- **Mode: light** — This is an algorithmic refactoring with a clear problem and obvious solution. No discovery or deep exploration needed.
- Decided to include `advance_to_next_active_target` in scope since it has the same linear-scan pattern and benefits from the same HashMap.
- **Item ID uniqueness:** Item IDs in the backlog are unique. This is enforced by preflight validation (`validate_duplicate_ids`). The HashMap will contain one entry per item.
- **Snapshot immutability within a cycle:** The snapshot (`BacklogFile`) is loaded once per scheduling cycle and not mutated while the HashMap borrows from it. The Rust borrow checker enforces this invariant at compile time.
- **Convenience wrapper not needed:** The only external caller of `unmet_dep_summary` is the test file. Tests will be updated to pass the HashMap directly rather than adding a backward-compatible wrapper. This keeps the API surface minimal.
- **Filtered vs. unfiltered snapshot:** The diagnostic logging HashMap should always be built from the full `snapshot.items` (not the filtered snapshot), since dependency resolution needs visibility into all items. This matches the current behavior.

## References

- `src/scheduler.rs` — `unmet_dep_summary` and `skip_for_unmet_deps` (current linear scan)
- `src/scheduler.rs` — `select_actions` (main scheduling function, calls `skip_for_unmet_deps` in 4 loops)
- `src/scheduler.rs` — `advance_to_next_active_target` (linear scan for target items)
- `src/scheduler.rs` — `select_targeted_actions` (target-item lookup + dependency check)
- `tests/scheduler_test.rs` — Tests that call `unmet_dep_summary` directly
