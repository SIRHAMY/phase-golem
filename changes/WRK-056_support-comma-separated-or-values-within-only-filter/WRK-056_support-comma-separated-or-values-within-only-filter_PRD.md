# Change: Support Comma-Separated OR Values Within --only Filter

**Status:** Proposed
**Created:** 2026-02-20
**Author:** AI (autonomous PRD creation)

## Problem Statement

The `--only` filter on `phase-golem run` supports combining multiple criteria with AND logic via repeated flags (e.g., `--only impact=high --only size=small`). However, there is no way to express OR logic within a single field. A user who wants to process all items with impact=high OR impact=medium must run the orchestrator twice with different filters, or omit the impact filter entirely and accept a broader result set.

This gap is already acknowledged in the codebase: when a user tries `--only impact=high --only impact=low`, the error message explicitly suggests the planned syntax: "For OR logic within a field, use comma-separated values: --only impact=high,low". WRK-056 implements that promised syntax.

## User Stories / Personas

- **Solo developer** — Has a backlog where items span multiple severity levels and wants to process a subset: "run all high or medium impact items that are small" (`--only impact=high,medium --only size=small`). Currently impossible without two separate runs or using `--target` with manually curated IDs.

## Desired Outcome

Users can specify comma-separated values within a single `--only` flag to express OR logic for that field:

```
phase-golem run --only impact=high,medium --only size=small
```

This processes items that match `(impact=high OR impact=medium) AND size=small`. The comma-separated OR composes with the existing cross-field AND logic from WRK-055.

Terminal output displays the filter using commas within each criterion and `AND` between criteria:

```
[config] Filter: impact=high,medium AND size=small — 5 items match (from 47 total)
```

A single value (no comma) continues to work identically to today.

## Success Criteria

### Must Have

- [ ] `--only field=value1,value2` syntax is supported for all filter fields (status, impact, size, risk, complexity, tag, pipeline_type)
- [ ] Comma-separated values within a field use OR logic: an item matches the criterion if it matches ANY of the listed values
- [ ] Cross-field criteria continue to use AND logic: `--only impact=high,medium --only size=small` means `(impact=high OR impact=medium) AND size=small`
- [ ] Each individual value within a comma-separated list is validated using the same rules as today (valid value type, within allowed range). Validation is fail-fast: the first invalid value encountered aborts with an error before any items are processed. Each token inherits the same case-handling rules as single-value parsing (e.g., enum fields are parsed case-insensitively; tag and pipeline_type values are matched case-sensitively).
- [ ] Invalid values within a comma list produce error messages that identify the invalid value and the field: e.g., `Invalid value 'huge' for field 'size'. Valid values: small, medium, large`
- [ ] Empty values in comma lists are rejected: `--only impact=high,,low`, `--only impact=,high`, `--only impact=high,`, and `--only impact=,` all produce errors identifying the empty token
- [ ] Duplicate values within a single comma-separated list are rejected by comparing parsed values (not raw strings): `--only impact=high,high` errors because `high` appears twice. If a field has aliases that parse to the same value, those are also rejected as duplicates.
- [ ] Duplicate single-valued field detection (from WRK-055) still applies across `--only` flags for fields that accept only one value per item (status, impact, size, risk, complexity, pipeline_type): `--only impact=high,medium --only impact=low` is rejected because `impact` appears in two separate `--only` arguments
- [ ] The error message for duplicate single-valued fields across `--only` flags is updated to: `Field 'impact' specified multiple times in separate --only flags. Combine values in a single flag: --only impact=value1,value2`
- [ ] The `tag` field retains its multi-flag AND semantics from WRK-055: `--only tag=a --only tag=b` remains valid and means items must have both tags. Multi-value and single-value tag flags can be combined: `--only tag=a,b --only tag=c` means `(tag=a OR tag=b) AND tag=c`
- [ ] Items with `None` for a filtered optional field (e.g., no `impact` set) do not match any value in a multi-value criterion and are excluded, consistent with single-value behavior
- [ ] Single-value usage is fully backward compatible: `--only impact=high` (no comma) produces identical filter output, terminal display, and halt behavior as before this change
- [ ] Terminal display shows comma-separated values within criteria: `format_filter_criteria` for `--only impact=high,medium --only size=small` returns exactly `impact=high,medium AND size=small`. Single-value criteria display unchanged.
- [ ] Filter display in halt messages uses the same comma-separated format. `FilterExhausted` (all matching items done/blocked) and `NoMatchingItems` (zero matches at startup) both display the criteria in comma-separated format.
- [ ] The `--only` flag's `--help` text is updated to document both comma-separated OR syntax within a field and repeated-flag AND syntax across fields, with examples of each. The help text also documents that `--only tag=a,b` (OR) differs from `--only tag=a --only tag=b` (AND).

### Should Have

- [ ] Whitespace around commas is trimmed after splitting: `--only "impact=high, medium"` (quoted to survive shell parsing) works the same as `--only impact=high,medium`. Whitespace-only tokens after trimming are treated as empty values and rejected.

### Nice to Have

- [ ] None identified

## Scope

### In Scope

- Extending `parse_filter` (or introducing a new parsing function) to split the value portion on commas and produce a representation that holds multiple values for one field
- Updating or introducing types as needed to represent a single-field criterion with one or more values (OR semantics). The specific data model choice (modify `FilterCriterion` vs. introduce a wrapper type) is a design-phase decision.
- Updating `matches_item` (or its call sites) to check if an item matches ANY value in a multi-value criterion
- Updating `validate_filter_criteria` to handle multi-value criteria: duplicate field detection across `--only` flags for single-valued fields, duplicate value detection within a comma-separated list
- Updating the duplicate-field error message in `validate_filter_criteria` to reflect that comma-separated OR is now supported
- Updating `apply_filters` to work with the new multi-value criteria
- Updating `format_filter_criteria` and the `Display` impl to show comma-separated values within criteria
- Updating terminal display in `handle_run()` for the new format
- Updating halt-reason display logic — note that halt messages reuse the pre-formatted `filter_display` string from `handle_run()`, so updating `format_filter_criteria` propagates automatically
- Updating the inlined filter matching logic in the scheduler loop (the `params.filter.iter().all(|c| filter::matches_item(c, item))` expression used for `any_match_in_snapshot` checks) to support multi-value criteria
- Updating the `--only` flag's clap help string
- New unit tests for: comma-separated parsing, validation (empty values, trailing commas, duplicate values, cross-flag duplicate fields, tag multi-flag + multi-value), multi-value matching (including None-field exclusion), and display formatting
- Updating existing tests in `tests/filter_test.rs` and `tests/scheduler_test.rs` that construct or assert on `FilterCriterion` if the type signature changes

### Out of Scope

- Negation filters (`--only-not`, `--only impact!=low`)
- Complex boolean expressions or nested logic beyond the field-level OR + cross-field AND
- Glob/wildcard matching for tag or pipeline_type values
- Range filters (e.g., `--only impact>=medium`)
- Combining `--target` and `--only`
- Persisting filter configurations
- The `format_filter_criteria` output is display-only; round-trip re-parsing of the display string is not guaranteed or required

## Non-Functional Requirements

- **Performance:** For a criterion with m comma-separated values, matching is O(m) per item per criterion (short-circuit on first match). Given m is bounded by the number of valid values per field (at most 6 for status, 3 for dimensions/size), this remains sub-millisecond for realistic backlogs. For free-text fields (tag, pipeline_type), there is no enforced maximum on the number of comma-separated values beyond what duplicate detection catches.

## Constraints

- Must preserve backward compatibility with single-value `--only` syntax
- Comma is the delimiter for OR values within a field. Tag and pipeline_type values containing commas cannot be expressed with this syntax. This is an accepted limitation; these values are expected to be identifier-style strings without commas.
- The `tag` field has different semantics depending on syntax: `--only tag=a,b` (OR — has either tag) vs `--only tag=a --only tag=b` (AND — has both tags). This difference must be documented in `--help` text because the syntax distinction is subtle.
- The existing `matches_item` function should be reused where possible to minimize the scope of code changes. It is called from both the scheduler loop and test code; reusing it rather than introducing a replacement reduces the number of call sites requiring updates.

## Dependencies

- **Depends On:** WRK-055 (already shipped — multi-filter AND logic with `Vec<FilterCriterion>`)
- **Blocks:** None identified

## Risks

- [ ] **Tag OR vs AND confusion:** The difference between `--only tag=a,b` (OR) and `--only tag=a --only tag=b` (AND) could confuse users. Mitigated by `--help` text and consistent behavior with how single-valued fields work (comma = OR within field, separate flags = AND across fields).
- [ ] **Comma in values:** If a tag or pipeline_type value legitimately contains a comma, it cannot be filtered with the `--only` syntax. This is an accepted trade-off for the simplicity of the comma delimiter.

## Open Questions

None — the syntax was already specified in WRK-055's error message (`--only impact=high,low`) and the semantics (OR within field, AND across fields) follow from the existing design.

## Assumptions

Decisions made without human input during autonomous PRD creation:

1. **Minimal exploration** — This is a direct follow-up to WRK-055 with clear requirements already specified in the codebase's own error message. The syntax and semantics are predetermined.
2. **Comma as delimiter** — The comma character was already specified in the WRK-055 error message. This precludes commas in tag/pipeline_type values, which is an acceptable trade-off for identifier-style values.
3. **Whitespace trimming as Should Have** — Per-token trimming after comma splitting is a usability nicety. Note that shell parsing means `--only impact=high, medium` (unquoted space) arrives as two separate tokens, so trimming only helps in quoted strings like `--only "impact=high, medium"`. This makes it less critical in practice.
4. **Duplicate value rejection compares parsed values** — `--only impact=high,high` is rejected by comparing parsed enum variants, not raw strings. This means aliases that resolve to the same value are also caught. This matches the WRK-055 precedent of rejecting duplicate tag criteria by structural equality.
5. **Tag multi-flag + multi-value composition** — `--only tag=a,b --only tag=c` is valid and means `(tag=a OR tag=b) AND tag=c`. This follows naturally from the existing semantics: each `--only tag=...` argument is a separate AND criterion, and within each one, comma-separated values are OR. Duplicate tag criteria across flags are detected by comparing the full parsed value set; `--only tag=a --only tag=a` remains rejected as identical.
6. **Error message update for duplicate fields** — The updated error message reads: `Field 'impact' specified multiple times in separate --only flags. Combine values in a single flag: --only impact=value1,value2`. This replaces the forward-looking hint from WRK-055 with actionable guidance now that the feature exists.
7. **Data model is a design-phase decision** — The PRD specifies what behavior is needed, not whether `FilterCriterion` is modified or a new wrapper type is introduced. That choice belongs in the design phase.

## References

- WRK-055 PRD/SPEC/Design: `changes/WRK-055_support-multiple-only-filters-with-and-logic/` — predecessor that introduced AND logic and the comma-separated OR error hint
- Filter module: `src/filter.rs` — `parse_filter()`, `matches_item()`, `validate_filter_criteria()`, `apply_filters()`, `format_filter_criteria()`
- CLI definition: `src/main.rs` — `only: Vec<String>` arg, `handle_run()` filter parsing
- Scheduler: `src/scheduler.rs` — `RunParams.filter`, filter application and inlined `matches_item` call in scheduler loop
