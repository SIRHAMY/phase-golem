# WRK-055: Support multiple --only filters with AND logic

## Problem Statement

The `--only` CLI flag currently accepts a single filter criterion (e.g., `--only status=ready`). Users who want to narrow results further (e.g., ready items that are also small) cannot do so without manual inspection. Supporting multiple `--only` flags combined with AND logic allows precise item selection.

## Proposed Approach

### 1. Change CLI argument to accept multiple values

In `main.rs`, change:
```rust
only: Option<String>
```
to:
```rust
#[arg(long, conflicts_with = "target", action = clap::ArgAction::Append)]
only: Vec<String>,
```

Update the argument validation in the run handler to parse each element via `parse_filter()` and collect into a `Vec<FilterCriterion>`.

### 2. Add compound filter application in filter.rs

Add a new function alongside existing `apply_filter`:
```rust
pub fn apply_filters(criteria: &[FilterCriterion], backlog: &BacklogFile) -> BacklogFile
```

This filters items that match ALL criteria (AND logic). Each item must pass every criterion to be included.

### 3. Update scheduler integration

In `scheduler.rs`, change `RunParams::filter` from `Option<FilterCriterion>` to `Vec<FilterCriterion>` (empty vec = no filter). Update the filter application in the scheduler loop to call `apply_filters`.

### 4. Update logging

Adapt the filter display logging in `main.rs` to show all active filters.

### Files affected

1. **`src/main.rs`** (~10 lines) - CLI arg type change, parse loop, logging
2. **`src/filter.rs`** (~15 lines) - New `apply_filters` function
3. **`src/scheduler.rs`** (~10 lines) - `RunParams` type change, filter application
4. **`tests/filter_test.rs`** (~30-50 lines) - Tests for AND combination logic

## Relationship to WRK-056

WRK-056 (comma-separated OR values) is complementary. Together they enable: `--only impact=high,medium --only status=ready` meaning "(impact high OR medium) AND (status ready)". These can be implemented independently; WRK-055 (AND across flags) should come first as it establishes the multi-filter infrastructure.

## Assessment

| Dimension  | Rating | Rationale |
|------------|--------|-----------|
| Size       | Small  | 3-4 files, ~65-85 lines of changes including tests |
| Complexity | Low    | Straightforward extension of single-criterion to multi-criteria. No new architecture. |
| Risk       | Low    | Backward compatible — single `--only` works identically. No shared interfaces changed. |
| Impact     | Medium | Meaningful power-user improvement for filtering item selection |

## Assumptions

- AND logic is the correct default for multiple `--only` flags (consistent with how most CLI tools treat repeated flags)
- Empty `Vec<FilterCriterion>` (no `--only` flags) means "no filtering" — same as current `None` behavior
- `RunParams::filter` type change from `Option<FilterCriterion>` to `Vec<FilterCriterion>` is acceptable since it's internal
