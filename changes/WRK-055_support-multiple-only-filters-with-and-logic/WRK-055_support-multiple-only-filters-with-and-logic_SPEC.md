# SPEC: Support Multiple --only Filters with AND Logic

**ID:** WRK-055
**Status:** Draft
**Created:** 2026-02-20
**PRD:** ./WRK-055_support-multiple-only-filters-with-and-logic_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

The `--only` flag currently accepts a single `KEY=VALUE` filter. Users need to specify multiple criteria combined with AND logic to narrow runs to precise subsets (e.g., `--only impact=high --only size=small`). This was planned as a "Should Have" in the original WRK-030 PRD. The codebase already has the building blocks: `--target` uses `Vec<String>` with `ArgAction::Append`, `matches_item()` is composable, and `FilterCriterion` implements `Display`.

## Approach

Extend the filter pipeline through a vertical slice: CLI (`main.rs`) collects repeated `--only` values into `Vec<String>`, each is parsed via `parse_filter()` with fail-fast semantics, cross-criterion validation detects duplicate scalar fields, `RunParams.filter` changes from `Option<FilterCriterion>` to `Vec<FilterCriterion>`, and the scheduler applies all criteria via AND composition using `criteria.iter().all(|c| matches_item(c, item))`. Three new functions are added to `filter.rs` (`validate_filter_criteria`, `apply_filters`, `format_filter_criteria`) and the existing `apply_filter()` is removed.

The change is split into two phases: Phase 1 adds new functions and their tests without breaking existing code (additive-only). Phase 2 does the atomic migration: type changes, plumbing updates, old function removal, and test migrations. This ensures the codebase compiles after each phase.

**Patterns to follow:**

- `src/main.rs:57-58` — `--target` arg uses `Vec<String>` with `action = clap::ArgAction::Append` — exact template for the `--only` change
- `src/filter.rs:135-150` — existing `apply_filter()` structure — template for `apply_filters()` with multi-criterion composition
- `src/filter.rs:32-58` — `Display` impl for `FilterCriterion` — used by `format_filter_criteria()` to join criteria

**Implementation boundaries:**

- Do not modify: `matches_item()` signature or logic (multi-filter composes via repeated calls)
- Do not modify: `HaltReason` variants (semantics unchanged)
- Do not introduce: new types, modules, or external dependencies
- Out of scope: OR logic within a field (WRK-056), negation filters, combining `--target` and `--only`

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Foundation: New filter functions + tests | Low | Add `Hash` derives, `validate_filter_criteria()`, `apply_filters()`, `format_filter_criteria()` with unit tests — all additive, no breaking changes |
| 2 | Migration: CLI, plumbing, and test migration | Med | Change CLI to `Vec<String>`, migrate `RunParams.filter` to `Vec`, rewrite scheduler filter block, remove `apply_filter()`, update all tests |

**Ordering rationale:** Phase 1 is purely additive — it adds new functions and tests while the existing code continues to compile unchanged. Phase 2 then does the atomic migration, swapping `Option` to `Vec` across all call sites simultaneously. This boundary ensures a compilable codebase after each phase.

---

## Phases

---

### Phase 1: Foundation: New filter functions + tests

> Add Hash derives, new filter functions, and their unit tests — all additive, no breaking changes

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Add the three new functions (`validate_filter_criteria`, `apply_filters`, `format_filter_criteria`) to `filter.rs` with `Hash` derives on supporting types, plus comprehensive unit tests. The existing `apply_filter()` remains untouched — nothing breaks.

**Files:**

- `src/types.rs` — modify — Add `Hash` to derives on `ItemStatus`, `SizeLevel`, `DimensionLevel`
- `src/filter.rs` — modify — Add `Hash` derives to `FilterField`, `FilterValue`, `FilterCriterion`; add `validate_filter_criteria()`, `apply_filters()`, `format_filter_criteria()`; add `use std::collections::HashSet`
- `tests/filter_test.rs` — modify — Add new test functions for `validate_filter_criteria()`, `apply_filters()` multi-criteria, and `format_filter_criteria()`

**Patterns:**

- Follow `src/filter.rs:135-150` (`apply_filter()`) as structural template for `apply_filters()` — same pattern but with `.all()` across criteria slice

**Tasks:**

- [ ] Add `Hash` to `#[derive(...)]` on `ItemStatus` (`src/types.rs`), `SizeLevel` (`src/types.rs`), `DimensionLevel` (`src/types.rs`)
- [ ] Add `Hash` to `#[derive(...)]` on `FilterField`, `FilterValue`, `FilterCriterion` (`src/filter.rs`)
- [ ] Add `use std::collections::HashSet;` to `src/filter.rs`
- [ ] Implement `validate_filter_criteria(criteria: &[FilterCriterion]) -> Result<(), String>`:
  - Use `HashSet<&FilterField>` for scalar field uniqueness (all fields except `Tag`)
  - Use `HashSet<&FilterCriterion>` for identical tag-value pair detection
  - No-op returning `Ok(())` on empty slice
  - Error message for scalar duplicates includes WRK-056 hint: `"Field '{field}' specified multiple times. For OR logic within a field, use comma-separated values: --only {field}=value1,value2"`
  - Error message for identical tag pairs uses `Display` format: `"Duplicate filter: tag=backend specified multiple times"` (i.e., `criterion.to_string()` produces the `tag=value` portion)
- [ ] Implement `apply_filters(criteria: &[FilterCriterion], backlog: &BacklogFile) -> BacklogFile`:
  - Filter items where `criteria.iter().all(|c| matches_item(c, item))`
  - Same structural shape as existing `apply_filter()` but composing multiple criteria
- [ ] Implement `format_filter_criteria(criteria: &[FilterCriterion]) -> String`:
  - Join criteria `Display` strings with ` AND ` separator
  - Single criterion: no separator (just `criterion.to_string()`)
  - Panics or empty string on empty slice is acceptable since callers guard with `is_empty()`, but prefer returning empty string for robustness
- [ ] Write tests for `validate_filter_criteria()`:
  - Empty slice returns `Ok(())`
  - Single criterion returns `Ok(())`
  - Two different scalar fields returns `Ok(())` (e.g., `impact=high` + `size=small`)
  - Duplicate scalar field returns `Err` with WRK-056 hint (e.g., `impact=high` + `impact=low`)
  - Identical scalar field+value pair returns `Err` (e.g., `impact=high` + `impact=high`)
  - Two different tag values returns `Ok(())` (e.g., `tag=backend` + `tag=sprint-1`)
  - Identical tag values returns `Err` (e.g., `tag=backend` + `tag=backend`)
  - Mixed scalar and tag criteria returns `Ok(())`
  - Non-adjacent duplicate scalar fields detected (e.g., `impact=high` + `tag=backend` + `impact=low` returns `Err`)
- [ ] Write tests for `apply_filters()` with multiple criteria:
  - Two criteria AND: only items matching both pass
  - Item matching one criterion but not other is excluded
  - Items with `None` for a filtered optional field are excluded by AND
  - Multi-tag AND: items must have all specified tags
  - Empty criteria slice returns all items (vacuous truth)
  - Single criterion behaves identically to current `apply_filter()`
- [ ] Write tests for `format_filter_criteria()`:
  - Empty slice: `""` (empty string)
  - Single criterion: `"impact=high"`
  - Two criteria: `"impact=high AND size=small"`
  - Three criteria: `"impact=high AND size=small AND status=ready"`

**Verification:**

- [ ] `cargo build` succeeds (no compile errors)
- [ ] `cargo test` passes (all existing tests still pass, new tests pass)
- [ ] New functions are `pub` and importable (confirmed by test file compiling with `use phase_golem::filter::{validate_filter_criteria, apply_filters, format_filter_criteria}`)
- [ ] Existing `apply_filter()` still works (not yet removed)
- [ ] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-055][P1] Feature: Add multi-filter validation, application, and formatting functions`

**Notes:**

- `Hash` derives are safe on all target types: `FilterField` is a fieldless enum, `FilterValue` contains `ItemStatus`/`SizeLevel`/`DimensionLevel` (fieldless enums) and `String` (all implement `Hash`), `FilterCriterion` is a struct of `FilterField` + `FilterValue`.
- `apply_filter()` coexists with `apply_filters()` during this phase — removal happens in Phase 2.

**Followups:**

---

### Phase 2: Migration: CLI, plumbing, and test migration

> Change CLI to Vec, migrate RunParams.filter, rewrite scheduler filter block, remove apply_filter(), update all tests

**Phase Status:** not_started

**Complexity:** Med

**Goal:** Complete the end-to-end migration from `Option<FilterCriterion>` to `Vec<FilterCriterion>` across CLI, main.rs, scheduler.rs, and all test files. Remove the superseded `apply_filter()`. This is the compile-breaking phase where all type changes must land atomically.

**Files:**

- `src/main.rs` — modify — CLI arg `Option<String>` → `Vec<String>` with `Append`; `handle_run()` signature; mutual-exclusion guard; filter parsing loop with fail-fast + validation; startup display; `filter_display` construction; `RunParams` construction
- `src/scheduler.rs` — modify — `RunParams.filter`: `Option<FilterCriterion>` → `Vec<FilterCriterion>`; filter application block rewrite (lines 678-748)
- `src/filter.rs` — modify — Remove `apply_filter()` function
- `tests/filter_test.rs` — modify — Remove `apply_filter` import; migrate all `apply_filter()` call sites to `apply_filters(&[criterion], &backlog)`
- `tests/scheduler_test.rs` — modify — `run_params()` helper: `filter: None` → `filter: vec![]`; all inline `filter: None` → `filter: vec![]`; all inline `filter: Some(x)` → `filter: vec![x]`

**Tasks:**

- [ ] Change CLI arg definition (`src/main.rs:60-62`): `only: Option<String>` → `only: Vec<String>` with `action = clap::ArgAction::Append`. Keep `conflicts_with = "target"` and update help text to indicate repeatable flag.
- [ ] Update `handle_run()` signature (`src/main.rs:333`): `only: Option<String>` → `only: Vec<String>`
- [ ] Update mutual-exclusion guard (`src/main.rs:363`): `only.is_some()` → `!only.is_empty()`
- [ ] Rewrite filter parsing block (`src/main.rs:418-422`): Replace `match only { Some(ref raw) => ... }` with a loop: iterate `only` values, call `parse_filter()` on each with `?` (fail-fast), collect into `Vec<FilterCriterion>`, then call `validate_filter_criteria(&parsed_filters)?`
- [ ] Update startup display (`src/main.rs:456-464`): Change `if let Some(ref criterion) = parsed_filter` to `if !parsed_filters.is_empty()`, use `apply_filters(&parsed_filters, &backlog)` for match count, use `format_filter_criteria(&parsed_filters)` for display string
- [ ] Update `filter_display` construction (`src/main.rs:632`): `if !parsed_filters.is_empty() { Some(format_filter_criteria(&parsed_filters)) } else { None }`
- [ ] Update RunParams construction (`src/main.rs:634-636`): `filter: parsed_filters`
- [ ] Halt-reason display (`src/main.rs:733-748`): Verify no changes needed — `filter_display` remains `Option<String>`, `if let Some(ref filter_str)` guards still work
- [ ] Change `RunParams.filter` type (`src/scheduler.rs:52`): `Option<crate::filter::FilterCriterion>` → `Vec<crate::filter::FilterCriterion>`
- [ ] Rewrite scheduler filter application block (`src/scheduler.rs:678-748`):
  - Change guard: `if let Some(ref criterion) = params.filter` → `if !params.filter.is_empty()`
  - Change filter call: `filter::apply_filter(criterion, &snapshot)` → `filter::apply_filters(&params.filter, &snapshot)`
  - **Critical line** — Change NoMatchingItems cross-check (`any_match_in_snapshot`, ~line 686-689): Replace `snapshot.items.iter().any(|item| filter::matches_item(criterion, item))` with `snapshot.items.iter().any(|item| params.filter.iter().all(|c| filter::matches_item(c, item)))`. Note the `.any()` wraps `.all()` — this checks whether *any* item in the unfiltered snapshot matches *all* criteria. Getting `.any()` vs `.all()` wrong here would misclassify `NoMatchingItems` vs `FilterExhausted`.
  - Change log messages: use `filter::format_filter_criteria(&params.filter)`. For the NoMatchingItems message, use `"No items match combined filter criteria: {}"` when `params.filter.len() > 1`, or keep existing `"No items match filter criteria: {}"` when `params.filter.len() == 1` (preserves backward-compatible single-criterion output per PRD).
  - Change else branch: `else { None }` remains unchanged
- [ ] Remove `apply_filter()` from `src/filter.rs` (lines 135-150)
- [ ] Update `tests/filter_test.rs`:
  - Change import: `apply_filter` → `apply_filters` (also add `validate_filter_criteria, format_filter_criteria` if not already imported from Phase 1)
  - Migrate all `apply_filter(&f, &snapshot)` calls to `apply_filters(&[f], &snapshot)` — 16 call sites across 16 existing tests: `parse_and_match_status_in_progress`, `tag_filter_empty_tags_never_match`, `tag_filter_case_sensitive`, `tag_filter_exact_match`, `none_impact_never_matches`, `none_size_never_matches`, `none_risk_never_matches`, `none_complexity_never_matches`, `none_pipeline_type_never_matches`, `apply_filter_returns_matching_subset`, `apply_filter_empty_snapshot_returns_empty`, `apply_filter_preserves_schema_version`, `pipeline_type_case_sensitive_matching`, `pipeline_type_exact_match`, `status_filter_matches_correctly`, `apply_filter_preserves_next_item_id`. Use bulk find-and-replace: `apply_filter(&` → `apply_filters(&[` with corresponding closing `]`.
- [ ] Add new imports in `src/main.rs` for `filter::validate_filter_criteria`, `filter::apply_filters`, `filter::format_filter_criteria` (or use qualified `filter::` paths). Similarly ensure `src/scheduler.rs` can access `filter::apply_filters` and `filter::format_filter_criteria`.
- [ ] Update `tests/scheduler_test.rs`:
  - `run_params()` helper (line 197): `filter: None` → `filter: vec![]`
  - 16 inline `filter: None` occurrences → `filter: vec![]`
  - 5 inline `filter: Some(filter::parse_filter(...).unwrap())` occurrences → `filter: vec![filter::parse_filter(...).unwrap()]`
- [ ] Add multi-criteria scheduler integration tests in `tests/scheduler_test.rs`:
  - Test with `filter: vec![criterion_a, criterion_b]` where items match individual criteria but not the AND intersection → verify `NoMatchingItems` halt
  - Test with `filter: vec![criterion_a, criterion_b]` where matching items complete → verify `FilterExhausted` halt

**Verification:**

- [ ] `cargo build` succeeds
- [ ] `cargo test` passes (all existing tests pass with migrated call sites, no regressions)
- [ ] `apply_filter` is no longer exported from `filter.rs` (removed)
- [ ] Single `--only` backward compatibility: all 5 existing scheduler filter tests pass with `vec![criterion]` syntax; `format_filter_criteria` with one criterion produces no ` AND ` separator
- [ ] Verify `--only` and `--target` remain mutually exclusive: `conflicts_with = "target"` annotation present in CLI definition, runtime guard updated to `!only.is_empty()`
- [ ] Multi-criteria scheduler integration tests pass (new tests from this phase)
- [ ] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-055][P2] Feature: Migrate CLI, scheduler, and tests to multi-filter Vec<FilterCriterion>`

**Notes:**

- **Compiler-driven refactoring**: Start by making the type changes (`RunParams.filter`, CLI `only`), then systematically fix all compiler errors. Don't rely solely on the enumerated line numbers — search for the code patterns (e.g., `only: Option<String>`, `params.filter`) to find all sites.
- **Line numbers are approximate anchors**: Phase 1 adds functions to `filter.rs` which may shift line numbers within that file. `main.rs` and `scheduler.rs` line references should be stable since Phase 1 doesn't modify those files, but always search for the actual code pattern rather than trusting exact line numbers.
- The `RunParams.filter` type change is compile-breaking — `scheduler.rs`, `main.rs`, and `tests/scheduler_test.rs` must all be updated atomically.
- The `apply_filter()` removal is compile-breaking for `tests/filter_test.rs` — removal and test migration must happen together.
- The halt-reason display block in `main.rs` (lines 733-748) needs no structural changes because `filter_display` retains its `Option<String>` type.
- The `filtered_snapshot` variable in the scheduler retains its `Option<BacklogFile>` type — only the construction condition changes.

**Followups:**

- [ ] [Medium] If WRK-056 (comma-separated OR values) is descoped, update the duplicate scalar field error message to remove the OR syntax hint — deferred because WRK-056 is the planned next change.

---

## Final Verification

- [ ] All phases complete
- [ ] All PRD success criteria met:
  - [ ] `--only` accepts multiple values via repeated flag
  - [ ] Multiple criteria combined with AND logic
  - [ ] Items with `None` for filtered optional field are excluded
  - [ ] Each criterion validated at startup (fail-fast)
  - [ ] Duplicate scalar fields rejected with error (with WRK-056 hint)
  - [ ] `tag` field exempt from duplicate-field rejection; identical tag-value pairs rejected
  - [ ] Terminal output shows criteria joined by ` AND `
  - [ ] Single `--only` backward compatible
  - [ ] `--only` and `--target` mutually exclusive
  - [ ] Zero matches on initial snapshot → `NoMatchingItems` halt
  - [ ] All matching items done/blocked → `FilterExhausted` halt
  - [ ] No `--only` → empty Vec, unfiltered run
- [ ] Tests pass
- [ ] No regressions introduced
- [ ] Code reviewed

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|

## Followups Summary

### Critical

### High

### Medium

- [ ] If WRK-056 is descoped, update duplicate scalar field error message to remove OR syntax hint

### Low

## Design Details

### Key Types

No new types introduced. Existing types modified:

```rust
// src/filter.rs — Hash added to derives
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FilterField { Status, Impact, Size, Risk, Complexity, Tag, PipelineType }

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FilterValue { Status(ItemStatus), Dimension(DimensionLevel), Size(SizeLevel), Tag(String), PipelineType(String) }

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FilterCriterion { pub field: FilterField, pub value: FilterValue }

// src/scheduler.rs — Option → Vec
pub struct RunParams {
    pub targets: Vec<String>,
    pub filter: Vec<crate::filter::FilterCriterion>,  // was Option<FilterCriterion>
    pub cap: u32,
    pub root: PathBuf,
    pub config_base: PathBuf,
    pub auto_advance: bool,
}
```

### New Function Signatures

```rust
// src/filter.rs

/// Validates that parsed filter criteria don't contain duplicate scalar fields
/// or identical tag-value pairs. No-op on empty slice.
pub fn validate_filter_criteria(criteria: &[FilterCriterion]) -> Result<(), String>;

/// Filters backlog to items matching ALL criteria (AND composition).
/// Composes via repeated matches_item() calls.
pub fn apply_filters(criteria: &[FilterCriterion], backlog: &BacklogFile) -> BacklogFile;

/// Formats criteria for display, joining with " AND " separator.
/// Single criterion: no separator. Returns criteria display string.
pub fn format_filter_criteria(criteria: &[FilterCriterion]) -> String;
```

### Design Rationale

See Design doc for full rationale. Key decisions:
- **Vec over Option<Vec>**: Empty vec = no filter. Idiomatic Rust, avoids double-wrapping.
- **Separate validate function**: Keeps `parse_filter()` single-responsibility. Only one caller today; convention adequate.
- **Remove apply_filter()**: `apply_filters()` subsumes single-criterion case. Keeping both adds maintenance burden.
- **Shared format function**: `format_filter_criteria()` eliminates display duplication between main.rs and scheduler.rs.

## Self-Critique Summary

### Auto-fixes Applied (13)

1. Fixed "approximately 10 call sites" to "16 call sites across 16 tests" for `apply_filter` migration in `filter_test.rs`
2. Fixed approximate counts ("~16", "~5") to exact counts ("16", "5") for `scheduler_test.rs` migration
3. Added empty-slice test case for `format_filter_criteria()` (edge case for robustness)
4. Added non-adjacent duplicate scalar field test for `validate_filter_criteria()` (ordering-sensitive edge case)
5. Clarified tag duplicate error message format uses `Display` (e.g., `"Duplicate filter: tag=backend specified multiple times"`)
6. Strengthened "pub and importable" verification to reference concrete test compilation
7. Added explicit note about needed imports in `main.rs` and `scheduler.rs` as a task
8. Added compiler-driven refactoring note (make type changes first, fix compiler errors systematically)
9. Added line-number approximation note (search for code patterns, not exact line numbers)
10. Added conditional log message text for single vs multi-criteria (use "combined filter criteria" only when >1 criterion)
11. Added explicit `any_match_in_snapshot` critical-line note with exact replacement expression for the `.any()` wrapping `.all()` pattern
12. Added multi-criteria scheduler integration tests (2 tests: NoMatchingItems + FilterExhausted with AND intersection)
13. Strengthened Phase 2 verification steps (specific backward-compat checks, mutual-exclusion confirmation, multi-criteria test passing)

### Directional Items (1)

**WRK-056 hint in error message** — Multiple critics noted the forward reference to WRK-056's comma-separated OR syntax in the duplicate scalar field error message. If WRK-056 is descoped, the hint would mislead users. Decision: keep as specified in PRD "Should Have" — the PRD explicitly requires this hint, and WRK-056 is the planned next change. Tracked as Medium followup.

### Quality Items (6)

1. **Help text verification** — No explicit step to check `--help` output shows the `--only` flag as repeatable. Low risk since clap auto-generates help for `Append` args.
2. **validate_filter_criteria task granularity** — The parse loop + validation call is bundled in one task. Could be split for clarity, but task is well-specified with sub-bullets.
3. **Parse loop integration test** — No test for the fail-fast parse loop in `main.rs`. Difficult to unit test CLI-level logic; covered by manual verification during code review.
4. **Multi-attribute test fixture** — No builder helper for multi-attribute items. Inline field mutation is the existing pattern and is acceptable for ~6 tests.
5. **conflicts_with automated test** — No automated test that `--only` and `--target` conflict. The clap annotation + runtime guard provide sufficient safety; enhancement deferred.
6. **Terminal display format test** — The `[config] Filter: ... — N items match` output line is not unit-tested. Covered by code review and manual verification.

## Assumptions

Decisions made without human input during autonomous SPEC creation:

1. **Light mode selected** — Small change with clear design, familiar patterns, no unknowns. Two phases is appropriate.
2. **Two-phase structure** — Phase 1 (additive) / Phase 2 (migration) boundary chosen because it's the only natural point where the codebase compiles without dead code. The alternative of finer phases would create intermediate states that don't compile.
3. **apply_filter() coexists in Phase 1, removed in Phase 2** — This avoids compile breakage in Phase 1 while keeping Phase 2's migration atomic.
4. **apply_filter() removal is safe** — `apply_filters()` subsumes single-criterion behavior via `&[criterion]`. This is a binary crate (not a library), so there are no external consumers. All internal callers are identified and migrated in Phase 2.
5. **Empty-slice behavior for format_filter_criteria()** — Returns empty string rather than panicking, for robustness. Callers guard with `is_empty()` before calling, but defensive behavior is preferable.
6. **WRK-056 hint kept in error message** — PRD "Should Have" explicitly requires the OR syntax hint in the duplicate scalar field error message. If WRK-056 is descoped or ships with different syntax, the hint should be updated (tracked as Medium followup).
