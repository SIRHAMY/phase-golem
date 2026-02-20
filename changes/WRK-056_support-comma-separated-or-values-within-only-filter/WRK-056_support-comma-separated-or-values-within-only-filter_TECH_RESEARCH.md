# Tech Research: Support Comma-Separated OR Values Within --only Filter

**ID:** WRK-056
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-056_support-comma-separated-or-values-within-only-filter_PRD.md
**Mode:** Light

## Overview

Research for extending the `--only` filter to support comma-separated OR values within a field (e.g., `--only impact=high,medium`). This is a direct follow-up to WRK-055 which established AND logic for multiple `--only` flags. The syntax was already specified in WRK-055's error message, so the primary research focus is confirming the approach aligns with established CLI patterns and identifying the right data model change.

## Research Questions

- [x] Is comma-separated OR within a filter key a standard CLI pattern?
- [x] What data model change is needed in `FilterCriterion` to support multi-value OR?
- [x] What are the integration points that need updating?

---

## External Research

### Landscape Overview

The "comma = OR within key, repeated flag = AND across keys" pattern is a well-established convention across major CLI tools. AWS CLI, Kubernetes label selectors, and GitHub CLI all use variations. The design is intuitive because it mirrors how humans express alternatives ("high or medium") versus conjunctions ("high impact AND small size").

### Common Patterns & Approaches

#### Pattern: Comma-Delimited Values with Implicit OR (AWS CLI Style)

**How it works:** A single filter key accepts multiple comma-separated values. Values within the same key are OR'd; separate filter invocations are AND'd.

```bash
aws ec2 describe-instances \
  --filters "Name=instance-state-name,Values=running,stopped" \
             "Name=vpc-id,Values=vpc-123"
```

**When to use:** Key=value structured filters needing both OR (within key) and AND (across keys). Closest analogue to `--only impact=high,medium`.

**Tradeoffs:**
- Pro: Intuitive, widely understood, compact syntax
- Con: Commas cannot appear in values, OR vs AND distinction must be documented

**References:**
- [AWS CLI Filtering Output](https://docs.aws.amazon.com/cli/latest/userguide/cli-usage-filter.html) — canonical example
- [AWS EC2 Filtering](https://docs.aws.amazon.com/AWSEC2/latest/UserGuide/Using_Filtering.html)

#### Pattern: Set-Based Selectors (Kubernetes Label Selectors)

**How it works:** Explicit `in` operator with parenthesized value lists for OR logic.

```bash
kubectl get pods -l 'environment in (production, qa), partition=frontend'
```

**When to use:** Complex queries needing rich operators (equality, inequality, set membership). Overkill for simple key=value filtering.

**Tradeoffs:**
- Pro: Unambiguous, supports negation
- Con: Verbose, requires shell quoting for parentheses

**References:**
- [Kubernetes Labels and Selectors](https://kubernetes.io/docs/concepts/overview/working-with-objects/labels/)

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Spaces after commas in shell args | `--only impact=high, medium` becomes two separate args | Trim whitespace after comma split (Should Have in PRD) |
| No escape mechanism for commas in values | Tag/pipeline_type values with commas cannot be filtered | Accept as trade-off (PRD Constraint); values are identifier-style |
| AND/OR confusion in documentation | Users may not understand when OR vs AND applies | Clear `--help` text with examples of both |
| Brace expansion `{a,b}` in shell | `--only impact={high,medium}` expands to two args | Not our format; `impact=high,medium` is safe |

### Key Learnings

- The comma=OR pattern is the de facto standard for key=value CLI filters
- Do NOT use clap's `value_delimiter` — need to split on `=` first, then `,` on the value portion
- Whitespace trimming after comma split is a common UX expectation
- No escape mechanism needed for enum-like values

---

## Internal Research

### Existing Codebase State

WRK-055 established the foundation: `Vec<FilterCriterion>` with AND logic across criteria. The current filter module has a clean parse-validate-apply pipeline.

**Relevant files/modules:**
- `src/filter.rs` — Core filter types and logic (`parse_filter`, `matches_item`, `validate_filter_criteria`, `apply_filters`, `format_filter_criteria`)
- `src/types.rs` — Value parsing functions (`parse_item_status`, `parse_dimension_level`, `parse_size_level`)
- `src/main.rs:418-465` — Filter parsing loop, display formatting, startup messages
- `src/scheduler.rs:678-749` — Filter application in scheduler loop, halt detection with inlined `matches_item` call
- `tests/filter_test.rs` — Comprehensive unit tests for parsing, matching, validation, display
- `tests/scheduler_test.rs` — Integration tests for scheduler with filters
- `tests/common/mod.rs` — Test fixture helpers (`make_item`, `make_backlog`)

**Existing patterns in use:**
- Parse-validate-apply pipeline: `parse_filter()` → `validate_filter_criteria()` → `apply_filters()`
- Case-insensitive parsing for enum fields, case-sensitive for free-text (tag, pipeline_type)
- `FilterCriterion` with `Display` impl formatting as `field=value`
- Tag field exempted from duplicate-field detection in validation (allows AND semantics across flags)

### Reusable Components

- `parse_item_status()`, `parse_dimension_level()`, `parse_size_level()` — Call per-value after comma split
- `matches_item(criterion, item)` — Reuse for per-value matching within multi-value criteria
- HashSet-based validation in `validate_filter_criteria()` — Extend for within-list duplicate detection
- Test fixture builders in `tests/common/mod.rs`

### Constraints from Existing Code

- `matches_item()` should remain the reusable building block (PRD constraint)
- Backward compatibility: single-value `--only impact=high` must produce identical output
- Tag field has special multi-flag semantics (AND across flags) that must compose with multi-value (OR within flag)
- Scheduler inlines `params.filter.iter().all(|c| filter::matches_item(c, item))` — must work with new type

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Comma as delimiter | Aligns with de facto CLI standard (AWS, kubectl) | No concern — well-established pattern |
| Manual parsing (no clap `value_delimiter`) | Confirmed correct — need `=` split before `,` split | Implementation should split on `=` first, then `,` on value portion |
| Whitespace trimming as Should Have | External research confirms this is a common UX expectation | Low risk to defer, but easy to implement alongside comma splitting |

No significant concerns. The PRD is well-aligned with established patterns.

---

## Critical Areas

### Data Model for Multi-Value Criteria

**Why it's critical:** The choice between modifying `FilterCriterion.value` to hold multiple values vs. introducing a wrapper type affects every function in the filter module and all call sites.

**Why it's easy to miss:** Both approaches work, but they have different implications for `matches_item()` reuse and test migration.

**What to watch for:** The design phase should evaluate:
- **Option A:** Change `FilterCriterion.value` to `Vec<FilterValue>` — simpler type, but `matches_item` needs to iterate internally
- **Option B:** Keep `FilterCriterion.value` as single value, add `values: Vec<FilterValue>` field — backward-compatible shape, but redundant for single values
- **Option C:** Introduce a new top-level type (e.g., `FilterCriterionSet`) wrapping `FilterField` + `Vec<FilterValue>` — cleanest separation but requires updating all call sites

### Duplicate Detection Within Comma List

**Why it's critical:** Must compare parsed values (not raw strings) per PRD requirement. For enum fields, case-insensitive aliases resolve to the same variant.

**Why it's easy to miss:** Simple string comparison after split would miss `HIGH,high` as duplicates for enum fields.

**What to watch for:** Duplicate detection must happen after parsing each value through `parse_dimension_level` etc., comparing the parsed enum variants.

---

## Deep Dives

_No deep dives needed — light mode research with well-understood problem space._

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| Which data model approach for multi-value criteria? | Affects all filter functions and call sites | Option A (Vec<FilterValue>), Option B (additional field), Option C (new wrapper type) |

### Recommended Approaches

#### Data Model for Multi-Value Criteria

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Modify `FilterCriterion.value` to `Vec<FilterValue>` | Simple, one type to understand | Changes `matches_item` signature semantics; single-value case wraps in vec | Minimal surface area desired |
| New wrapper type `FilterCriterionSet(field, Vec<FilterValue>)` | Clean separation, explicit multi-value support | More types, all call sites update | Clarity and explicitness preferred |

**Initial recommendation:** Modify `FilterCriterion.value` to `Vec<FilterValue>` (Option A). The change is small, keeps the type count low, and single-value criteria simply have a one-element vec. `matches_item` becomes `values.iter().any(|v| matches_single_value(v, item))` internally. This aligns with the PRD's constraint to reuse `matches_item` as the building block.

#### Parsing Strategy

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Extend `parse_filter()` to return multi-value criterion | Single entry point, backward compatible API | Slightly more complex parse function | Keeping API surface minimal |
| New `parse_filter_multi()` alongside existing | No changes to existing function | Two parsing functions to maintain | Strict backward compatibility needed |

**Initial recommendation:** Extend `parse_filter()` in place. The function already handles the `KEY=VALUE` split; adding a `.split(',')` on the value portion and parsing each token is a natural extension. Single-value input produces a one-element vec, preserving backward compatibility.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [AWS CLI Filtering](https://docs.aws.amazon.com/cli/latest/userguide/cli-usage-filter.html) | Docs | Canonical comma=OR pattern reference |
| [K8s Labels and Selectors](https://kubernetes.io/docs/concepts/overview/working-with-objects/labels/) | Docs | Set-based OR alternative (for comparison) |
| [Clap `value_delimiter` docs](https://docs.rs/clap/latest/clap/struct.Arg.html) | Docs | Why NOT to use clap's built-in for this case |
| WRK-055 SPEC | Internal | Predecessor design patterns to follow |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Light external research: CLI comma-separated patterns | Confirmed comma=OR is de facto standard; no concerns |
| 2026-02-20 | Light internal research: codebase exploration | Mapped all integration points; identified data model as key design decision |
| 2026-02-20 | PRD analysis | No conflicts between PRD and research findings |
