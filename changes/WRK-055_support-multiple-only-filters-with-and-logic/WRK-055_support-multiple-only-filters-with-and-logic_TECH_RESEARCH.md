# Tech Research: Support Multiple --only Filters with AND Logic

**ID:** WRK-055
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-055_support-multiple-only-filters-with-and-logic_PRD.md
**Mode:** Light

## Overview

Research how to extend the `--only` CLI flag from single-value (`Option<String>`) to multi-value (`Vec<String>`) with AND composition, following the existing `--target` pattern. The codebase already has all the building blocks; this research verifies the approach and identifies integration points.

## Research Questions

- [x] How does clap handle `Vec<String>` with `ArgAction::Append` and `conflicts_with`?
- [x] What existing patterns does the codebase use for multi-value flags?
- [x] Where are all the integration points for the filter change?
- [x] Are there any gotchas with the `Option` → `Vec` migration?

---

## External Research

### Landscape Overview

Repeated CLI flags with AND composition is a well-established pattern. The dominant Rust approach is clap's `ArgAction::Append` combined with `Vec<T>` derive fields. This project's `--target` flag already uses this exact pattern. The design space cleanly separates flag-level AND (repeated flags, WRK-055) from value-level OR (comma-separated values, WRK-056).

### Common Patterns & Approaches

#### Pattern: Repeated Flag Accumulation (Append) with Application-Level AND

**How it works:** Each `--flag value` occurrence appends to a `Vec<T>`. The application applies all collected values as an intersection (AND).

```rust
#[arg(long, action = clap::ArgAction::Append)]
filter: Vec<String>,
// Usage: --filter status=ready --filter impact=high
// Collected as: ["status=ready", "impact=high"]
// App logic: item must match ALL
```

**When to use:** Multi-dimensional data filtering where each flag invocation adds an independent constraint.

**Tradeoffs:**
- Pro: Natural UX — repeated flags reads as "and also require this"
- Pro: CLI order preserved for display
- Pro: Cleanly separates from OR-within-field (WRK-056)
- Con: Verbose for many OR values of the same field (addressed by WRK-056)

**References:**
- [clap derive tutorial — multi-value options](https://docs.rs/clap/latest/clap/_derive/_tutorial/index.html)
- [clap ArgAction::Append discussion](https://github.com/clap-rs/clap/discussions/4331)

#### Pattern: Single Flag with value_delimiter (Out of Scope)

Uses `value_delimiter = ','` for `--filter status=ready,impact=high`. Explicitly out of scope — this is the basis for WRK-056's OR logic and would conflict if mixed here.

### Technologies & Tools

No new dependencies needed. clap v4 with `ArgAction::Append` is the only tool, already in use for `--target`.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Plain `Vec<String>` without explicit `action = Append` | clap v4 may accept both `--only a b` and `--only a --only b` forms | Always use `action = clap::ArgAction::Append` explicitly |
| `Vec<T>` is optional by default | Empty vec when flag absent (not an error) | This is desired behavior — empty vec = no filter |
| `conflicts_with` string must match arg name exactly | Silent failure if mismatched | Verify string matches the field name (`"target"`) |
| `num_args > 1` consuming positional args | Following args consumed as values | `Append` with default `num_args = 1` avoids this |

### Key Learnings

- The `--target` pattern in this codebase is the exact template to follow
- `conflicts_with` works correctly with `Append` — fires when conflicting arg is present in any quantity
- Empty `Vec` from absent flag is the desired "no filter" semantics

---

## Internal Research

### Existing Codebase State

The filter architecture is clean and modular with clear separation:

1. **CLI parsing** (`main.rs`): `only: Option<String>` → will become `Vec<String>`
2. **Filter validation** (`filter.rs`): `parse_filter()` handles single `KEY=VALUE` strings
3. **Filter application** (`filter.rs`): `apply_filter()` and `matches_item()` for single criterion
4. **Scheduler integration** (`scheduler.rs`): `RunParams.filter: Option<FilterCriterion>` → will become `Vec<FilterCriterion>`

**Relevant files/modules:**

- `src/filter.rs` (172 lines) — `FilterField` enum (7 variants), `FilterValue` enum, `FilterCriterion` struct with `Display` impl, `parse_filter()`, `apply_filter()`, `matches_item()`
- `src/main.rs` (1173 lines) — CLI args struct (lines 56-69), `handle_run()` (lines 328-756) containing filter parsing, display, RunParams construction, halt-reason display
- `src/scheduler.rs` (1840 lines) — `RunParams` struct (lines 50-60), filter application block (lines 678-748), action selection (lines 750-763)
- `tests/filter_test.rs` — Existing single-criterion filter tests
- `tests/scheduler_test.rs` — Scheduler tests with `run_params()` helper using `filter: None`

**Existing patterns in use:**

- `--target` uses `Vec<String>` with `action = clap::ArgAction::Append` (lines 57-58 of main.rs) — exact template for `--only`
- Fail-fast validation — single invalid filter aborts with `Err`
- `matches_item()` takes one criterion, returns bool — composable for multi-criteria AND
- `FilterCriterion` implements `Display` returning `field=value` — joinable with ` AND `

### Reusable Components

- **`parse_filter(raw: &str)` → `Result<FilterCriterion, String>`** — Call once per `--only` value
- **`matches_item(criterion, item)` → `bool`** — Call per criterion per item, AND results
- **`Display` impl for `FilterCriterion`** — Format each, join with ` AND `
- **`conflicts_with` annotation** — Works unchanged with `Vec<String>`
- **`HaltReason` variants** (`FilterExhausted`, `NoMatchingItems`) — Semantics unchanged

### Constraints from Existing Code

- `matches_item()` signature must not change (PRD constraint) — multi-filter composes by repeated calls
- `Tag` field uses `Vec<String>` on items with `contains()` matching — enables multi-tag AND
- Scalar fields (status, impact, size, risk, complexity, pipeline_type) are single-valued — duplicates are contradictions
- `None`-valued optional fields don't match — existing `matches_item()` behavior
- Filter re-evaluated each scheduler loop iteration — no caching

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| `conflicts_with = "target"` works with `Vec<String>` Append | Confirmed — clap fires conflict check when conflicting arg present in any quantity | No concern; runtime guard still useful as safety net |
| `apply_filters` composes via `matches_item()` with AND reduction | Confirmed — `.all()` on iterator of `matches_item()` calls is idiomatic Rust | Clean implementation path |
| Empty `Vec` = no filter (replaces `None`) | Confirmed — `Vec<T>` defaults to empty when flag absent | Semantically equivalent; all `is_some()`/`is_none()` checks become `!is_empty()`/`is_empty()` |

No concerns found. PRD assumptions are well-aligned with codebase reality.

---

## Critical Areas

### Halt Condition Logic Update

**Why it's critical:** The scheduler's halt condition logic (lines 678-748 of scheduler.rs) currently pattern-matches on `Option<FilterCriterion>`. The `NoMatchingItems` check re-scans all items with `matches_item()` on a single criterion. With multiple criteria, this must AND all criteria.

**Why it's easy to miss:** The halt logic has two distinct paths (NoMatchingItems vs FilterExhausted) with different cross-checks. Both need updating, and the `any_match_in_snapshot` logic must check ALL criteria, not just one.

**What to watch for:** Ensure the `snapshot.items.iter().any(...)` check in the NoMatchingItems branch applies ALL criteria (`.all()` inside `.any()`). The FilterExhausted branch uses `filtered.items.is_empty()` which automatically works correctly when `apply_filters` returns the AND-filtered result.

### Duplicate Field Validation

**Why it's critical:** Must correctly distinguish scalar fields (reject duplicates) from the `tag` field (allow duplicates for AND composition).

**Why it's easy to miss:** The exemption is for one specific field variant (`FilterField::Tag`). Easy to forget or implement the check incorrectly.

**What to watch for:** Collect fields into a `HashSet`, but skip the insert-and-check for `FilterField::Tag`. Error message should suggest WRK-056's OR syntax per PRD "Should Have".

---

## Deep Dives

No deep dives needed. The implementation path is unambiguous.

---

## Synthesis

### Open Questions

None. The PRD is well-specified and the codebase research confirms all assumptions.

### Recommended Approaches

#### CLI Argument Change

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `Vec<String>` + `Append` (recommended) | Matches `--target` pattern exactly, preserves order, clean empty-vec semantics | None for this use case | Always — this is the only correct approach |

**Initial recommendation:** Follow the `--target` pattern exactly. Change `only: Option<String>` to `only: Vec<String>` with `action = clap::ArgAction::Append`.

#### Multi-Criteria Composition

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| New `apply_filters()` calling `matches_item()` per criterion (recommended) | Preserves existing `matches_item()`, clean AND via `.all()` | Minor code duplication with `apply_filter()` | Always — PRD constraint requires this |

**Initial recommendation:** Add `apply_filters(criteria: &[FilterCriterion], backlog: &BacklogFile) -> BacklogFile` that filters items where ALL criteria match. The existing `apply_filter()` can remain for backward compatibility or be removed if unused after migration.

#### RunParams.filter Type

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `Vec<FilterCriterion>` (recommended) | Cleaner than `Option<Vec>`, empty = no filter | Must change all match sites from `Option` to `is_empty()` | Always — PRD assumption #3 |

**Initial recommendation:** `Vec<FilterCriterion>` with empty vec = no filter.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [clap derive tutorial](https://docs.rs/clap/latest/clap/_derive/_tutorial/index.html) | Docs | Canonical `Vec<String>` multi-value example |
| [clap Arg docs](https://docs.rs/clap/latest/clap/struct.Arg.html) | Docs | `ArgAction::Append` and `conflicts_with` reference |
| [clap Discussion #4331](https://github.com/clap-rs/clap/discussions/4331) | Discussion | Append vs num_args vs value_delimiter |
| [clap Issue #1772](https://github.com/clap-rs/clap/issues/1772) | Issue | Vec<T> default behavior (empty vec when absent) |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Light external research: clap Append pattern | Confirmed approach matches existing `--target` pattern; identified pitfalls with implicit Vec behavior |
| 2026-02-20 | Light internal research: filter architecture | Mapped all integration points across 3 files + tests; confirmed PRD assumptions align with codebase |
| 2026-02-20 | PRD analysis | No concerns; all PRD assumptions validated by research |

## Assumptions

Decisions made without human input during autonomous tech research:

1. **Light mode selected** — The PRD is thorough, the codebase pattern (`--target`) is an exact template, and no new dependencies are involved. Deep research would yield no additional insights.
2. **No deep dives needed** — All research questions were answered by the initial parallel research phase. The implementation path is unambiguous.
3. **Existing `apply_filter()` disposition** — The research recommends adding `apply_filters()` alongside the existing `apply_filter()`. Whether to remove the single-criterion version is a design/implementation decision, not a research question.
