# SPEC: Support Comma-Separated OR Values Within --only Filter

**ID:** WRK-056
**Status:** Draft
**Created:** 2026-02-20
**PRD:** ./WRK-056_support-comma-separated-or-values-within-only-filter_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

WRK-055 introduced `Vec<FilterCriterion>` with AND logic across multiple `--only` flags. Its error message for duplicate scalar fields already promises comma-separated OR syntax: `"For OR logic within a field, use comma-separated values: --only impact=high,low"`. WRK-056 implements that promise. The syntax and semantics are predetermined — comma splits within a value portion produce OR logic within a single field, composing with the existing cross-field AND.

## Approach

Modify `FilterCriterion.value: FilterValue` to `FilterCriterion.values: Vec<FilterValue>`. Extract two private helpers (`parse_single_value()`, `matches_single_value()`) from the existing inline logic in `parse_filter()` and `matches_item()`. Add a `Display` impl on `FilterValue`. Update `parse_filter()` to split on commas, validate empty tokens and within-list duplicates, and return multi-value criteria. Update `matches_item()` to iterate with `.any()` for OR semantics. Update the scalar-field duplicate error message and `--only` help text. Single-value usage produces a one-element `Vec`, preserving full backward compatibility.

The change follows the same vertical-slice pattern as WRK-055, touching the same layers: CLI (main.rs) → Parsing (filter.rs) → Validation (filter.rs) → Matching (filter.rs) → Display (filter.rs).

**Patterns to follow:**

- `src/filter.rs` — existing `parse_filter()`, `matches_item()`, `validate_filter_criteria()`, `Display for FilterCriterion`, `Display for FilterField` patterns
- `tests/filter_test.rs` — existing test structure: parse tests, matching tests, validation tests, display tests, roundtrip tests

**Implementation boundaries:**

- Do not modify: `src/types.rs`, `src/scheduler.rs`, `tests/scheduler_test.rs`, `tests/common/mod.rs`
- Do not refactor: existing test structure beyond what's required for the `.value` → `.values` migration
- Do not add: negation filters, glob matching, range filters, or any syntax beyond comma-separated OR

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Structural Refactor | Low | Rename `value` to `values: Vec<FilterValue>`, extract helpers, add `FilterValue::Display`, migrate existing tests — identical behavior |
| 2 | Multi-Value OR Functionality | Med | Add comma splitting, OR matching, duplicate detection, validation message update, help text, new tests |

**Ordering rationale:** Phase 1 restructures the code while keeping all behavior identical (single-value criteria become one-element Vecs). Phase 2 adds new functionality on top of the restructured foundation. This boundary ensures Phase 1 can be verified by running existing tests — all must pass with zero behavior change.

---

## Phases

### Phase 1: Structural Refactor

> Rename `value` to `values: Vec<FilterValue>`, extract helpers, add `FilterValue::Display`, migrate existing tests — identical behavior

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Restructure `FilterCriterion` and internal helpers so multi-value support can be added in Phase 2, while keeping all existing behavior identical. After this phase, every existing test passes and the codebase compiles.

**Files:**

- `src/filter.rs` — modify — rename field, extract helpers, add `FilterValue::Display`, update `FilterCriterion::Display`
- `tests/filter_test.rs` — modify — migrate 14 `.value` assertions to `.values` (mechanical)

**Patterns:**

- Follow existing `Display for FilterField` impl (line 34-47) as template for `Display for FilterValue`
- Follow existing `matches_item()` body (lines 143-162) — extract unchanged into `matches_single_value()`
- Follow existing per-field match arms in `parse_filter()` (lines 81-138) — extract unchanged into `parse_single_value()`

**Tasks:**

- [ ] Rename `FilterCriterion.value: FilterValue` to `FilterCriterion.values: Vec<FilterValue>` (line 28-32)
- [ ] Add `impl Display for FilterValue` — extract value-to-string logic from current `FilterCriterion::Display` (lines 51-64). Status variant uses lowercase string mapping; Dimension/Size use their existing `Display` impls; Tag/PipelineType use the string directly.
- [ ] Update `impl Display for FilterCriterion` — iterate `self.values`, call `.to_string()` on each via `FilterValue::Display`, join with `,`, write `field=joined_values`. Single-value case produces identical output (no comma).
- [ ] Extract `fn parse_single_value(field: &FilterField, token: &str) -> Result<FilterValue, String>` — private helper containing the per-field value-parsing branches from `parse_filter()` (lines 82-138). The field name dispatch (`match field_str.to_lowercase()...`) stays in `parse_filter()`; only the value-parsing logic (e.g., `parse_dimension_level(token)`, `parse_item_status(token)`) moves into `parse_single_value()`. No behavior change.
- [ ] Update `parse_filter()` — call `parse_single_value()` for the single token, wrap result in `vec![value]` to construct `FilterCriterion { field, values: vec![value] }`. The existing `value_str.is_empty()` check (line 77) is retained before calling `parse_single_value()`. No comma splitting yet — that's Phase 2.
- [ ] Extract `fn matches_single_value(field: &FilterField, value: &FilterValue, item: &BacklogItem) -> bool` — private helper containing the exact match body from current `matches_item()` (lines 144-161). No behavior change.
- [ ] Update `matches_item()` — replace body with `criterion.values.iter().any(|v| matches_single_value(&criterion.field, v, item))`. For single-element Vec, `.any()` is equivalent to the current direct match.
- [ ] Migrate 14 `.value` assertion sites in `tests/filter_test.rs` to `.values` — pattern: `assert_eq!(f.value, FilterValue::X(y))` → `assert_eq!(f.values, vec![FilterValue::X(y)])`. Sites: lines 25, 32, 39, 46, 53, 60, 67, 147, 153, 166, 174, 180, 188, 441.

**Verification:**

- [ ] `cargo build` succeeds with no errors or warnings
- [ ] `cargo test` — all existing tests pass (zero behavior change)
- [ ] No changes to public API signatures: `parse_filter()`, `matches_item()`, `validate_filter_criteria()`, `apply_filters()`, `format_filter_criteria()` all retain identical signatures
- [ ] Display roundtrip test (`filter_criterion_display_roundtrip`) still passes
- [ ] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-056][P1] Clean: Restructure FilterCriterion for multi-value support`

**Notes:**

- The `validate_filter_criteria()` function accesses `criterion.field` and uses `criterion` (the whole struct) for tag dedup via `HashSet<&FilterCriterion>`. It does not access `.value` directly, so only the Hash/Eq derivation matters (which `Vec<FilterValue>` supports). However, the function needs no logic changes — just compiles against the new field name.
- The `apply_filters()` function only passes criteria through to `matches_item()` — no direct `.value` access, no changes needed.
- `format_filter_criteria()` calls `c.to_string()` which uses `FilterCriterion::Display` — updated in this phase.
- The `scheduler.rs` inlined `matches_item` call (line 690) works transparently after this phase's refactor — the public `matches_item()` signature is unchanged, and `.any()` on a single-element Vec is equivalent to the former direct match. No scheduler changes needed.
- Phase 1 must not ship independently without Phase 2. Between Phase 1 and Phase 2, `values` is always a one-element Vec, which is a temporary structural state. Phase 2 completes the feature.

**Followups:**

---

### Phase 2: Multi-Value OR Functionality

> Add comma splitting, OR matching, duplicate detection, validation message update, help text, new tests

**Phase Status:** not_started

**Complexity:** Med

**Goal:** Implement comma-separated OR values: parsing splits on commas, matching uses OR logic, validation detects within-list duplicates and updates error messages, help text documents the syntax. All PRD success criteria are met.

**Files:**

- `src/filter.rs` — modify — extend `parse_filter()` with comma splitting/validation, update `validate_filter_criteria()` error message
- `src/main.rs` — modify — update `--only` help text (line 60)
- `tests/filter_test.rs` — modify — add ~25 new test functions

**Tasks:**

- [ ] Update `parse_filter()` to split `value_str` on `,` after the existing `value_str.is_empty()` check. For each token: trim whitespace, reject if empty with error `"Empty value in comma-separated list for field '{field}'. Each value must be non-empty."`, parse via `parse_single_value()`. Collect into `Vec<FilterValue>`.
- [ ] Add within-list duplicate detection in `parse_filter()`: after parsing all tokens, iterate over the collected values inserting `&FilterValue` refs into a `HashSet`. If insertion returns false, error: `"Duplicate value '{raw_token}' in comma-separated list for field '{field}'"`. Use the raw token string (after trim) in the error message for user clarity, not the Debug representation of the parsed value. Implementation hint: collect into `Vec<(String, FilterValue)>` pairing each trimmed raw token with its parsed value, then iterate for duplicate checking while preserving access to the raw token for error messages. `FilterValue` already derives `Hash` + `Eq`, so `HashSet<&FilterValue>` works directly.
- [ ] Update `validate_filter_criteria()` scalar-field duplicate error message (line 178) from `"Field '{}' specified multiple times. For OR logic within a field, use comma-separated values: --only {}=value1,value2"` to `"Field '{}' specified multiple times in separate --only flags. Combine values in a single flag: --only {}=value1,value2"`.
- [ ] Update the `--only` help text in `src/main.rs` (line 60) to: `"Filter items by attribute. Comma-separated values = OR within field; repeated flags = AND across fields. Examples: --only impact=high,medium --only size=small (high or medium impact AND small size). Tag: --only tag=a,b (has either) vs --only tag=a --only tag=b (has both)."`
- [ ] Add tests for multi-value parsing (happy path): `parse_filter("impact=high,medium")` returns `values: [Dimension(High), Dimension(Medium)]`; `parse_filter("status=ready,blocked")` returns correct Status values; `parse_filter("tag=a,b")` returns correct Tag values; `parse_filter("pipeline_type=feature,bugfix")` returns correct PipelineType values.
- [ ] Add tests for empty token rejection: `"impact=high,,low"` (middle), `"impact=,high"` (leading), `"impact=high,"` (trailing), `"impact=,"` (comma only). Each should error with `"Empty value in comma-separated list"`.
- [ ] Add tests for within-list duplicate rejection: `"impact=high,high"` rejected; `"impact=high,HIGH"` rejected (case-insensitive enum parsing → same variant); `"tag=a,a"` rejected (case-sensitive, same string); `"tag=a,A"` accepted (case-sensitive, different strings).
- [ ] Add tests for multi-value OR matching: an item with `impact=High` matches `parse_filter("impact=high,medium")`; an item with `impact=None` does not match; multi-value OR composes with cross-field AND: `impact=high,medium` AND `size=small` filters correctly. Also add matching tests for `size=small,medium` (covers `FilterValue::Size` variant) and `pipeline_type=feature,bugfix` (covers free-text `FilterValue::PipelineType` variant).
- [ ] Add tests for multi-value display: `parse_filter("impact=high,medium").to_string()` == `"impact=high,medium"`; `format_filter_criteria` with multi-value + single-value criteria produces `"impact=high,medium AND size=small"`.
- [ ] Add tests for tag OR + AND composition: `tag=a,b` matches item with tag "a" or tag "b"; `[parse_filter("tag=a,b"), parse_filter("tag=c")]` filters items that have (a or b) AND c.
- [ ] Add test for whitespace trimming (Should Have): `parse_filter("impact=high, medium")` (with space after comma) parses same as `"impact=high,medium"`.
- [ ] Add multi-value roundtrip test: `parse_filter("impact=high,medium")` → `.to_string()` → `parse_filter()` → assert equal. Note: the PRD states roundtrip is "not guaranteed or required" (Out of Scope), but this test is intentionally added because the display format `field=v1,v2` happens to be valid input format and we want to preserve this property.
- [ ] Add test for invalid value within comma list: `parse_filter("size=small,huge")` → error containing `"Invalid value 'huge' for field 'size'"`. This verifies PRD Must Have #5 in the multi-value context.
- [ ] Add test for fail-fast ordering: `parse_filter("impact=high,huge,medium")` → error references `huge` (the first invalid token), not `medium`. Verifies PRD Must Have #4 (fail-fast, left-to-right).
- [ ] Add test for cross-flag duplicate validation with multi-value criteria: `validate_filter_criteria(&[parse_filter("impact=high,medium").unwrap(), parse_filter("impact=low").unwrap()])` → error. Verifies PRD Must Have #7 (scalar field duplicate detection applies even when one criterion is multi-valued).
- [ ] Add test for identical multi-value tag criteria across flags: `validate_filter_criteria(&[parse_filter("tag=a,b").unwrap(), parse_filter("tag=a,b").unwrap()])` → error (identical tag criteria rejected). Verifies tag identical-pair detection works with multi-value criteria.
- [ ] Add test for tag with equals + commas interaction: `parse_filter("tag=key=val1,key=val2")` → `values: [Tag("key=val1"), Tag("key=val2")]`. Verifies comma splitting interacts correctly with the first-`=`-only split.
- [ ] Verify existing `validate_duplicate_scalar_field_returns_err` test still passes. The existing assertions check for substrings `"Field 'impact' specified multiple times"` and `"--only impact=value1,value2"` — both appear in the updated message. Additionally, add a new assertion in a new test that checks for `"in separate --only flags"` to verify the message was actually updated (this substring distinguishes the new message from the old).

**Verification:**

- [ ] `cargo build` succeeds with no errors or warnings
- [ ] `cargo test` — all tests pass (existing + new)
- [ ] Manual smoke test: `cargo run -- run --help` shows updated `--only` help text containing "Comma-separated values = OR within field" and "repeated flags = AND across fields"
- [ ] All PRD Must Have criteria addressed (comma-separated OR, cross-field AND, validation, display, help text)
- [ ] All PRD Should Have criteria addressed (whitespace trimming)
- [ ] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-056][P2] Feature: Comma-separated OR values within --only filter`

**Notes:**

- The `tag=key=value` test (line 437-442) continues to work: the `=` in the tag value is not affected by comma splitting. `parse_filter("tag=key=value")` splits on first `=` → `("tag", "key=value")`, then splits `"key=value"` on `,` → `["key=value"]` (no comma), parses as single tag. No change needed.
- The existing empty-value check (`value_str.is_empty()` at line 77) catches `--only impact=` before comma splitting. Only `--only impact=,` reaches the comma-splitting path (`value_str` is `","`, not empty), where the empty-token check catches it. Expected error: `"Empty value in comma-separated list for field 'impact'. Each value must be non-empty."`
- Two-pass ordering: "fail-fast" (PRD Must Have #4) applies to parsing errors — the first invalid token aborts immediately. Duplicate detection is a separate post-parse step over the fully-parsed Vec. This means `"impact=huge,huge"` errors on invalid value (first pass), while `"impact=high,high"` errors on duplicate (second pass). Both behaviors are correct per the PRD.
- Tag duplicate detection across flags uses `Vec<FilterValue>` equality, which is order-dependent: `--only tag=a,b --only tag=b,a` passes validation despite being semantically redundant. This is an accepted limitation documented in the Design doc (see "Tag duplicate detection is order-sensitive" risk). Adding value sorting for canonicalization is not worth the complexity.

**Followups:**

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

### High

### Medium

### Low

## Design Details

### Key Types

```rust
// FilterCriterion (modified)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FilterCriterion {
    pub field: FilterField,
    pub values: Vec<FilterValue>,  // was: value: FilterValue
}

// FilterValue (unchanged, gains Display impl)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FilterValue {
    Status(ItemStatus),
    Dimension(DimensionLevel),
    Size(SizeLevel),
    Tag(String),
    PipelineType(String),
}

// New private helpers (not public API)
fn parse_single_value(field: &FilterField, token: &str) -> Result<FilterValue, String>
fn matches_single_value(field: &FilterField, value: &FilterValue, item: &BacklogItem) -> bool
```

### Architecture Details

Data flow is unchanged from WRK-055: CLI → parse → validate → display/match. The only internal change is that `FilterCriterion` carries a Vec of values, and matching uses `.any()` instead of a direct comparison. All public function signatures remain the same. `parse_filter()` is the only entry point for constructing `FilterCriterion`, and it enforces the invariants: non-empty values, no duplicates, all values match the field type.

### Design Rationale

See Design doc for full rationale. Key decisions:
- `Vec<FilterValue>` over wrapper type: simpler, one type to understand, compiler-enforced migration
- Within-list validation in `parse_filter()`: FilterCriterion valid by construction
- Helper extraction: enables per-token/per-value iteration without changing public API
- Two-pass duplicate detection: parse all tokens first (fail-fast on invalid), then check duplicates (avoids borrow-checker complexity)

---

## Self-Critique Summary

### Auto-fixes Applied (16)

1. Fixed help text smoke test command (`cargo run -- run --help` instead of `--only impact=high,medium --help`)
2. Added missing test: invalid value within comma list (`size=small,huge` → error)
3. Added missing test: fail-fast ordering (`impact=high,huge,medium` → error references `huge`)
4. Added missing test: cross-flag duplicate validation with multi-value criteria
5. Added missing test: identical multi-value tag criteria across flags
6. Added missing test: size and pipeline_type multi-value OR matching for variant coverage
7. Added missing test: tag with equals + commas interaction (`tag=key=val1,key=val2`)
8. Added new error message distinguishing assertion (`"in separate --only flags"`)
9. Clarified `parse_single_value()` scope — field name dispatch stays in `parse_filter()`, only value-parsing moves
10. Added implementation hint for raw token preservation in duplicate detection (Vec of pairs)
11. Added note that `FilterValue` already derives `Hash` + `Eq` for `HashSet` usage
12. Moved scheduler transparency note from Phase 2 to Phase 1 (where the refactor happens)
13. Added Phase 1/2 coupling note — Phase 1 must not ship independently
14. Clarified `impact=,` error path — `","` is not empty, reaches comma splitting
15. Added two-pass ordering note — fail-fast for parsing, then duplicate check
16. Noted multi-value roundtrip test as intentional promotion beyond PRD minimum

### Directional Items (0)

All concerns raised by critics were resolved via auto-fixes or by referencing existing Design decisions (tag dedup order-sensitivity, comma-in-tag behavioral change). No items require human input.

### Quality Items (0)

Remaining quality items from critics (e.g., end-to-end scheduler integration test, halt message display verification, PRD-to-test traceability table, automated help text test) were evaluated and determined to be out of scope for this small/low-complexity change. The SPEC's unit-test coverage is comprehensive, the scheduler path is verified transitively through `matches_item()` signature preservation, and halt message display propagates automatically through `format_filter_criteria()`.

---

## Assumptions

Decisions made without human input during autonomous SPEC creation:

1. **Light mode selected** — Direct follow-up to WRK-055 with syntax already specified in error message. Two analysis agents (File & Change Analyzer, Dependency Mapper) were sufficient.
2. **Two-phase structure** — Phase 1 (structural refactor, zero behavior change) and Phase 2 (new functionality). Natural boundary verified by existing test suite. No finer split needed for a small change.
3. **Scheduler integration test skipped** — The scheduler calls `matches_item()` whose signature is unchanged. Unit tests through `apply_filters()` exercise the same code path. Adding scheduler-level multi-value tests would be scope creep beyond the implementation boundaries.
4. **~25 new test functions in Phase 2** — Comprehensive coverage of all PRD Must Have and Should Have criteria, plus edge cases (tag+equals+commas, fail-fast ordering, error message distinguishing).
5. **Tag dedup order-sensitivity accepted** — Referenced Design doc decision rather than introducing canonicalization.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
