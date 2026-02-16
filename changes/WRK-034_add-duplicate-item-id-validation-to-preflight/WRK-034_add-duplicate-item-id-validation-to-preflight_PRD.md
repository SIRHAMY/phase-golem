# Change: Add Duplicate Item ID Validation to Preflight

**Status:** Proposed
**Created:** 2026-02-13
**Author:** Orchestrator (autonomous)

## Problem Statement

The orchestrator's preflight validation (`preflight.rs`) validates configuration and backlog state before execution begins. It currently runs four phases:

1. **Structural validation** — config correctness (fast, no I/O)
2. **Workflow probe** — verify referenced workflow files exist on disk
3. **Item validation** — in-progress/scoping items reference valid pipelines and phases
4. **Dependency graph validation** — detect dangling references and circular dependencies

Phase 4 builds a `HashSet<&str>` of item IDs, which silently deduplicates any collisions. There is no check that item IDs in `BACKLOG.yaml` are unique.

Duplicate IDs can arise from manual editing errors, unresolved merge conflicts, or the orchestrator itself creating follow-up items that collide with existing IDs (as observed during WRK-029's build phase, which spawned this very item). When duplicate IDs exist, downstream behavior is undefined and silently wrong: the scheduler may pick the wrong item, dependency graph validation silently drops duplicates, coordinator mutations (which update backlog state) may target the wrong entry, and phase results may be attributed to the wrong item.

This is a data integrity issue that preflight should catch before execution begins.

## User Stories / Personas

- **Orchestrator operator** — Wants preflight to catch corrupt backlog state before autonomous execution begins, rather than discovering inconsistencies mid-run that are hard to diagnose.

- **Human backlog editor** — Manually edits `BACKLOG.yaml` (e.g., to add items, resolve merges) and wants fast feedback if they accidentally introduce a duplicate ID.

## Desired Outcome

When `BACKLOG.yaml` contains two or more items with the same `id` value, preflight reports a clear error identifying each duplicate ID and the array indices of the conflicting items. The orchestrator exits with an error and does not proceed to execution. After fixing duplicates in `BACKLOG.yaml`, the operator re-runs the orchestrator to retry preflight.

The check runs as a dedicated validation step between the existing Phase 3 (item validation) and Phase 4 (dependency graph), so that dependency graph validation can trust that IDs are unique.

### Example Error Output

```
Preflight error: Duplicate item ID "WRK-034" found at indices [0, 5]
  Config: BACKLOG.yaml → items
  Fix: Remove or rename the duplicate item so each ID is unique
```

## Success Criteria

### Must Have

- [ ] Preflight detects and reports all duplicate item IDs in `BACKLOG.yaml`
- [ ] Each error message identifies the duplicate ID and the 0-based array indices of all items sharing that ID
- [ ] Errors use the existing `PreflightError` struct (condition, config_location, suggested_fix)
- [ ] The check runs before dependency graph validation (Phase 4) so that Phase 4 can assume unique IDs
- [ ] The check validates all items regardless of status (IDs must be globally unique)
- [ ] Existing preflight tests continue to pass

### Should Have

- [ ] New unit tests covering: empty backlog (passes), single item (passes), no duplicates (passes), one pair of duplicates, multiple distinct duplicate IDs, three-way duplicate of the same ID
- [ ] The check is O(n) in the number of backlog items (single pass with a HashMap counting occurrences)

### Nice to Have

- [ ] Error message includes item titles alongside IDs for easier identification in large backlogs

## Scope

### In Scope

- Adding a `validate_duplicate_ids` function to `preflight.rs`
- Integrating the check into `run_preflight` between Phase 3 and Phase 4
- Unit tests for the new validation

### Out of Scope

- Preventing duplicate IDs at write time (separate concern in the coordinator/backlog writer)
- Validating `next_item_id` consistency (covered by WRK-031)
- Deduplicating or auto-fixing duplicate IDs — preflight only reports errors
- Validating uniqueness of other fields (e.g., titles)
- Validating ID format (e.g., `WRK-\d+` pattern) — IDs are treated as opaque strings
- Checking for duplicates across files (e.g., archived backlogs) — only `BACKLOG.yaml` is checked

## Non-Functional Requirements

- **Performance:** The check must be O(n) — a single pass over the items list with a `HashMap<&str, Vec<usize>>` tracking indices per ID. Backlog sizes are small (< 1000 items), so this is not a practical concern, but we should not introduce quadratic behavior.

## Constraints

- Must use the existing `PreflightError` struct and integrate into the `run_preflight` flow
- Must not change the `run_preflight` function signature or return type
- Must run after Phase 3 (item validation) and before Phase 4 (dependency graph), since Phase 4's `HashSet<&str>` silently deduplicates
- ID comparison is case-sensitive, matching Rust's standard `String` equality (IDs like `WRK-034` and `wrk-034` are considered distinct)

## Dependencies

- **Depends On:** Nothing — this is a standalone addition to existing infrastructure
- **Blocks:** Nothing directly, but improves reliability for all downstream preflight consumers

## Risks

- [ ] Low risk: The implementation is straightforward (HashMap-based duplicate detection is well-understood). The main risk is choosing the wrong integration point in the validation sequence, but the constraint above makes the correct placement explicit.

## Open Questions

None — the scope and approach are well-defined.

## Assumptions

- The `id` field on `BacklogItem` is a non-optional `String` (enforced by serde deserialization). Items with missing or malformed IDs fail at YAML parse time before preflight runs.
- The duplicate check considers all items in the backlog regardless of status, since IDs must be globally unique.
- The check runs unconditionally (not gated on earlier phases passing), consistent with how Phase 3 currently runs alongside Phase 1.
- `BACKLOG.yaml` is the single source of truth for item IDs. Archived or external items are not checked.

## References

- `preflight.rs` — Current validation phases
- WRK-029 — Build phase that originally discovered the duplicate ID issue
- WRK-037, WRK-040 — Duplicate backlog entries tracking this same feature (ironic evidence that the problem exists in practice)
