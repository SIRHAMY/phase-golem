# SPEC: Add Duplicate Item ID Validation to Preflight

**ID:** WRK-034
**Status:** Ready
**Created:** 2026-02-13
**PRD:** ./WRK-034_add-duplicate-item-id-validation-to-preflight_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

The existing dependency graph validation (currently Phase 4) builds a `HashSet<&str>` of item IDs, which silently deduplicates collisions. Duplicate IDs in `BACKLOG.yaml` cause undefined downstream behavior: wrong items scheduled, dependency graph corruption, coordinator mutations targeting wrong entries. This was discovered during WRK-029's build phase when the orchestrator created follow-up items that collided with existing IDs.

The fix is a new validation phase inserted between the existing Phase 3 (item validation) and the existing Phase 4 (dependency graph). After insertion, the phases are renumbered: duplicate ID validation becomes Phase 4, and dependency graph validation becomes Phase 5. The new phase detects and reports all duplicate IDs before execution begins.

## Approach

Add a single private function `validate_duplicate_ids` to `preflight.rs` that uses `HashMap<&str, Vec<usize>>` for O(n) duplicate detection. The function iterates all items with `enumerate()`, builds a map from ID to indices, filters for entries with more than one index, sorts by first occurrence for deterministic output, and returns a `Vec<PreflightError>` with one error per duplicate ID.

Integrate via a single `errors.extend(validate_duplicate_ids(&backlog.items))` call in `run_preflight`, unconditionally executed (matching Phases 3 and 5 behavior). Update the doc comment to list all 5 phases (renumbering the existing dependency graph phase from 4 to 5).

**Patterns to follow:**

- `orchestrator/src/preflight.rs:validate_items` — Function signature pattern: private function accepting immutable references, returning `Vec<PreflightError>`
- `orchestrator/src/preflight.rs:validate_dependency_graph` — Takes `&[BacklogItem]` directly (minimal surface area), same pattern for the new function
- `orchestrator/src/preflight.rs:run_preflight` (lines 52-55) — `errors.extend()` integration pattern for adding a new phase
- `orchestrator/tests/preflight_test.rs` — Test structure: `default_config()` + `make_feature_item()` + `common::make_backlog()` + `run_preflight()` + assertions

**Implementation boundaries:**

- Do not modify: `types.rs`, `lib.rs`, `Cargo.toml`, `tests/common/mod.rs`
- Do not refactor: Existing validation phases or their error messages
- Do not implement: Nice-to-Have item titles in error messages (deferred per design decision)

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Implement Duplicate ID Validation | Low | Add `validate_duplicate_ids` function, integrate into `run_preflight`, update doc comment, add 7 tests |

**Ordering rationale:** This is a single-phase implementation. The function, integration, doc comment update, and tests are all in one file pair (`preflight.rs` + `preflight_test.rs`) with no external dependencies. Splitting into multiple phases would create artificial boundaries.

---

## Phases

### Phase 1: Implement Duplicate ID Validation

> Add the `validate_duplicate_ids` function, integrate it into `run_preflight`, and add comprehensive tests

**Phase Status:** complete

**Complexity:** Low

**Goal:** Detect and report all duplicate item IDs in `BACKLOG.yaml` during preflight validation, with clear error messages identifying each duplicate ID and all conflicting indices.

**Files:**

- `orchestrator/src/preflight.rs` — modify — Add `validate_duplicate_ids` function (~20 lines), add integration line in `run_preflight`, update doc comment
- `orchestrator/tests/preflight_test.rs` — modify — Add 7 test cases for duplicate ID validation (~90-110 lines)

**Patterns:**

- Follow `orchestrator/src/preflight.rs:validate_dependency_graph` for function signature (`fn validate_duplicate_ids(items: &[BacklogItem]) -> Vec<PreflightError>`)
- Follow `orchestrator/src/preflight.rs:run_preflight` Phase 3/5 pattern for unconditional `errors.extend()` integration (Phase 2 is the only gated phase)
- Follow `orchestrator/tests/preflight_test.rs:preflight_dangling_dependency_fails` for test structure and error assertion patterns

**Tasks:**

- [x] Add `validate_duplicate_ids` function to `preflight.rs` between `validate_items` and `validate_dependency_graph`:
  - Takes `items: &[BacklogItem]`, returns `Vec<PreflightError>`
  - Builds `HashMap<&str, Vec<usize>>` via `enumerate()` and `entry().or_default().push(index)`
  - Filters entries where `indices.len() > 1`
  - Sorts duplicate entries by first occurrence index (`sort_by_key(|(_, indices)| indices[0])`)
  - Returns one `PreflightError` per duplicate ID:
    - `condition`: `Duplicate item ID "{id}" found at indices {indices:?}`
    - `config_location`: `BACKLOG.yaml → items`
    - `suggested_fix`: `Remove or rename the duplicate item so each ID is unique`
  - Add code comment above the function explaining why HashMap is used instead of the HashSet::insert() pattern from Phase 1 (Phase 1 only detects the second occurrence; this function needs all indices per the PRD requirement)
- [x] Add integration line in `run_preflight` between Phase 3 (item validation) and the existing dependency graph validation: `errors.extend(validate_duplicate_ids(&backlog.items));` — update the surrounding comments to reflect renumbered phases (new Phase 4: duplicate IDs, old Phase 4 becomes Phase 5: dependency graph)
- [x] Update `run_preflight` doc comment (lines 28-34) to list all 5 phases:
  1. Structural validation — config correctness (fast, no I/O)
  2. Workflow probe — verify referenced workflow files exist on disk
  3. Item validation — in-progress items reference valid pipelines/phases
  4. Duplicate ID validation — ensure no two items share the same ID (NEW)
  5. Dependency graph validation — detect dangling references and circular dependencies
- [x] Add test: `preflight_empty_backlog_no_duplicate_errors` — empty items vec passes
- [x] Add test: `preflight_single_item_no_duplicate_errors` — single item passes
- [x] Add test: `preflight_unique_ids_no_duplicate_errors` — multiple items with distinct IDs pass
- [x] Add test: `preflight_duplicate_id_pair_fails` — two items with same ID (use different `ItemStatus` values, e.g. New and Done, to demonstrate status-independence) produce one error containing the duplicate ID and indices `[0, 1]`
- [x] Add test: `preflight_multiple_distinct_duplicate_ids_fails` — two different duplicate IDs each produce their own error; assert errors are ordered by first occurrence index (e.g., `errors[0].condition` contains the ID that appears at the lower index)
- [x] Add test: `preflight_three_way_duplicate_id_fails` — same ID at three indices produces one error listing all three indices
- [x] Add test: `preflight_case_sensitive_ids_not_duplicates` — items with IDs differing only in case (e.g., "WRK-001" and "wrk-001") are treated as distinct and pass validation

**Verification:**

- [x] All existing preflight tests pass (`cargo test -p orchestrate`)
- [x] All 7 new tests pass
- [x] `cargo build -p orchestrate` succeeds with no warnings
- [x] Duplicate ID errors include the correct ID string, all conflicting indices in ascending order (using `{indices:?}` Debug format, e.g. `[0, 5]`, matching PRD example output), and a suggested fix
- [x] Error ordering is deterministic (sorted by first occurrence index) — verified by `preflight_multiple_distinct_duplicate_ids_fails` asserting error vector order
- [x] The check runs unconditionally (not gated on earlier phase results) — implemented via `errors.extend()` outside any conditional block, matching Phases 3 and 5
- [x] HashMap single-pass approach satisfies PRD's O(n) Should-Have requirement

**Commit:** `[WRK-034][P1] Feature: Add duplicate item ID validation to preflight`

**Notes:**

- The function is private (`fn`, not `pub fn`), matching other internal validation phase functions. Tests exercise it indirectly through `run_preflight`.
- `HashMap` is already imported on line 1 of `preflight.rs` — no new imports needed.
- Test placement: add a new section `// --- Duplicate ID validation tests ---` after the item validation tests (after `preflight_item_with_default_pipeline_type_passes`) and before the dependency graph tests (before `preflight_dangling_dependency_fails`), matching the phase ordering in `run_preflight`.
- Use varied `ItemStatus` values across tests (New, Ready, Done, InProgress) to demonstrate status-independence. The `preflight_duplicate_id_pair_fails` test specifically uses two different statuses for the duplicate items.
- Error message format uses Rust's `{indices:?}` Debug formatting which produces `[0, 5]` — this matches the PRD's example output exactly.
- The `errors.extend()` call is placed outside any conditional block in `run_preflight`, ensuring it runs unconditionally like Phases 3 and 5 (dependency graph). Phase 2 (workflow probe) is the only gated phase.

**Followups:**

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
| 1 | complete | `[WRK-034][P1] Feature: Add duplicate item ID validation to preflight` | All 7 tests pass, 37 total preflight tests pass, no warnings, code review clean |

## Followups Summary

### Critical

### High

### Medium

### Low

- Nice-to-Have: Include item titles alongside IDs in error messages for easier identification in large backlogs — deferred per design decision, can be added later without changing function signature

## Design Details

### Key Types

No new types introduced. Uses existing types:

```rust
// Already in preflight.rs
#[derive(Debug, Clone, PartialEq)]
pub struct PreflightError {
    pub condition: String,
    pub config_location: String,
    pub suggested_fix: String,
}

// Already in types.rs
pub struct BacklogItem {
    pub id: String,
    // ... other fields
}
```

### Architecture Details

The new function fits into the existing validation pipeline:

```
run_preflight
  ├── Phase 1: validate_structure(config)
  ├── Phase 2: probe_workflows(config, project_root)  [gated on Phase 1]
  ├── Phase 3: validate_items(config, backlog)
  ├── Phase 4: validate_duplicate_ids(&backlog.items)  ← NEW
  └── Phase 5: validate_dependency_graph(&backlog.items)
```

Phase 4 (new) runs unconditionally like Phases 3 and 5. It validates all items regardless of status since IDs must be globally unique.

### Design Rationale

- **HashMap over HashSet:** PRD requires reporting all indices of duplicate IDs (e.g., `[0, 5]`), not just the second occurrence. `HashSet::insert()` only detects the collision at the second occurrence and cannot report the first index. The HashMap approach collects all indices in a single O(n) pass.
- **Deterministic sorting:** HashMap iteration is non-deterministic. Sorting duplicates by first occurrence index ensures reliable test assertions and predictable operator output.
- **Unconditional execution:** Duplicate IDs are a data integrity issue independent of config correctness or workflow file existence. Running unconditionally gives operators the full error picture in a single run.
- **Private visibility:** The function is called only from `run_preflight` and follows the pattern of all other validation phase functions. Tests exercise it through the public API.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
