# Change: Add Duplicate Item ID Validation to Preflight

**Status:** Rejected (Already Implemented)
**Created:** 2026-02-13
**Author:** Orchestrator (autonomous)

## Problem Statement

WRK-040 requests adding duplicate item ID validation to the preflight check system. However, this feature **already exists** — it was implemented as part of WRK-034 and is fully operational as Phase 4 of `run_preflight()` in `orchestrator/src/preflight.rs`.

This item is a duplicate of WRK-034 (which is done) and WRK-037 (which is also a duplicate). The backlog already contains items WRK-042, WRK-043, and WRK-044 acknowledging the duplication and recommending closure.

## Current Implementation

The duplicate ID validation already:

- Runs as Phase 4 in `run_preflight()` (`preflight.rs:57`)
- Uses `HashMap<&str, Vec<usize>>` to track all indices where each ID appears
- Reports ALL duplicate indices (not just the second occurrence)
- Returns actionable `PreflightError` with condition, config location, and suggested fix
- Has comprehensive test coverage in `tests/preflight_test.rs`:
  - Empty backlog (no false positives)
  - Single item (no false positives)
  - Unique IDs (no false positives)
  - Duplicate pair detection
  - Multiple distinct duplicate IDs
  - Three-way duplicate detection
  - Case-sensitive ID comparison

## Recommendation

**Close WRK-040 as duplicate.** No new work is needed. The feature is fully implemented and tested.

## Assumptions

- Determined this item is a duplicate by reading the existing `preflight.rs` implementation and confirming the `validate_duplicate_ids` function matches the requested behavior exactly.
- The backlog's own meta-items (WRK-042, WRK-043, WRK-044) confirm the duplication was already identified.

## References

- `orchestrator/src/preflight.rs:320-343` — `validate_duplicate_ids()` implementation
- `orchestrator/tests/preflight_test.rs:437-557` — duplicate ID test suite
- WRK-034 — the original implementation (status: done, shipped)
- WRK-037 — another duplicate of this same feature
- WRK-042, WRK-043, WRK-044 — backlog items acknowledging the duplication
