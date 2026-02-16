# Change: Add Duplicate ID Validation to Preflight Checks

**Status:** Rejected (Already Implemented)
**Created:** 2026-02-13
**Author:** Orchestrator (autonomous)

## Problem Statement

WRK-037 requests adding duplicate item ID validation to the orchestrator's preflight check system. However, this feature **already exists** as Phase 4 of `run_preflight()` in `orchestrator/src/preflight.rs` (lines 313-343). It was implemented as part of earlier work and is fully operational with comprehensive test coverage.

This item is one of three duplicates tracking the same feature:
- **WRK-034** — "Add duplicate item ID validation to preflight" (has a PRD, feature is implemented)
- **WRK-037** — This item
- **WRK-040** — "Add duplicate item ID validation to preflight" (PRD correctly identifies it as already done)

The backlog already contains meta-items acknowledging this duplication: WRK-042 (deduplicate WRK-034/037/040), WRK-043 (close WRK-034 as dup of WRK-037), WRK-044 (close WRK-040 as dup of WRK-037), and WRK-074 (close WRK-040 as dup of WRK-034).

## Current Implementation

The `validate_duplicate_ids()` function in `preflight.rs:320-343` already:

- Runs as Phase 4 in `run_preflight()` (called at line 57)
- Uses `HashMap<&str, Vec<usize>>` to track all indices where each ID appears
- Reports ALL duplicate indices, not just the second occurrence
- Returns actionable `PreflightError` with condition, config_location, and suggested_fix
- Runs after item validation (Phase 3) and before dependency graph validation (Phase 5)
- Is case-sensitive (consistent with Rust `String` equality)
- Is O(n) in the number of backlog items

### Test Coverage

Comprehensive tests exist in `tests/preflight_test.rs` (lines 437-557):
- Empty backlog — no false positives
- Single item — no false positives
- All unique IDs — no false positives
- Duplicate pair detection — reports both indices
- Multiple distinct duplicate IDs — separate error per group
- Three-way duplicate — reports all three indices
- Case sensitivity — `WRK-001` and `wrk-001` treated as distinct

### Example Error Output (actual)

```
Preflight error: Duplicate item ID "WRK-001" found at indices [0, 2]
  Config: BACKLOG.yaml -> items
  Fix: Remove or rename the duplicate item so each ID is unique
```

## Recommendation

**Close WRK-037 as already implemented.** No new work is needed. The feature matches the requested behavior exactly. The related items (WRK-034, WRK-040) should also be closed.

## Assumptions

- Determined this item is already implemented by reading the existing `preflight.rs` code and confirming `validate_duplicate_ids` matches the requested behavior.
- The backlog's own meta-items (WRK-042, WRK-043, WRK-044, WRK-074) confirm the duplication was already identified by the orchestrator's triage process.
- No additional work is needed — the implementation, tests, and integration are all complete.

## References

- `orchestrator/src/preflight.rs:320-343` — `validate_duplicate_ids()` implementation
- `orchestrator/src/preflight.rs:56-57` — integration into `run_preflight()` as Phase 4
- `orchestrator/tests/preflight_test.rs:437-557` — duplicate ID test suite
- WRK-034, WRK-040 — sibling duplicates of this same feature
- WRK-042, WRK-043, WRK-044, WRK-074 — meta-items acknowledging the duplication
