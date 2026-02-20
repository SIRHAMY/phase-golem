# Change: Support Multiple --only Filters with AND Logic

**Status:** Proposed
**Created:** 2026-02-20
**Author:** AI (autonomous PRD creation)

## Problem Statement

The `--only` flag on `phase-golem run` currently accepts a single `KEY=VALUE` filter criterion (e.g., `--only impact=high`). Users who want to narrow their run to a specific intersection of attributes — such as high-impact, small items in ready status — cannot do so. They must either pick the single most important filter and accept a broader result set, or resort to `--target` with manually curated ID lists.

This was identified as a "Should Have" in the original WRK-030 PRD (which introduced `--only`). The single-filter initial implementation has shipped; this change completes the planned multi-filter extension.

## User Stories / Personas

- **Solo developer** - Has a large backlog and wants to run a focused session on a specific slice: "process all small, ready, low-risk items" or "work on high-impact features only." Needs to combine 2-3 filter criteria to get a precise subset without manually listing IDs.

## Desired Outcome

Users can specify `--only` multiple times on the CLI, and the criteria are combined with AND logic:

```
phase-golem run --only impact=high --only size=small
```

This processes only items that match **all** specified criteria (high impact AND small size). The existing single `--only` usage continues to work identically.

A user watching the terminal sees the combined filter criteria displayed and the count of matching items. When all matching items reach a `status` of `done` or `blocked`, the orchestrator halts with `FilterExhausted`, the same behavior it exhibits today for a single filter.

## Success Criteria

### Must Have

- [ ] `--only` accepts multiple values via repeated flag (e.g., `--only impact=high --only status=ready`)
- [ ] Multiple criteria are combined with AND logic: an item must match ALL criteria to be included
- [ ] Items with `None` for a filtered optional field (e.g., no `impact` set) do not match that criterion and are excluded from results
- [ ] Each individual criterion is validated at startup using the same validation rules as today (valid field name, correct value type, value within allowed range). Validation is fail-fast: the first invalid criterion aborts startup with an error.
- [ ] Duplicate filter fields for scalar-valued fields (`status`, `impact`, `size`, `risk`, `complexity`, `pipeline_type`) are rejected at startup with error (e.g., `--only impact=high --only impact=low` errors because `impact` is specified twice). Identical field+value pairs (e.g., `--only impact=high --only impact=high`) are also rejected.
- [ ] The `tag` field is exempt from duplicate-field rejection because items carry multiple tags (`Vec<String>`). Multiple `--only tag=X` criteria compose as AND: `--only tag=backend --only tag=sprint-1` matches items that have both tags.
- [ ] Terminal output shows all active filter criteria in CLI order, joined by ` AND `: e.g., `[config] Filter: impact=high AND size=small — 3 items match (from 47 total)`. With a single criterion, the format is unchanged from today (no `AND` separator).
- [ ] Existing single `--only` usage is backward compatible: `Vec<FilterCriterion>` with length 1 produces identical filter output, terminal display, and halt behavior as before this change
- [ ] `--only` and `--target` remain mutually exclusive
- [ ] When the combined criteria produce zero matches on the initial snapshot with no prior progress, the orchestrator halts immediately with `NoMatchingItems` (an existing halt reason) and logs: `[filter] No items match combined filter criteria: impact=high AND size=small`
- [ ] When items did initially match the combined criteria but all have since reached `done` or `blocked` status, the orchestrator halts with `FilterExhausted` and the run summary displays: `Filter: all items matching impact=high AND size=small are done or blocked`
- [ ] When `--only` is not specified, `RunParams.filter` is an empty `Vec` and the scheduler runs without any filter, identical to current behavior

### Should Have

- [ ] Error message for duplicate scalar fields suggests the planned OR syntax from WRK-056 (the follow-up for comma-separated OR values within `--only`): "Field 'impact' specified multiple times. For OR logic within a field, use comma-separated values: --only impact=high,medium"

### Nice to Have

- [ ] None identified

## Scope

### In Scope

- Changing CLI argument `only: Option<String>` to `only: Vec<String>` using clap's `Append` action
- Verifying that `conflicts_with = "target"` clap annotation works correctly with the `Vec<String>` `Append` action
- Updating the runtime mutual-exclusion guard in `handle_run()` from `only.is_some()` to `!only.is_empty()`
- Changing `RunParams.filter` from `Option<FilterCriterion>` to `Vec<FilterCriterion>` (empty vec = no filter)
- Updating `parse_filter` call site to parse each `--only` value independently
- Adding duplicate field detection at startup (with `tag` exemption)
- Adding `apply_filters(criteria: &[FilterCriterion], backlog: &BacklogFile) -> BacklogFile` that composes via `matches_item()` per criterion with AND reduction
- Updating the scheduler filter application block in `run_scheduler()` — specifically: the `params.filter` type/binding, the `apply_filter` → `apply_filters` call, the `matches_item` cross-check in the `NoMatchingItems` branch, and the log messages referencing criterion display
- Updating the `filter_display` variable in `handle_run()` and the halt-reason display logic for `FilterExhausted` and `NoMatchingItems` to join multiple criteria with `AND`
- Updating terminal startup output to display multiple criteria
- Updating existing filter and scheduler tests to compile against the new `Vec<FilterCriterion>` signature
- New unit tests for multi-criteria parsing, validation (including tag exemption), filtering, and halt conditions

### Out of Scope

- OR logic within a single field (WRK-056 — comma-separated values)
- Complex boolean expressions or nested logic
- Combining `--target` and `--only`
- Negation filters (`--only-not`)
- Persisting filter configurations
- Detection of logically unsatisfiable filter combinations (e.g., `--only status=ready --only impact=high` on a backlog where all ready items have no `impact` set). Both semantically empty and legitimately empty filter results produce `NoMatchingItems`.

## Non-Functional Requirements

- **Performance:** Multi-criteria filter application adds negligible overhead. The existing single-criterion filter is sub-millisecond for 500 items. With k criteria, the cost scales linearly: O(n * k) where n = items and k = criteria. Given k is bounded by the number of distinct filter fields (7 maximum for scalar fields, plus any number of tag criteria), the <10ms budget is easily met.

## Constraints

- Must use clap's `action = Append` for multi-value `--only`, consistent with how `--target` already works
- Must preserve the existing `matches_item()` function signature, which takes a single `FilterCriterion`. Multi-filter logic composes by calling `matches_item()` once per criterion and reducing with AND.
- Filter operates on the backlog snapshot each scheduler loop iteration (re-evaluated per cycle), consistent with existing design
- This change should land before WRK-056 (comma-separated OR values) to avoid merge conflicts in shared filter code

## Dependencies

- **Depends On:** WRK-030 (already shipped — introduced `--only` with single filter)
- **Blocks:** WRK-056 (comma-separated OR values within `--only`)

## Risks

- [ ] **Field ordering ambiguity:** Two filters on different fields is unambiguous AND. Two filters on the same scalar field could be interpreted as OR or as an error. Mitigated by rejecting duplicate scalar fields with a clear error message. The `tag` field is explicitly exempted because items carry multiple tags.

## Open Questions

None — the design was already outlined in WRK-030's "Should Have" section and the implementation path is traced in the Scope section above.

## Assumptions

Decisions made without human input during autonomous PRD creation:

1. **Light mode (minimal exploration) selected** — This enhancement touches four modules (CLI, RunParams, filter, scheduler) with no new external dependencies. The requirements were already specified in WRK-030's "Should Have" section and the implementation path is traced in the Scope section.
2. **Duplicate field rejection for scalar fields, tag exempted** — When the same scalar field appears twice (e.g., `--only impact=high --only impact=low`), we reject with an error rather than silently treating it as OR. This cleanly separates the AND (WRK-055) and OR (WRK-056) features. The `tag` field is exempted because items carry a `Vec<String>` of tags, making multi-tag AND (`--only tag=a --only tag=b`) a valid and useful composition.
3. **Vec over Option for RunParams.filter** — Changing from `Option<FilterCriterion>` to `Vec<FilterCriterion>` is cleaner than `Option<Vec<FilterCriterion>>`. Empty vec means "no filter" (equivalent to today's `None`).
4. **Compose via existing matches_item** — Rather than creating a new multi-match function, multi-criteria filtering calls `matches_item()` per criterion and ANDs the results. This keeps the single-criterion matching logic untouched.
5. **Fail-fast validation** — Validation aborts on the first invalid criterion (matching current single-filter behavior) rather than collecting all errors. This keeps error handling simple and consistent.
6. **Criteria display in CLI order** — Multiple criteria are displayed in the order the user specified them on the command line. Order does not affect filter results (AND is commutative).

## References

- WRK-030 PRD: `changes/WRK-030_add-target-priority-list-and-only-flag-for-orchestrator-run/WRK-030_..._PRD.md` — original `--only` design with multi-filter as "Should Have"
- Filter module: `src/filter.rs` — `parse_filter()`, `apply_filter()`, `matches_item()`, `FilterCriterion` type
- CLI definition: `src/main.rs` — `only: Option<String>` arg, `handle_run()` filter parsing, `filter_display` variable, halt-reason display
- Scheduler: `src/scheduler.rs` — `RunParams.filter`, filter application block in `run_scheduler()`, halt condition checks
- WRK-056: Planned follow-up for comma-separated OR values within `--only`
