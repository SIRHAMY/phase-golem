# Design: Support Comma-Separated OR Values Within --only Filter

**ID:** WRK-056
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-056_support-comma-separated-or-values-within-only-filter_PRD.md
**Tech Research:** ./WRK-056_support-comma-separated-or-values-within-only-filter_TECH_RESEARCH.md
**Mode:** Light

## Overview

Extend `FilterCriterion` from holding a single `FilterValue` to a `Vec<FilterValue>`, enabling comma-separated OR logic within a single `--only` field (e.g., `--only impact=high,medium`). The parsing function `parse_filter()` splits the value portion on commas and parses each token through the existing per-field parsers. `matches_item()` becomes `values.iter().any(|v| matches_single_value(field, v, item))` internally. Validation adds within-list duplicate detection and updates the cross-flag duplicate-field error message. Display shows comma-separated values within criteria (e.g., `impact=high,medium AND size=small`). Single-value usage produces a one-element Vec, preserving full backward compatibility.

---

## System Design

### High-Level Architecture

The change follows the same vertical-slice pattern as WRK-055, touching the same four layers:

```
CLI (main.rs)  →  Parsing (filter.rs)  →  Validation (filter.rs)  →  Matching (filter.rs)
                                        ↘  Display (filter.rs via Display impl)
```

No new modules or public types are introduced. Two private helper functions are added (`matches_single_value()` for per-value matching, `parse_single_value()` for per-token field parsing), and `FilterValue` gets a `Display` impl. The changes are:
1. `FilterCriterion.value: FilterValue` → `FilterCriterion.values: Vec<FilterValue>` (field rename + type change)
2. `parse_filter()` extended to split on commas and parse each token via a new private `parse_single_value()` helper
3. `matches_item()` refactored to iterate over `values` with OR semantics via a new private `matches_single_value()` helper
4. `validate_filter_criteria()` updated error message for scalar field duplicates
5. `FilterValue` gets a `Display` impl; `FilterCriterion::Display` updated to show comma-separated values
6. Within-list duplicate detection added in `parse_filter()`
7. `--only` help text updated

### Component Breakdown

#### FilterCriterion Data Model (filter.rs)

**Purpose:** Represent a filter criterion that may match one or more values for a single field.

**Change:**
```rust
// Before (WRK-055)
pub struct FilterCriterion {
    pub field: FilterField,
    pub value: FilterValue,
}

// After (WRK-056)
pub struct FilterCriterion {
    pub field: FilterField,
    pub values: Vec<FilterValue>,
}
```

**Invariants:**
- `values` is always non-empty (enforced by `parse_filter()`)
- All values in `values` are of the same variant matching `field` (enforced by per-field parsing)
- No duplicate values within `values` (enforced by `parse_filter()`)

**Impact on derives:** `FilterCriterion` currently derives `Debug, Clone, PartialEq, Eq, Hash`. With `Vec<FilterValue>`, `Hash` is still derivable (chain: `String: Hash` → `FilterValue: Hash` (derived) → `Vec<FilterValue>: Hash` (std blanket impl) → `FilterCriterion: Hash` (derived)). `PartialEq` and `Eq` also work. All existing derives remain valid.

**Note on non-empty invariant:** The `values` field is `pub`, so code constructing `FilterCriterion` directly (e.g., tests) can create an empty `values` Vec. An empty `values` causes `matches_item()` to return `false` (`.any()` on empty iterator), which is a silent logic error rather than a panic. This is an accepted gap: all production construction goes through `parse_filter()` which enforces non-emptiness. Test code should use `parse_filter()` where possible, or assert the full `values` Vec when constructing directly.

#### Parsing (filter.rs — `parse_filter()`)

**Purpose:** Parse a raw `KEY=VALUE` string (now `KEY=VALUE1,VALUE2,...`) into a `FilterCriterion`.

**Changes to `parse_filter()`:**
1. Split on first `=` as before → `(field_str, value_str)`. The existing `value_str.is_empty()` check is retained before comma splitting as a short-circuit: `--only impact=` still produces the existing `"Filter must be in format KEY=VALUE"` error.
2. Identify the field from `field_str` (existing match dispatch, unchanged)
3. Split `value_str` on `,` → `Vec<&str>` of tokens
4. For each token: trim whitespace (Should Have), reject if empty, parse through a new private `parse_single_value(field: &FilterField, token: &str) -> Result<FilterValue, String>` helper extracted from the existing per-field match arms
5. Collect into `Vec<FilterValue>`
6. Check for within-list duplicates (comparing parsed `FilterValue`s via `HashSet`)
7. Return `FilterCriterion { field, values }`

**The `parse_single_value()` helper:** The existing `parse_filter()` has per-field parsing logic inlined in match arms (e.g., `parse_dimension_level(value_str)` for impact). These must be extracted into a standalone `parse_single_value(field: &FilterField, token: &str) -> Result<FilterValue, String>` private helper so the comma-splitting loop can call it per-token. This helper contains the exact same logic and error messages as the current match arms.

**Fail-fast semantics:** Tokens are processed left-to-right. The first empty token or invalid value aborts parsing immediately. Error message identifies the invalid value and field: `"Invalid value 'huge' for field 'size'. Valid values: small, medium, large"` — identical to single-value error messages. No positional context within the comma list is added; this is accepted because the token value uniquely identifies the problem without needing position.

**Empty token detection:** After splitting on `,` and trimming whitespace, any empty token produces an error: `"Empty value in comma-separated list for field 'impact'. Each value must be non-empty."` This catches `impact=high,,low`, `impact=,high`, `impact=high,`, and `impact=,`.

**Within-list duplicate detection:** After parsing all tokens into a `Vec<FilterValue>` (two-pass approach: first parse all tokens with fail-fast, then check for duplicates), iterate over the collected values inserting `&FilterValue` refs into a `HashSet<&FilterValue>`. If insertion returns false (value already seen), produce an error: `"Duplicate value 'high' in comma-separated list for field 'impact'"`. Comparison uses parsed `FilterValue`s (enum variants), so `HIGH,high` for enum fields are correctly detected as duplicates because both parse to the same `DimensionLevel::High`. For `tag` and `pipeline_type` fields, duplicate detection compares the stored `String` values case-sensitively (matching the existing case-sensitive matching behavior): `--only tag=backend,Backend` is accepted as two distinct values, while `--only tag=backend,backend` is rejected.

**Whitespace trimming (Should Have):** Each token is trimmed after comma splitting. This handles `--only "impact=high, medium"` (quoted to survive shell parsing). A whitespace-only token after trimming is treated as empty and rejected.

**Signature:** `parse_filter(raw: &str) -> Result<FilterCriterion, String>` — unchanged. The return type naturally accommodates multi-value via the new `values: Vec<FilterValue>` field.

#### Matching (filter.rs — `matches_item()`)

**Purpose:** Check if an item matches a (possibly multi-value) criterion.

**Changes:** Extract current single-value matching logic into a helper, then iterate with OR semantics:

```rust
fn matches_single_value(field: &FilterField, value: &FilterValue, item: &BacklogItem) -> bool {
    // Current matches_item body, unchanged
    match (field, value) {
        (FilterField::Status, FilterValue::Status(target)) => item.status == *target,
        (FilterField::Impact, FilterValue::Dimension(target)) => item.impact.as_ref() == Some(target),
        // ... etc
    }
}

pub fn matches_item(criterion: &FilterCriterion, item: &BacklogItem) -> bool {
    criterion.values.iter().any(|v| matches_single_value(&criterion.field, v, item))
}
```

**Backward compatibility:** A single-value criterion has `values: vec![single_value]`. `.any()` on a one-element iterator is equivalent to the current direct match. No change in behavior.

**None-field exclusion:** Items with `None` for an optional field fail `matches_single_value` for every value in the list, so `.any()` returns false. This is correct — `None` doesn't match any specific value.

**Performance:** For m comma-separated values, matching is O(m) per item per criterion with short-circuit on first match. Given m is bounded by the number of valid values per field (at most 6 for status, 3 for dimensions), this is sub-millisecond for realistic backlogs.

#### Validation (filter.rs — `validate_filter_criteria()`)

**Purpose:** Cross-criterion validation of the filter list.

**Changes:**
1. **Updated error message for scalar field duplicates:** The message changes from `"Field '{field}' specified multiple times. For OR logic within a field, use comma-separated values: --only {field}=value1,value2"` to `"Field '{field}' specified multiple times in separate --only flags. Combine values in a single flag: --only {field}=value1,value2"`. The hint is now actionable (the feature exists) rather than forward-looking.

2. **Scalar field duplicate detection unchanged in mechanism:** `validate_filter_criteria()` inserts `&criterion.field` (the `FilterField` enum variant) into `seen_scalar_fields: HashSet<&FilterField>`. This logic is unchanged from WRK-055. All non-Tag fields are scalar: `Status`, `Impact`, `Size`, `Risk`, `Complexity`, `PipelineType`. `PipelineType` is treated as scalar because `BacklogItem.pipeline_type` is `Option<String>` — each item has at most one pipeline type.

3. **Tag duplicate detection update:** For tag criteria, the existing identical-criterion detection (via `HashSet<&FilterCriterion>`) still works because `FilterCriterion` with `Vec<FilterValue>` still supports `Hash` + `Eq`. Two tag criteria are "identical" if they have the same field and the same values list **in the same order**. `Vec` equality is order-sensitive: `--only tag=a,b --only tag=a,b` is rejected, but `--only tag=a,b --only tag=b,a` is accepted as two distinct criteria (both applied with AND). This is an accepted limitation — the reordered case is logically redundant (AND of two identical ORs) but not incorrect, and adding canonicalization (sorting values before comparison) is not worth the complexity for this edge case.

**No new validation logic needed for within-list duplicates** — that's handled inside `parse_filter()` before the criterion reaches `validate_filter_criteria()`.

#### Display (filter.rs — `FilterValue::Display` + `FilterCriterion::Display`)

**Purpose:** Format values and criteria for terminal output.

**Changes:**

1. **New `impl Display for FilterValue`:** Extract the current value-to-string logic from `FilterCriterion::Display` into a standalone `Display` impl on `FilterValue`. This is the natural Rust idiom and enables clean iteration over multi-value criteria. The logic is identical to the current match block in `FilterCriterion::Display` (lines 51-64 of `filter.rs`):

```rust
impl Display for FilterValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterValue::Status(s) => write!(f, "{}", /* status lowercase */),
            FilterValue::Dimension(d) => write!(f, "{}", d),
            FilterValue::Size(s) => write!(f, "{}", s),
            FilterValue::Tag(t) => write!(f, "{}", t),
            FilterValue::PipelineType(p) => write!(f, "{}", p),
        }
    }
}
```

2. **Updated `FilterCriterion::Display`:** Uses `FilterValue::Display` to iterate and join:

```rust
impl Display for FilterCriterion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let values_str: String = self.values.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(",");
        write!(f, "{}={}", self.field, values_str)
    }
}
```

**Examples:**
- Single value: `impact=high` (unchanged)
- Multi-value: `impact=high,medium`
- Multi-criteria: `impact=high,medium AND size=small` (via `format_filter_criteria()`, unchanged)

#### CLI Help Text (main.rs)

**Purpose:** Document `--only` syntax including comma-separated OR.

**Change:** Update the `#[arg(long, ...)]` help string for `only`:

```
Filter items by attribute. Comma-separated values = OR within field; repeated flags = AND across fields.
Examples: --only impact=high,medium --only size=small (high or medium impact AND small size).
Tag: --only tag=a,b (has either) vs --only tag=a --only tag=b (has both).
```

### Data Flow

1. **CLI parsing** — clap collects repeated `--only` values into `Vec<String>` (unchanged from WRK-055)
2. **Parse each (fail-fast)** — Each string passes through `parse_filter()`, which now splits on commas and returns `FilterCriterion { field, values: Vec<FilterValue> }`. First error aborts via `?`.
3. **Cross-criterion validation** — `validate_filter_criteria()` checks for duplicate scalar fields across flags (with updated error message) and identical tag criteria.
4. **Display** — `format_filter_criteria()` joins criteria with ` AND `; each criterion's `Display` impl joins values with `,`. Result: `impact=high,medium AND size=small`.
5. **RunParams** — `Vec<FilterCriterion>` passed into scheduler (type unchanged; internal structure of `FilterCriterion` changed).
6. **Scheduler loop** — `apply_filters()` and inlined `matches_item()` calls work with multi-value criteria transparently via the new `matches_item()` → `matches_single_value()` delegation.

### Key Flows

#### Flow: Multi-Value OR Filter (Happy Path)

> User runs `phase-golem run --only impact=high,medium --only size=small` to process high or medium impact items that are small.

1. **CLI collects** — clap accumulates `["impact=high,medium", "size=small"]` into `only: Vec<String>`
2. **Parse first** — `parse_filter("impact=high,medium")`: split on `=` → `("impact", "high,medium")`, split `"high,medium"` on `,` → `["high", "medium"]`, parse each → `[Dimension(High), Dimension(Medium)]`, no duplicates → `FilterCriterion { field: Impact, values: [Dimension(High), Dimension(Medium)] }`
3. **Parse second** — `parse_filter("size=small")` → `FilterCriterion { field: Size, values: [Size(Small)] }`
4. **Validate** — `validate_filter_criteria()`: `Impact` into scalar HashSet (ok), `Size` into scalar HashSet (ok, different field) → passes
5. **Display** — `[config] Filter: impact=high,medium AND size=small — 5 items match (from 47 total)`
6. **Scheduler** — Each loop: `apply_filters()` filters items where `(impact=high OR impact=medium) AND size=small`. `matches_item()` for the impact criterion returns true if the item's impact is High or Medium.
7. **Halt** — When all matching items reach done/blocked → `FilterExhausted` with display: `Filter: all items matching impact=high,medium AND size=small are done or blocked`

**Edge cases:**
- Single value (no comma) → `values: vec![single_value]`, behavior identical to WRK-055
- Zero matches initially → `NoMatchingItems` halt

#### Flow: Tag Multi-Value OR + Multi-Flag AND Composition

> User runs `--only tag=a,b --only tag=c` to process items with (tag a OR tag b) AND tag c.

1. **CLI collects** — `["tag=a,b", "tag=c"]`
2. **Parse first** — `parse_filter("tag=a,b")` → `FilterCriterion { field: Tag, values: [Tag("a"), Tag("b")] }`
3. **Parse second** — `parse_filter("tag=c")` → `FilterCriterion { field: Tag, values: [Tag("c")] }`
4. **Validate** — Both are Tag fields. Tag is exempt from scalar duplicate detection. The two criteria are structurally different (different values lists) → no identical-pair rejection → passes.
5. **Matching** — For each item: first criterion matches if item has tag "a" OR tag "b". Second criterion matches if item has tag "c". Both must match (AND). Net effect: item must have (a or b) AND c.

**Edge cases:**
- `--only tag=a,b --only tag=a,b` → identical criteria (same order), rejected by `validate_filter_criteria()`
- `--only tag=a --only tag=a` → identical criteria, still rejected (unchanged from WRK-055)
- `--only tag=a,b --only tag=b,a` → different `Vec` ordering, accepted as two distinct AND criteria. Both criteria reduce to "has tag a or tag b", so the result is logically redundant but not incorrect. This is an accepted edge case (see Validation section).
- `--only tag=backend,Backend` → accepted as two distinct tag values because tag matching is case-sensitive. An item must have the literal tag "backend" or "Backend" to match.

#### Flow: Invalid Value in Comma List

> User runs `--only size=small,huge` — second value is invalid.

1. **Parse** — Split `"small,huge"` on `,` → `["small", "huge"]`. Parse `"small"` → Ok(`Size(Small)`). Parse `"huge"` → Err.
2. **Error** — `"Invalid value 'huge' for field 'size'. Valid values: small, medium, large"`
3. **Abort** — Error propagates via `?` from `handle_run()`. Same path as existing single-value errors.

#### Flow: Empty Token in Comma List

> User runs `--only impact=high,,low`.

1. **Parse** — Split `"high,,low"` on `,` → `["high", "", "low"]`. Trim each. Second token is empty after trimming.
2. **Error** — `"Empty value in comma-separated list for field 'impact'. Each value must be non-empty."`
3. **Abort** — Error propagates via `?`.

#### Flow: Duplicate Value Within Comma List

> User runs `--only impact=high,HIGH`.

1. **Parse** — Split `"high,HIGH"` on `,` → `["high", "HIGH"]`. Parse `"high"` → `Dimension(High)`. Parse `"HIGH"` → `Dimension(High)` (case-insensitive).
2. **Duplicate detection** — Insert `Dimension(High)` into HashSet (ok). Insert `Dimension(High)` again → duplicate detected.
3. **Error** — `"Duplicate value 'HIGH' in comma-separated list for field 'impact'"`
4. **Abort** — Error propagates via `?`.

#### Flow: Duplicate Scalar Field Across Flags (Updated Error)

> User runs `--only impact=high,medium --only impact=low`.

1. **Parse** — Both parse successfully.
2. **Validate** — `impact` inserted into scalar HashSet (ok). Second `impact` insert fails → duplicate.
3. **Error** — `"Field 'impact' specified multiple times in separate --only flags. Combine values in a single flag: --only impact=value1,value2"` (updated message)
4. **Abort** — Error propagates via `?`.

---

## Technical Decisions

### Key Decisions

#### Decision: Rename `value` to `values` (Vec<FilterValue>)

**Context:** Need to represent one or more OR values for a single field criterion.

**Decision:** Change `FilterCriterion.value: FilterValue` to `FilterCriterion.values: Vec<FilterValue>`. Rename the field from singular to plural.

**Rationale:** Tech research recommended this approach (Option A). Single-value criteria become one-element Vecs — semantically equivalent and handled uniformly by `.any()` iteration. No new types introduced. The rename from `value` to `values` makes the multi-value nature explicit at every call site, and the Rust compiler catches all missed sites during migration.

**Consequences:** Every site that accesses `criterion.value` must change to `criterion.values`. This includes `matches_item()`, `Display` impl, `validate_filter_criteria()` (tag identical-pair check), and all tests that construct `FilterCriterion` directly. The compiler enforces exhaustive migration.

#### Decision: Within-List Duplicate Detection in parse_filter()

**Context:** PRD requires that `--only impact=high,high` is rejected by comparing parsed values.

**Decision:** Perform within-list duplicate detection inside `parse_filter()` immediately after parsing all tokens, before returning the `FilterCriterion`.

**Rationale:** The duplicates are a property of a single `--only` argument's value list. Detecting them at parse time is the earliest possible point and keeps `validate_filter_criteria()` focused on cross-criterion concerns. It also means a `FilterCriterion` is guaranteed to have no internal duplicates by construction.

**Consequences:** `parse_filter()` takes on slightly more responsibility (parsing + within-list validation). This is acceptable because the validation is intrinsic to the parsed data, not a cross-criterion concern.

#### Decision: Extract matches_single_value() and parse_single_value() Helpers

**Context:** `matches_item()` needs to iterate over multiple values, and `parse_filter()` needs to call per-field parsing logic per-token. Both require extracting the current inline logic into callable helpers.

**Decision:** Extract two private helpers:
- `matches_single_value(field, value, item) -> bool` — current `matches_item()` body, unchanged
- `parse_single_value(field, token) -> Result<FilterValue, String>` — current per-field match arms from `parse_filter()`, unchanged

These are internal extractions to enable iteration, not new scope. The PRD constraint ("reuse `matches_item`") is satisfied because the public `matches_item()` function retains the same signature and is the entry point for all callers.

**Rationale:** The existing matching and parsing logic is correct and tested. Extracting it preserves that logic unchanged while enabling per-token and per-value iteration. Both helpers are private — the public API is unchanged.

**Consequences:** `matches_item()` signature remains `(criterion: &FilterCriterion, item: &BacklogItem) -> bool`. `parse_filter()` signature remains `(raw: &str) -> Result<FilterCriterion, String>`. All external callers continue unchanged. Only internal dispatch changes.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Field rename churn | Every `criterion.value` access migrates to `criterion.values` (14 test assertion sites confirmed) | Explicit plural naming prevents confusion about single vs multi | One-time migration; compiler catches all sites |
| parse_filter() handles both parsing and within-list validation | `parse_filter()` handles comma splitting, per-token parsing, empty detection, and duplicate detection — this is validation mixed with parsing | All within-list concerns handled at parse time; `FilterCriterion` is valid by construction (no internal duplicates, non-empty) | Deliberate consolidation: within-list validation is intrinsic to the parsed data. Separating it would require either exposing the raw token list or adding a second validation pass. `validate_filter_criteria()` remains focused on cross-criterion concerns. |
| Commas in tag/pipeline_type values unsupported | Tag or pipeline_type values containing commas cannot be filtered; `tag=a,b` which previously matched a tag named `"a,b"` now becomes `OR(tag=a, tag=b)` | Simple, standard delimiter syntax | Tag/pipeline_type values are identifier-style strings; commas in values are not expected. This is a breaking change only for users who have tags containing literal commas. |
| pipeline_type is scalar (no multi-flag AND) | Unlike `tag`, `pipeline_type` cannot use `--only pipeline_type=a --only pipeline_type=b` for AND semantics; duplicate flags are rejected | Consistent with single-valued fields | `BacklogItem.pipeline_type` is `Option<String>` — each item has at most one pipeline type, so AND over multiple values would always return zero results |

---

## Alternatives Considered

### Alternative: New FilterCriterionSet Wrapper Type

**Summary:** Introduce a new `FilterCriterionSet { field: FilterField, values: Vec<FilterValue> }` type alongside the existing `FilterCriterion`, keeping `FilterCriterion` as a single-value type for internal use.

**How it would work:**
- `parse_filter()` returns `FilterCriterionSet` (the multi-value type)
- `FilterCriterionSet` exposes `fn matches(&self, item: &BacklogItem) -> bool` using `matches_item()` internally
- `apply_filters()` and `validate_filter_criteria()` operate on `&[FilterCriterionSet]`

**Pros:**
- `FilterCriterion` remains unchanged — no migration of existing tests
- Clear separation between single-value and multi-value concepts

**Cons:**
- Introduces a new public type for a conceptually small change
- `FilterCriterion` would only be used internally by `FilterCriterionSet`, adding indirection
- All external call sites (`apply_filters`, `validate_filter_criteria`, `format_filter_criteria`, scheduler, main.rs) still need migration to the new type

**Why not chosen:** The indirection adds complexity without proportional benefit. Modifying `FilterCriterion` in place is simpler and the compiler catches all migration sites. The WRK-055 design set a precedent of modifying existing types (e.g., `Option<FilterCriterion>` → `Vec<FilterCriterion>`) rather than introducing wrappers.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Test migration misses assertions on `criterion.value` | Compile error (not runtime) | Certain for 14 `.value` assertion sites in filter_test.rs | Rust compiler catches field rename; mechanical migration. Migrate to full-vec assertion `assert_eq!(f.values, vec![...])` rather than `f.values[0]` to also verify list length. |
| Tag OR/AND confusion for users | Users may not understand `tag=a,b` (OR) vs `--only tag=a --only tag=b` (AND) | Medium | Documented in `--help` with explicit before/after examples for tag specifically. Only `tag` has this multi-flag AND duality; scalar fields reject duplicate flags with an error that guides users to comma syntax. |
| Comma in tag/pipeline_type values silently changes semantics | A tag named `"a,b"` was previously filterable via `tag=a,b`; after WRK-056 this becomes `OR(tag=a, tag=b)` | Low (tag values are identifier-style) | Accepted limitation per PRD. Verify no existing backlog data contains comma-containing tag values before shipping. |
| Unbounded value list for free-text fields | `tag` and `pipeline_type` accept unlimited comma-separated values | Low | Acceptable at current backlog scale; each value adds O(1) per-item matching cost. Could add a max-values-per-criterion limit if needed. |
| Tag duplicate detection is order-sensitive | `--only tag=a,b --only tag=b,a` passes validation despite being semantically redundant | Low | Accepted: produces redundant but correct filtering. Adding canonicalization (value sorting) is not worth the complexity for this edge case. |

---

## Integration Points

### Existing Code Touchpoints

Line references are accurate as of commit `db99c63` (WRK-055 build) and should be re-verified at implementation time.

- `src/filter.rs` line 19-26 — `FilterValue` enum: add `Display` impl
- `src/filter.rs` line 28-32 — `FilterCriterion` struct: rename `value` to `values`, change type to `Vec<FilterValue>`
- `src/filter.rs` line 49-67 — `FilterCriterion::Display` impl: simplify to use `FilterValue::Display`, iterate over `values`, join with `,`
- `src/filter.rs` line 69-141 — `parse_filter()`: extract `parse_single_value()` helper from per-field match arms, add comma splitting, per-token parsing, empty token detection, within-list duplicate detection, whitespace trimming
- `src/filter.rs` line 143-162 — `matches_item()`: extract `matches_single_value()` helper, iterate with `.any()`
- `src/filter.rs` line 164-185 — `validate_filter_criteria()`: update scalar duplicate error message text only
- `src/main.rs` line 60 — `--only` help text: update to document comma-separated OR syntax
- `src/scheduler.rs` line 690 — Inlined `matches_item` call in `any_match_in_snapshot`: no change needed (still calls `matches_item()` which now handles multi-value internally)
- `tests/filter_test.rs` — Confirmed: no direct `FilterCriterion { field, value }` struct literals exist (all construction goes through `parse_filter()`). 14 `.value` assertion sites must migrate to `.values` (recommended pattern: `assert_eq!(f.values, vec![FilterValue::Dimension(DimensionLevel::High)])` to verify both value and list length). Existing `filter_criterion_display_roundtrip` test passes without modification for single-value cases; multi-value round-trip also works by construction since the display format `field=v1,v2` is the input format.
- `tests/filter_test.rs` — Existing `validate_duplicate_scalar_field_returns_err` test asserts on substrings `"Field 'impact' specified multiple times"` and `"--only impact=value1,value2"` — both substrings appear in the new error message wording, so the test passes without modification.
- `tests/filter_test.rs` — New tests for: multi-value parsing, empty token rejection, within-list duplicate rejection, multi-value matching, multi-value display, tag OR+AND composition, tag case-sensitivity in duplicate detection
- `tests/scheduler_test.rs` — Confirmed: no direct `FilterCriterion` references or `.value` field accesses. No migration needed.

### External Dependencies

None. No new crates or external services.

---

## Open Questions

None. The syntax was already specified in WRK-055's error message and the design follows directly from the PRD and tech research.

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
| 2026-02-20 | Initial design draft (light mode) | Complete design following WRK-055 patterns |
| 2026-02-20 | Self-critique (7 agents), triage, auto-fixes | 12 auto-fixes applied, 0 directional items, 0 quality items |

## Self-Critique Summary

### Auto-fixes Applied (12)

1. Corrected "no new functions introduced" claim — two private helpers (`matches_single_value`, `parse_single_value`) and a `FilterValue::Display` impl are added
2. Added `FilterValue::Display` impl as explicit component — the design referenced a non-existent `format_value` helper; the natural Rust idiom is a `Display` impl on `FilterValue`
3. Added `parse_single_value()` helper — the design implied per-field parsers exist as callable units when they are currently inlined in match arms; extracting this is a prerequisite for comma-splitting iteration
4. Documented non-empty `values` invariant gap — `pub` fields allow construction of empty Vecs; documented as accepted with guidance to use `parse_filter()` and full-vec assertions in tests
5. Confirmed tag duplicate detection is order-sensitive — `--only tag=a,b --only tag=b,a` passes validation; documented as accepted limitation with rationale
6. Documented tag case-sensitivity in within-list duplicate detection — `tag=backend,Backend` is accepted as two distinct values; `tag=backend,backend` is rejected
7. Updated test migration from "~25+ sites" to confirmed count: 14 `.value` assertions in filter_test.rs, 0 in scheduler_test.rs; recommended full-vec assertion pattern
8. Added existing empty-value check layering — `--only impact=` still hits existing `value_str.is_empty()` check before comma splitting
9. Improved tag OR/AND risk mitigation — removed incorrect "consistent with other fields" claim; only `tag` has multi-flag AND duality
10. Added risks: comma in tag values (silent behavioral change), unbounded value list for free-text fields, tag duplicate detection order-sensitivity
11. Added `pipeline_type` scalar field tradeoff — documented asymmetry with `tag` (no multi-flag AND for pipeline_type)
12. Added line reference fragility note — references are accurate as of commit `db99c63`

### Directional Items (0)

All directional concerns raised by critics (tag order-sensitivity in duplicate detection, comma-in-tag behavioral change) were resolved as documented accepted limitations with clear rationale. No items require human input.

### Quality Items (0)

No remaining quality items after auto-fixes.

## Assumptions

Decisions made without human input during autonomous design:

1. **Light mode selected** — Direct follow-up to WRK-055 with syntax already specified in the codebase's error message. One main alternative considered (wrapper type), rejected for unnecessary complexity.
2. **Vec<FilterValue> over wrapper type** — Tech research recommended Option A (modify `FilterCriterion.value` to `Vec`). The alternative (FilterCriterionSet) adds indirection without proportional benefit.
3. **Within-list validation in parse_filter()** — Deliberate consolidation of parsing + within-list validation so `FilterCriterion` is valid by construction. Cross-criterion validation remains in `validate_filter_criteria()`.
4. **Tag duplicate detection order-sensitivity accepted** — `--only tag=a,b --only tag=b,a` passes validation despite being logically redundant. Adding canonicalization (value sorting before comparison) is not worth the complexity for this edge case.
5. **Comma-in-tag behavioral change accepted** — `tag=a,b` silently changes from matching a single tag `"a,b"` to OR(a, b). This follows directly from the PRD's comma-as-delimiter constraint and is accepted for identifier-style values.
6. **FilterValue::Display impl** — Natural Rust idiom for value-to-string conversion. Enables clean multi-value display without ad-hoc helpers.
7. **Two-pass duplicate detection** — Parse all tokens first (fail-fast on invalid), then check for duplicates in a second pass over the collected `Vec<FilterValue>`. This avoids borrow-checker complexity of single-pass approaches.
