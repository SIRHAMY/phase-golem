# SPEC: Gate preflight Phase 3 item validation on Phase 1 structural validation passing

**ID:** WRK-015
**Status:** Draft
**Created:** 2026-02-19
**PRD:** ./WRK-015_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

Phase 3 (`validate_items`) in `run_preflight` runs unconditionally, even when Phase 1 (`validate_structure`) has already found structural errors in the pipeline configuration. This produces misleading secondary errors when items reference broken pipelines. Phase 2 already gates on Phase 1 passing — Phase 3 should follow the same pattern, but with a snapshot variable to isolate the gate to Phase 1 results specifically (not Phase 1+2 combined).

## Approach

Add a snapshot variable `let structural_ok = errors.is_empty();` after Phase 1 completes (before Phase 2 runs), then wrap the Phase 3 call in `if structural_ok { ... }`. This is a 2-line addition to `run_preflight` in `src/preflight.rs`, following the identical gate pattern already used by Phase 2 at lines 50-52.

The snapshot variable (rather than inline `errors.is_empty()`) ensures the gate is precisely "Phase 1 passed" — not "all prior phases passed." This matters because Phase 3 depends on config structure (Phase 1) but not on workflow file existence (Phase 2). An item referencing a valid pipeline/phase should still be validated even if a workflow file is missing.

**Patterns to follow:**

- `src/preflight.rs:50-52` — Phase 2 gate pattern (`if errors.is_empty() { errors.extend(...) }`)

**Implementation boundaries:**

- Do not modify: `validate_structure`, `validate_items`, `probe_workflows`, `validate_duplicate_ids`, `validate_dependency_graph` — validation logic is unchanged
- Do not modify: `run_preflight` function signature or return type
- Do not modify: Phases 4 and 5 — they must remain ungated and run unconditionally even when Phase 1 fails (they operate on backlog item fields only, independent of config structure)

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Guard clause + tests | Low | Add snapshot variable and guard clause in `run_preflight`, add 3 tests, update doc comment |

**Ordering rationale:** Single phase — the production change and tests are tightly coupled and small enough to implement together.

---

## Phases

Each phase should leave the codebase in a functional, stable state. Complete and verify each phase before moving to the next.

---

### Phase 1: Guard clause + tests

> Add snapshot variable and guard clause to gate Phase 3 on Phase 1 passing, with test coverage

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Gate Phase 3 item validation on Phase 1 structural validation passing, using a snapshot variable to isolate the gate to Phase 1 results only. Verify with three new tests and ensure all existing tests still pass.

**Files:**

- `src/preflight.rs` — modify — Add snapshot variable after Phase 1, wrap Phase 3 in guard clause, update doc comment
- `tests/preflight_test.rs` — modify — Add 3 new test functions for gating behavior

**Patterns:**

- Follow `src/preflight.rs:50-52` for the guard clause pattern (Phase 2 gate)
- Follow existing tests in `tests/preflight_test.rs` (e.g., `preflight_no_main_phases_fails` at line 61, `preflight_missing_workflow_files_fails` at line 288) for test structure and helper usage

**Tasks:**

- [ ] Add `let structural_ok = errors.is_empty();` after line 47 (`errors.extend(validate_structure(config));`) and before line 50 (`if errors.is_empty()` for Phase 2). Include an inline comment: `// Snapshot before Phase 2; gates Phase 3 on Phase 1 results only`
- [ ] Wrap Phase 3 call (line 55: `errors.extend(validate_items(config, backlog));`) in `if structural_ok { ... }`
- [ ] Update the `run_preflight` doc comment line for Phase 3 from `/// 3. Item validation — in-progress items reference valid pipelines/phases` to `/// 3. Item validation — in-progress items reference valid pipelines/phases (skipped when Phase 1 finds structural errors)`
- [ ] Add test `preflight_phase3_skipped_when_phase1_fails`: create a config with a structurally broken pipeline (no main phases via `PipelineConfig { pre_phases: vec![], phases: vec![] }`) and an `InProgress` item referencing that pipeline with `pipeline_type = "broken"`. Assert: (1) at least one error contains `"no main phases"`, (2) no error contains `"unknown pipeline type"` or `"unknown phase"` (proving Phase 3 was skipped)
- [ ] Add test `preflight_phase3_runs_when_phase1_passes_but_phase2_fails`: create a structurally valid config with a single phase referencing a nonexistent workflow file (triggers Phase 2 failure), and a valid `InProgress` item with matching `pipeline_type`, `phase`, and `phase_pool`. Use `tempfile::tempdir()` for the root. Assert: (1) at least one error contains `"Workflow file not found"` (Phase 2 ran), (2) no error contains `"unknown phase"` or `"unknown pipeline type"` (Phase 3 ran and found no issues for the valid item). This test also serves as a regression guard for correct snapshot placement — if the snapshot were placed after Phase 2, it would be `false` and Phase 3 would be incorrectly skipped
- [ ] Add test `preflight_phase4_and_phase5_run_when_phase1_fails`: create a config with a structurally broken pipeline (no main phases) and a backlog with two items sharing the same ID (triggers Phase 4 duplicate error). Assert: (1) at least one error contains `"no main phases"` (Phase 1 ran), (2) at least one error contains `"Duplicate item ID"` (Phase 4 ran despite Phase 1 failure). This verifies Phases 4-5 remain ungated

**Verification:**

- [ ] `cargo test` — all existing tests pass without modification
- [ ] `cargo test` — all three new tests pass
- [ ] `cargo clippy` — no new warnings
- [ ] Codebase builds without errors
- [ ] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-015][P1] Fix: Gate Phase 3 item validation on Phase 1 passing`

**Notes:**

The snapshot variable must be placed between Phase 1's `errors.extend(validate_structure(config))` and Phase 2's `if errors.is_empty()` gate. This is the only position where `errors.is_empty()` reflects exactly "Phase 1 passed." Placing it after Phase 2 would gate on Phase 1+2 combined, which violates the PRD requirement that Phase 3 should still run when Phase 1 passes but Phase 2 fails.

**Followups:**

---

## Final Verification

- [ ] All phases complete
- [ ] All PRD success criteria met:
  - [ ] Phase 3 skipped when Phase 1 produces any error
  - [ ] Phase 3 still runs when Phase 1 passes but Phase 2 fails
  - [ ] Phases 4 and 5 continue to run unconditionally
  - [ ] Test verifies Phase 3 suppressed when Phase 1 fails
  - [ ] Test verifies Phase 3 runs when Phase 2 fails
  - [ ] Test verifies Phases 4-5 run when Phase 1 fails
  - [ ] Existing tests pass without modification
  - [ ] `cargo clippy` produces no new warnings
  - [ ] Snapshot variable used (Should Have)
  - [ ] Doc comment updated (Nice to Have)
- [ ] Tests pass
- [ ] No regressions introduced

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|

## Followups Summary

### Critical

### High

### Medium

### Low

## Design Details

### Design Rationale

The snapshot variable approach was chosen over inline `errors.is_empty()` because:

1. **Correctness:** After Phase 2 runs and potentially adds errors, `errors.is_empty()` reflects Phase 1+2 combined. The snapshot isolates the gate to Phase 1 only.
2. **Robustness:** If a future change adds a new phase between 1 and 3, the snapshot remains correct. The inline approach would silently change behavior.
3. **Consistency:** Follows the established guard clause pattern from Phase 2, with the minimal addition of a snapshot for precise scoping.

See Design doc (`./WRK-015_DESIGN.md`) for the full alternatives analysis.
