# WRK-029: Enforce dependency ordering in the scheduler

## Problem Statement

`BacklogItem` has a `dependencies: Vec<String>` field that stores references to other item IDs. The field is fully serialized/deserialized through `BACKLOG.yaml` and preserved through migration. However, the scheduler's `select_actions()` function completely ignores it — items with unmet dependencies are scheduled as if they have none.

This means an item declaring `dependencies: [WRK-026]` will be picked up and executed before WRK-026 is done, potentially producing incorrect or incomplete results. There is currently no way to express "do X before Y" in the orchestrator, making the `dependencies` field purely decorative.

## Proposed Approach

### 1. Dependency filtering in `select_actions()` (scheduler.rs)

Add a filtering step early in `select_actions()` that identifies items with unmet dependencies. An item's dependencies are "met" when every referenced item ID has `status == Done` in the snapshot. Items with unmet dependencies are excluded from scheduling consideration — they are not promoted, not advanced, and not triaged.

This is a scheduling-only filter rather than a new status. Items remain in their current status (New, Ready, InProgress, etc.) but are simply not actionable until dependencies resolve. This avoids adding state machine complexity and keeps the filter reversible (remove the dependency, item becomes schedulable again).

### 2. Cycle detection in preflight validation (preflight.rs)

Add a new validation step to Phase 1 (Structural Validation) that builds a directed graph from all items' dependency lists and runs a DFS-based cycle detection algorithm. Circular dependencies (A→B→C→A) are reported as preflight errors with the full cycle path, preventing the scheduler from running with unsatisfiable constraints.

Also validate that dependency references point to existing item IDs in the backlog. References to unknown IDs should be reported as warnings or errors.

### 3. Status output visibility

Items with unmet dependencies should be distinguishable in status output so users understand why an item isn't being scheduled. This could be done by annotating items with their unmet dependency list in the status display, without changing the underlying status model.

## Files Affected

- **Modified:**
  - `orchestrator/src/scheduler.rs` — Add dependency filtering logic in `select_actions()`, helper function to check if dependencies are met
  - `orchestrator/src/preflight.rs` — Add cycle detection and dangling reference validation
  - `orchestrator/tests/scheduler_test.rs` — Add unit tests for dependency filtering, integration tests for multi-item dependency chains
  - `orchestrator/tests/preflight_test.rs` (or inline tests) — Add cycle detection tests

## Assessment

| Dimension  | Rating | Rationale |
|------------|--------|-----------|
| Size       | Medium | 4-5 files modified; ~100 lines of scheduler logic, ~80 lines of cycle detection, ~250 lines of tests |
| Complexity | Medium | Design decisions around filter-vs-status, cycle detection algorithm, edge cases (transitive deps, blocked deps in chain) |
| Risk       | Medium | Modifies core scheduling logic; incorrect filtering could prevent items from ever being scheduled; needs thorough testing |
| Impact     | High   | Unlocks fundamental orchestration capability; prerequisite for meaningful work ordering across dependent items |

## Edge Cases

- **Self-dependencies**: Item depending on itself — caught by cycle detection
- **Transitive chains**: A→B→C — works naturally since each item checks only its direct dependencies
- **Partial completion**: Some deps done, others not — item remains non-schedulable until all deps are done
- **Dangling references**: Dependency on non-existent item ID — preflight should catch this
- **Blocked items in chain**: A depends on B which is Blocked — A remains non-schedulable (correct behavior, B must be unblocked and completed first)
- **Target mode**: When `--target` specifies an item with unmet deps — should still enforce ordering, but this needs consideration

## Assumptions

- Dependency filtering is implemented as a scheduling filter in `select_actions()` rather than introducing a new `ItemStatus` variant. This keeps the state machine simple and avoids a migration.
- Only direct dependencies are checked (not transitive). Since each item in a chain has its own dependency declaration, transitive ordering is enforced naturally as items complete in sequence.
- The cycle detection algorithm uses iterative DFS with a coloring scheme (white/gray/black) for O(V+E) performance, which is well-suited to the small graph sizes involved.
- Dependency references to non-existent item IDs are treated as preflight errors (not warnings), since they likely indicate typos or stale references that would cause items to be permanently non-schedulable.
- No changes to the `BacklogItem` struct or serialization format are needed — the existing `dependencies` field is sufficient.
