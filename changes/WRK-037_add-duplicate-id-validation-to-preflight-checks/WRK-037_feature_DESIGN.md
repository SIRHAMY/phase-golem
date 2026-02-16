# Design: Add Duplicate ID Validation to Preflight Checks

**ID:** WRK-037
**Status:** Complete (No Design Needed)
**Created:** 2026-02-13
**PRD:** ./WRK-037_feature_PRD.md
**Mode:** Light

## Overview

No design work is required. The feature described by WRK-037 — duplicate item ID validation in the orchestrator's preflight check system — is **already fully implemented, tested, and integrated**.

The `validate_duplicate_ids()` function exists in `orchestrator/src/preflight.rs:320-343`, runs as Phase 4 of `run_preflight()`, and has comprehensive test coverage in `tests/preflight_test.rs:437-557`.

WRK-037 is a duplicate of WRK-034 (which has a complete PRD, tech research, design, and spec) and WRK-040 (whose PRD also correctly identifies the feature as already implemented).

---

## Existing Implementation Summary

### Architecture

The duplicate ID validation is a single pure function integrated into the existing preflight check pipeline:

- **Function:** `validate_duplicate_ids(items: &[BacklogItem]) -> Vec<PreflightError>`
- **Location:** `orchestrator/src/preflight.rs:320-343`
- **Integration:** Called at line 57 as Phase 4 of `run_preflight()`
- **Algorithm:** O(n) scan using `HashMap<&str, Vec<usize>>` to track all indices per ID
- **Output:** One `PreflightError` per duplicate ID group, reporting all indices where the duplicate appears

### Test Coverage

Seven tests in `tests/preflight_test.rs:437-557` cover:
- Empty backlog, single item, all unique IDs (no false positives)
- Duplicate pair detection (reports both indices)
- Multiple distinct duplicate IDs (separate error per group)
- Three-way duplicates (reports all three indices)
- Case sensitivity (`WRK-001` vs `wrk-001` treated as distinct)

---

## Recommendation

**Close WRK-037.** This item is a duplicate of already-completed work. The backlog already contains meta-items acknowledging this duplication (WRK-042, WRK-043, WRK-044, WRK-074).

---

## Assumptions

- Determined no design is needed because the PRD was rejected as "Already Implemented" and code inspection confirms the feature exists and is fully operational.
- This design doc serves as a record that the design phase was evaluated and correctly skipped.

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Design phase evaluation | No design needed — feature already implemented as part of WRK-034 |
