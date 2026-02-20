# Design: Support Multiple --only Filters with AND Logic

**ID:** WRK-055
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-055_support-multiple-only-filters-with-and-logic_PRD.md
**Tech Research:** ./WRK-055_support-multiple-only-filters-with-and-logic_TECH_RESEARCH.md
**Mode:** Light

## Overview

Extend the `--only` CLI flag from single-value (`Option<String>`) to multi-value (`Vec<String>`) with AND composition. Each `--only KEY=VALUE` occurrence adds a filter criterion; items must match **all** criteria to be included. The design follows the existing `--target` pattern for CLI accumulation, composes filtering through repeated calls to `matches_item()`, and replaces `Option<FilterCriterion>` with `Vec<FilterCriterion>` throughout the stack. No new modules or data types are introduced; three new functions are added (`validate_filter_criteria()`, `apply_filters()`, `format_filter_criteria()`).

---

## System Design

### High-Level Architecture

The change touches four layers in a vertical slice through the existing filter pipeline:

```
CLI (main.rs)  →  Validation (filter.rs)  →  RunParams (scheduler.rs)  →  Filter Application (scheduler.rs + filter.rs)
                                          ↘  Display (main.rs via filter.rs formatting)
```

The existing `FilterCriterion`, `parse_filter()`, and `matches_item()` are reused as-is (no signature changes). New code is limited to:
1. A `validate_filter_criteria()` function for duplicate-field detection (including identical tag-value pairs)
2. An `apply_filters()` function for multi-criteria AND composition
3. A `format_filter_criteria()` function for consistent display formatting across all sites
4. Plumbing changes to pass `Vec<FilterCriterion>` instead of `Option<FilterCriterion>`

### Component Breakdown

#### CLI Argument Parsing (main.rs)

**Purpose:** Collect multiple `--only` values from the command line.

**Changes:**
- `only: Option<String>` → `only: Vec<String>` with `action = clap::ArgAction::Append`
- `conflicts_with = "target"` annotation remains (works with Append per tech research)
- Mutual exclusivity guard: `only.is_some()` → `!only.is_empty()`

**Interfaces:**
- Input: Raw CLI args
- Output: `Vec<String>` of `KEY=VALUE` strings

#### Filter Validation (filter.rs)

**Purpose:** Parse each raw string into `FilterCriterion` and detect invalid duplicate fields.

**Changes:**
- Existing `parse_filter()` is called once per `--only` value in order, with fail-fast semantics: the first parse error aborts immediately via `?` before any further parsing or validation occurs (no signature change to `parse_filter()`)
- New `validate_filter_criteria(criteria: &[FilterCriterion]) -> Result<(), String>` checks for duplicate scalar fields and identical tag-value pairs. Called only after all individual criteria parse successfully.
- Add `Hash` derive to `FilterField` to support `HashSet`-based duplicate detection. This is safe because `FilterField` is a fieldless enum — all variants are unit variants with no contained data, so auto-derived `Hash` is correct. No existing code has manual `Hash` impls or conflicting constraints.
- `validate_filter_criteria()` is placed in `filter.rs` because it operates on `FilterCriterion` types defined in that module, alongside `parse_filter()`. It keeps filter-related validation co-located.
- When called with an empty slice, `validate_filter_criteria()` is a no-op returning `Ok(())`.

**Interfaces:**
- Input: `&[FilterCriterion]` (parsed criteria)
- Output: `Result<(), String>` — Ok if valid, Err with message for duplicate scalar fields or identical tag pairs

**Duplicate detection logic:**
- Maintain a `HashSet<FilterField>` for scalar field uniqueness
- Maintain a `HashSet<&FilterCriterion>` (or equivalent) for identical-pair detection on tags
- For each criterion:
  - If `field != FilterField::Tag`: insert field into scalar HashSet; if already present → error
  - If `field == FilterField::Tag`: skip scalar HashSet check, but check if an identical `FilterCriterion` (same field AND same value) was already seen → if so, error
- This satisfies the PRD requirement that identical field+value pairs are rejected (e.g., `--only tag=backend --only tag=backend` → error) while allowing different tag values (e.g., `--only tag=backend --only tag=sprint-1` → ok)
- Error message for scalar duplicates includes WRK-056 hint per PRD "Should Have": `"Field '{field}' specified multiple times. For OR logic within a field, use comma-separated values: --only {field}=value1,value2"`
- Error message for identical tag pairs: `"Duplicate filter: tag={value} specified multiple times"`

**Note on `Hash` for duplicate detection:** `FilterCriterion` will also need `Hash` derived (it contains `FilterField` and `FilterValue`; `FilterValue` contains `String`, `ItemStatus`, `DimensionLevel`, `SizeLevel` — all of which support `Hash` or can be derived). This enables `HashSet<FilterCriterion>` for identical-pair detection on tags.

#### Filter Display (filter.rs)

**Purpose:** Provide a single, shared formatting function for multi-criteria display.

**Changes:**
- New `format_filter_criteria(criteria: &[FilterCriterion]) -> String` — joins criteria with ` AND ` separator; single criterion = no separator. Uses each criterion's existing `Display` impl.
- Both `main.rs` (startup display + halt-reason display) and `scheduler.rs` (halt condition log messages) call this function, ensuring consistent formatting across all display sites.

**Interfaces:**
- Input: `&[FilterCriterion]`
- Output: `String` (e.g., `"impact=high AND size=small"` or `"impact=high"` for single criterion)

#### RunParams (scheduler.rs)

**Purpose:** Carry parsed filter criteria to the scheduler.

**Changes:**
- `filter: Option<FilterCriterion>` → `filter: Vec<FilterCriterion>`
- Empty vec semantics = no filter (replaces `None`)

#### Filter Application (filter.rs)

**Purpose:** Apply multiple criteria with AND logic.

**Changes:**
- New `pub apply_filters(criteria: &[FilterCriterion], backlog: &BacklogFile) -> BacklogFile`
- Filters items where ALL criteria match: `criteria.iter().all(|c| matches_item(c, item))`
- **Empty-slice behavior:** An empty criteria slice is vacuously true — all items match (`.all()` on an empty iterator returns `true`). Callers should short-circuit on empty slice before calling (the scheduler does via `is_empty()` check), but the function is safe to call with an empty slice.
- Existing `apply_filter()` (single criterion) is removed as part of this change. `apply_filters()` handles the single-criterion case identically. See "Decision: Remove apply_filter()" below.

**Interfaces:**
- Input: Slice of criteria + backlog snapshot
- Output: Filtered `BacklogFile` containing only items matching all criteria

#### Scheduler Halt Logic (scheduler.rs)

**Purpose:** Detect halt conditions (NoMatchingItems, FilterExhausted) with multi-criteria.

The scheduler has two distinct halt paths that both need updating:

**Path 1: Empty filtered result (filtered.items.is_empty())**
- Entered when `apply_filters()` returns zero items
- Sub-path A (`NoMatchingItems`): No items match in the snapshot AND no prior progress. The `any_match_in_snapshot` cross-check changes from `snapshot.items.iter().any(|item| matches_item(criterion, item))` to `snapshot.items.iter().any(|item| params.filter.iter().all(|c| matches_item(c, item)))`. This uses inline AND composition rather than calling `apply_filters()` to avoid allocating a new `BacklogFile` just for an existence check.
- Sub-path B (`FilterExhausted`): Items matched previously but all are now done/blocked. Falls through when `any_match_in_snapshot || has_prior_progress`.
- Log message format: `[filter] No items match combined filter criteria: {format_filter_criteria(&params.filter)}` (NoMatchingItems) or `[filter] All items matching {format_filter_criteria(&params.filter)} are done or blocked.` (FilterExhausted)

**Path 2: All filtered items done/blocked (all_done_or_blocked check)**
- Entered when `apply_filters()` returns items but all have status Done or Blocked
- Always produces `FilterExhausted`
- Log message format: `[filter] All items matching {format_filter_criteria(&params.filter)} are done or blocked.`

**Note on `has_prior_progress` approximation:** The existing `has_prior_progress` check (`!state.items_completed.is_empty() || !state.items_blocked.is_empty()`) is coarser than "prior progress on items matching the current filter criteria" — it tracks all completed/blocked items across the run, not just those matching the filter. This is inherited from the existing single-criterion design and is an acceptable approximation: false positives (reporting FilterExhausted instead of NoMatchingItems when progress was on unfiltered items) are benign because the user still sees the correct final state.

**Other changes:**
- `if let Some(ref criterion) = params.filter` → `if !params.filter.is_empty()`
- `apply_filter(criterion, &snapshot)` → `apply_filters(&params.filter, &snapshot)`
- `filtered_snapshot` variable retains its type `Option<BacklogFile>` — only the construction condition changes (from `if let Some(...)` to `if !params.filter.is_empty()`). The action-selection dispatch at lines 759-763 is structurally unchanged.

#### Display (main.rs)

**Purpose:** Show active filter criteria in terminal output and run summary.

**Changes:**
- `filter_display` retains its type `Option<String>`. Construction changes from `parsed_filter.as_ref().map(|c| c.to_string())` to `if !parsed_filters.is_empty() { Some(format_filter_criteria(&parsed_filters)) } else { None }`. The `if let Some(ref filter_str) = filter_display` guards at the halt-reason display (lines 734-748) remain structurally unchanged.
- Startup log: `[config] Filter: {filter_display} — {count} items match (from {total} total)`. Uses `apply_filters()` on the initial loaded `backlog` (same timing as current code — before `prune_stale_dependencies`).
- Halt-reason display in run summary: uses `filter_display` string in `FilterExhausted` and `NoMatchingItems` messages. Format: `Filter: all items matching {filter_display} are done or blocked` (FilterExhausted) and `Filter: no items match {filter_display}` (NoMatchingItems).

### Data Flow

1. **CLI parsing**: clap collects repeated `--only` values into `Vec<String>`
2. **Parsing (fail-fast)**: Each string is parsed via `parse_filter()` in order; the first parse error aborts immediately via `?` (no further parsing or validation occurs). On success, produces `Vec<FilterCriterion>`.
3. **Cross-criterion validation**: `validate_filter_criteria()` checks for duplicate scalar fields and identical tag-value pairs. Error propagates via `?` through `handle_run()` → `main()` → `eprintln!("Error: ...")` + non-zero exit, consistent with how `parse_filter()` errors are handled today.
4. **Display**: Criteria formatted via `format_filter_criteria()` for startup log, match count computed via `apply_filters()` on the initially loaded `backlog`
5. **RunParams**: `Vec<FilterCriterion>` passed into scheduler
6. **Scheduler loop**: Each iteration applies `apply_filters()` to the per-loop coordinator-fetched `snapshot`, checks halt conditions on filtered result
7. **Run summary**: `filter_display` (`Option<String>`) used for halt-reason messages (FilterExhausted / NoMatchingItems)

### Key Flows

#### Flow: Multi-Filter Run (Happy Path)

> User runs `phase-golem run --only impact=high --only size=small` and items matching both criteria are processed.

1. **CLI collects** — clap accumulates `["impact=high", "size=small"]` into `only: Vec<String>`
2. **Parse each (fail-fast)** — `parse_filter("impact=high")` → Ok, `parse_filter("size=small")` → Ok → `Vec<FilterCriterion>` with 2 entries. If any parse fails, abort immediately with the parse error (see "Flow: Partial Parse Failure" below).
3. **Validate** — `validate_filter_criteria()` checks: `impact` field inserted into scalar HashSet (Ok), `size` field inserted (Ok, different field) → passes
4. **Display** — Log: `[config] Filter: impact=high AND size=small — 3 items match (from 47 total)`
5. **Scheduler runs** — Each loop: `apply_filters()` returns items matching BOTH; scheduler selects actions from filtered set
6. **Halt** — When all matching items reach done/blocked → `FilterExhausted` with display: `Filter: all items matching impact=high AND size=small are done or blocked`

**Edge cases:**
- Zero matches initially → `NoMatchingItems` halt with scheduler log: `[filter] No items match combined filter criteria: impact=high AND size=small` and run summary: `Filter: no items match impact=high AND size=small`
- Single `--only` value → Identical to current behavior (Vec of length 1, no `AND` in display)
- No `--only` → Empty Vec, scheduler runs unfiltered (same as today)

#### Flow: Partial Parse Failure

> User runs `--only impact=high --only stattus=ready` — second criterion has a typo, rejected at startup.

1. **CLI collects** — `["impact=high", "stattus=ready"]`
2. **Parse first** — `parse_filter("impact=high")` → Ok
3. **Parse second** — `parse_filter("stattus=ready")` → Err: `"Unknown filter field: stattus. Supported: status, impact, size, risk, complexity, tag, pipeline_type"`
4. **Abort** — Error propagates via `?` from `handle_run()`. The error message comes from `parse_filter()` unchanged — no positional context (e.g., "second --only argument") is added, consistent with existing single-filter behavior. The user sees the specific field/value that failed.

#### Flow: Duplicate Scalar Field Rejection

> User runs `--only impact=high --only impact=low` — rejected at startup.

1. **CLI collects** — `["impact=high", "impact=low"]`
2. **Parse each** — Both parse successfully
3. **Validate** — `impact` inserted into scalar HashSet (Ok), second `impact` insert returns false → Error: `"Field 'impact' specified multiple times. For OR logic within a field, use comma-separated values: --only impact=high,low"`
4. **Abort** — Error propagates via `?` from `handle_run()`, printed to stderr, non-zero exit. Same propagation path as parse errors.

**Note:** `pipeline_type` is treated as a scalar field (duplicate rejection applies) because `BacklogItem.pipeline_type` is `Option<String>` — each item has at most one pipeline type. This is consistent with status, impact, size, risk, and complexity, which are all single-valued per item.

#### Flow: Multi-Tag AND (Tag Exemption)

> User runs `--only tag=backend --only tag=sprint-1` — processes items with BOTH tags.

1. **CLI collects** — `["tag=backend", "tag=sprint-1"]`
2. **Parse each** — Both parse as `FilterField::Tag`
3. **Validate** — `Tag` field skipped in scalar HashSet check. Different tag values → no identical-pair match → passes
4. **Filter** — `matches_item()` for tag uses `item.tags.contains(target)`. Both criteria must return true (AND), so only items with both "backend" AND "sprint-1" tags pass
5. **Run proceeds** normally

**Edge case: Identical tag duplicates** — `--only tag=backend --only tag=backend` → rejected by identical-pair detection in `validate_filter_criteria()` with error: `"Duplicate filter: tag=backend specified multiple times"`. This satisfies the PRD requirement that identical field+value pairs are rejected.

---

## Technical Decisions

### Key Decisions

#### Decision: Vec<FilterCriterion> over Option<Vec<FilterCriterion>>

**Context:** Need to represent zero-or-more filter criteria on RunParams.

**Decision:** Use `Vec<FilterCriterion>` directly. Empty vec = no filter.

**Rationale:** Avoids double-wrapping (`Option<Vec>`). Empty vec has the same semantics as `None` and is idiomatic Rust. All check sites use `.is_empty()` instead of `.is_none()`.

**Consequences:** Every existing `if let Some(ref criterion) = params.filter` must change to `if !params.filter.is_empty()`. This is straightforward but touches multiple sites.

#### Decision: Separate validate_filter_criteria() Function

**Context:** Duplicate field detection is a new validation step between parsing and filter construction.

**Decision:** Add a standalone `validate_filter_criteria()` in `filter.rs` rather than embedding validation in parse logic or the caller.

**Rationale:** Keeps `parse_filter()` single-responsibility (parse one criterion). Validation of cross-criterion constraints is a separate concern. Testable in isolation. Placed in `filter.rs` because it operates on `FilterCriterion` types defined there.

**Consequences:** Callers must call `validate_filter_criteria()` after collecting all parsed criteria. Currently only `handle_run()` in `main.rs` is a caller. The two-step obligation (parse then validate) is enforced by convention, not the type system. If a future call site skips validation, duplicate scalar fields would produce a logically impossible AND filter that silently returns zero results with no diagnostic.

#### Decision: Remove apply_filter() in This Change

**Context:** The existing `apply_filter()` (single criterion) is superseded by `apply_filters()` (multi-criteria).

**Decision:** Remove `apply_filter()` as part of this change. `apply_filters()` handles the single-criterion case identically (`.all()` on a single-element slice returns the single result).

**Rationale:** Keeping both functions adds maintenance burden with no benefit. The multi-criteria version subsumes the single-criterion case. `apply_filter()` is `pub` in `filter.rs` but only consumed internally (this is a binary crate, not a library crate). Confirmed callers: `main.rs` (startup display), `scheduler.rs` (filter application), and `tests/filter_test.rs` (~10 tests directly import and call `apply_filter`). All must migrate to `apply_filters()`.

**Consequences:** `tests/filter_test.rs` tests that call `apply_filter()` must be updated to call `apply_filters(&[criterion], &backlog)` — functionally identical, all existing assertions remain valid.

#### Decision: Shared format_filter_criteria() Function

**Context:** Multiple sites format filter criteria for display — startup log, scheduler halt logs, and run summary.

**Decision:** Add `format_filter_criteria()` in `filter.rs` as the single source of truth for multi-criteria display formatting.

**Rationale:** Eliminates risk of inconsistent formatting between `main.rs` and `scheduler.rs`. The join format (` AND ` separator, no separator for single criterion) is defined once.

**Consequences:** Both `main.rs` and `scheduler.rs` call this function instead of doing ad-hoc string formatting.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Plumbing churn | ~20+ call sites change from Option to Vec pattern (including tests) | Clean, uniform API without Option<Vec> wrapping | One-time migration cost; Rust compiler catches all missed sites |
| Removing apply_filter | All callers (including ~10 tests) must migrate to apply_filters | Single entry point for all filtering | apply_filters handles single-criterion identically |
| Parse/validate two-step | Callers must remember to call both parse and validate | Simple function signatures, no new types | Only one call site today (handle_run); convention is adequate for current scope |

---

## Alternatives Considered

### Alternative: Option<Vec<FilterCriterion>>

**Summary:** Keep `Option` wrapping around the Vec to preserve the `None` vs `Some(empty)` distinction.

**How it would work:**
- `None` = no filter specified
- `Some(vec)` = filter active with 1+ criteria

**Pros:**
- Distinguishes "no flag given" from "flag given but empty" at type level

**Cons:**
- Double-wrapping adds complexity with no practical benefit (empty Vec and None have identical runtime behavior)
- More verbose pattern matching at every call site

**Why not chosen:** Empty Vec is semantically equivalent to "no filter" and is the idiomatic Rust pattern for "zero or more." The `--only` flag with Append action never produces `Some(empty_vec)` — it's either absent (empty Vec) or has 1+ values.

### Alternative: ValidatedFilters Newtype

**Summary:** Wrap `Vec<FilterCriterion>` in a newtype `ValidatedFilters(Vec<FilterCriterion>)` that can only be constructed through a function that both parses and validates.

**How it would work:**
- `parse_and_validate_filters(raws: &[String]) -> Result<ValidatedFilters, String>`
- `RunParams.filter: ValidatedFilters` (or `Option<ValidatedFilters>`)
- Only the constructor function can create a `ValidatedFilters`, making it impossible to skip validation

**Pros:**
- Type-system enforcement — impossible to forget validation
- Single entry point collapses two-step caller obligation

**Cons:**
- Adds a new type to the codebase for a single call site
- Makes test construction of filter criteria slightly more verbose
- Over-engineers the current need (only `handle_run()` constructs filters)

**Why not chosen:** Only one call site exists today. The maintenance cost of the two-step convention is minimal. If a second call site is added in the future, introducing the newtype at that point is straightforward.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Missed call site in Option→Vec migration | Compile error (not a runtime risk) | Low | Rust compiler catches all pattern match exhaustiveness and type mismatches |
| `conflicts_with` behavior change with Append | --only and --target could be combined | Low | Tech research confirmed it works; runtime guard remains as safety net. Add integration test asserting the conflict. |
| `apply_filter()` removal breaks tests | Compile errors in filter_test.rs (~10 tests) | Certain | Migrate all test call sites to `apply_filters(&[criterion], &backlog)` as part of this change |

---

## Integration Points

### Existing Code Touchpoints

- `src/main.rs` lines 61-62 — CLI arg definition: `Option<String>` → `Vec<String>` with Append
- `src/main.rs` line 333 — `handle_run` signature: `only: Option<String>` → `only: Vec<String>`
- `src/main.rs` line 363 — Mutual exclusion guard: `only.is_some()` → `!only.is_empty()`
- `src/main.rs` lines 419-422 — Filter parsing: iterate and parse each with fail-fast `?`, then call `validate_filter_criteria()`
- `src/main.rs` lines 456-464 — Startup display: use `apply_filters()` on the initially loaded `backlog`, use `format_filter_criteria()` for display string
- `src/main.rs` line 632 — `filter_display`: `Option<String>` built from `if !parsed_filters.is_empty() { Some(format_filter_criteria(...)) } else { None }`
- `src/main.rs` lines 634-636 — RunParams construction: `filter: parsed_filters` (now Vec)
- `src/main.rs` lines 734-748 — Halt-reason display: `filter_display` `if let Some` guards remain structurally unchanged
- `src/filter.rs` — Add `Hash` to `FilterField` and `FilterCriterion` derives, add `validate_filter_criteria()`, add `apply_filters()`, add `format_filter_criteria()`, remove `apply_filter()`
- `src/scheduler.rs` line 52 — RunParams: `Option<FilterCriterion>` → `Vec<FilterCriterion>`
- `src/scheduler.rs` lines 678-748 — Filter application block: two halt paths (empty result + all-done-or-blocked), replace Option matching with Vec emptiness checks, use `apply_filters()` and `format_filter_criteria()`
- `tests/filter_test.rs` — Migrate ~10 tests from `apply_filter()` to `apply_filters(&[criterion], &backlog)` (all existing assertions remain valid). Add new tests for multi-criteria filtering, `validate_filter_criteria()`, and `format_filter_criteria()`.
- `tests/scheduler_test.rs` — Update `run_params()` helper and ~20+ inline `RunParams` constructions: `filter: None` → `filter: vec![]`, `filter: Some(x)` → `filter: vec![x]`

### External Dependencies

None. No new crates or external services.

---

## Open Questions

None. The design follows directly from the PRD requirements and tech research findings.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain (or they're flagged for spec phase)

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Initial design draft (light mode) | Complete design following existing patterns |
| 2026-02-20 | Self-critique (7 agents), triage, auto-fixes | 16 auto-fixes applied, 2 directional items noted, 1 quality item addressed |

## Self-Critique Summary

### Auto-fixes Applied (16)

1. Fixed overview to accurately describe scope (3 new functions, not "no new code")
2. Normalized `apply_filter()` removal as firm decision with explicit caller enumeration
3. Added identical tag-value duplicate detection (satisfies PRD identical-pair requirement)
4. Clarified parse-then-validate ordering as sequential with fail-fast semantics
5. Added "Partial Parse Failure" flow for multi-value error handling
6. Documented error propagation path (via `?` through `handle_run()`)
7. Specified `filter_display` retains `Option<String>` type with explicit construction
8. Documented `apply_filters()` empty-slice behavior (vacuous truth)
9. Confirmed `filtered_snapshot` retains `Option<BacklogFile>` type
10. Distinguished two FilterExhausted halt paths in scheduler (empty result vs all-done-or-blocked)
11. Documented `pipeline_type` single-valued assumption with BacklogItem field reference
12. Noted `has_prior_progress` approximation as inherited from existing design
13. Added concrete log message formats in scheduler halt logic section
14. Noted `Hash` derive safety for `FilterField` (fieldless enum)
15. Listed `tests/filter_test.rs` as explicit migration target with ~10 test call sites
16. Added shared `format_filter_criteria()` function to eliminate display duplication

### Directional Items (2)

**1. ValidatedFilters newtype** — Multiple critics noted the parse/validate two-step has no type-system enforcement. A `ValidatedFilters(Vec<FilterCriterion>)` newtype would make it impossible to skip validation. Decision: deferred. Only one call site exists today; convention is adequate. Documented as an alternative with rationale.

**2. WRK-056 forward-reference in error message** — One critic noted the duplicate-field error message references WRK-056 syntax (`--only impact=high,low`) which doesn't exist yet. If WRK-056 ships with different syntax or is descoped, the hint is misleading. Decision: keep as specified in PRD "Should Have" — the PRD explicitly requires this hint, and WRK-056 is the planned next change. The spec should note that if WRK-056 is descoped, this message should be updated.

### Quality Items (1)

**1. Shared display function** — Multiple critics noted display formatting is duplicated between `main.rs` and `scheduler.rs`. Addressed by adding `format_filter_criteria()` as auto-fix #16.

## Assumptions

Decisions made without human input during autonomous design:

1. **Light mode selected** — The PRD is thorough with specific line-number references, tech research confirmed all assumptions, and the implementation follows an exact existing pattern (`--target`). There are no meaningful alternative architectures to explore.
2. **Remove apply_filter() as part of this change** — The multi-criteria `apply_filters()` cleanly subsumes the single-criterion case. All callers (including ~10 tests) are identified and must migrate.
3. **validate_filter_criteria() as separate function** — Cross-criterion validation (duplicate field detection) is a distinct concern from single-criterion parsing. Separating it keeps `parse_filter()` focused and makes the validation independently testable. Placed in `filter.rs` alongside the types it operates on.
4. **Add format_filter_criteria() for shared display** — Multiple display sites need consistent formatting of multi-criteria. A shared function eliminates divergence risk.
5. **Identical tag-value pairs rejected** — The PRD requires all identical field+value pairs to be rejected. The tag exemption from duplicate *field* detection does not exempt identical tag+value pairs. Implemented via separate identical-pair check in `validate_filter_criteria()`.
6. **Keep WRK-056 hint in error message** — PRD "Should Have" explicitly requests this. The hint references planned future syntax; if WRK-056 is descoped, the message should be updated.
