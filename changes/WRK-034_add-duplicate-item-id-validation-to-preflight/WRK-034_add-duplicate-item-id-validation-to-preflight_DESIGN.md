# Design: Add Duplicate Item ID Validation to Preflight

**ID:** WRK-034
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-034_add-duplicate-item-id-validation-to-preflight_PRD.md
**Tech Research:** ./WRK-034_add-duplicate-item-id-validation-to-preflight_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add a new validation phase to `run_preflight` that detects duplicate item IDs in the backlog before dependency graph validation runs. The implementation uses a `HashMap<&str, Vec<usize>>` to collect all indices per ID in a single O(n) pass, then reports an error for each ID with more than one occurrence. The function validates all items regardless of status, since IDs must be globally unique. This is a single-function addition with a one-line integration into the existing preflight flow.

---

## System Design

### High-Level Architecture

No new components or modules. This adds one private function (`validate_duplicate_ids`) to `preflight.rs` and one `errors.extend()` call in `run_preflight`. The function follows the identical pattern used by all existing validation phases: accept immutable references, return `Vec<PreflightError>`.

```
run_preflight
  ├── Phase 1: validate_structure(config)
  ├── Phase 2: probe_workflows(config, project_root)  [gated on Phase 1]
  ├── Phase 3: validate_items(config, backlog)
  ├── NEW: validate_duplicate_ids(&backlog.items)  ← added here
  └── Phase 4: validate_dependency_graph(&backlog.items)
```

### Component Breakdown

#### validate_duplicate_ids

**Purpose:** Detect and report all item IDs that appear more than once in the backlog, regardless of item status.

**Visibility:** Private (`fn`, not `pub fn`), matching `validate_structure`, `validate_items`, and other internal phase functions. Called only from `run_preflight`. Tests exercise it indirectly through `run_preflight`.

**Responsibilities:**
- Build a `HashMap<&str, Vec<usize>>` mapping each item ID to its list of 0-based indices
- Filter for entries where the index list has length > 1
- Sort duplicate entries by first occurrence index for deterministic output
- Produce one `PreflightError` per duplicate ID, listing all indices in ascending order

**Interfaces:**
- Input: `&[BacklogItem]` — the full items slice from BacklogFile (all statuses included)
- Output: `Vec<PreflightError>` — one error per duplicate ID (empty if no duplicates), sorted by first occurrence index

**Dependencies:** `HashMap` (already imported), `BacklogItem` (already imported), `PreflightError` (local)

### Data Flow

1. `run_preflight` calls `validate_duplicate_ids(&backlog.items)` after Phase 3, unconditionally
2. The function iterates over items with `enumerate()`, building `HashMap<&str, Vec<usize>>` via `entry().or_insert_with(Vec::new).push(index)`
3. After the pass, it collects entries with `vec.len() > 1` into a Vec
4. It sorts these entries by first occurrence index (`entries.sort_by_key(|(_, indices)| indices[0])`)
5. For each duplicate, it constructs a `PreflightError` with the ID and all indices (indices are already in ascending order since we iterate sequentially)
6. Returns the error vector; `run_preflight` extends its error collection
7. Phase 4 then runs regardless of whether duplicates were found (all errors accumulated)

### Key Flows

#### Flow: Duplicate Detection (Happy Path — No Duplicates)

> All item IDs are unique; validation passes silently.

1. **Iterate** — Walk all items (regardless of status) with `enumerate()`, insert each `(id, index)` into HashMap
2. **Filter** — Check for entries with `len() > 1` — none found
3. **Return** — Return empty `Vec<PreflightError>`, Phase 4 proceeds normally

#### Flow: Duplicate Detection (Error Path — Duplicates Found)

> One or more IDs appear multiple times; validation reports all of them.

1. **Iterate** — Walk all items (regardless of status) with `enumerate()`, insert each `(id, index)` into HashMap
2. **Filter** — Find entries with `len() > 1`
3. **Sort** — Sort duplicate entries by first occurrence index for deterministic output
4. **Build errors** — For each duplicate ID, create a `PreflightError`:
   - `condition`: `Duplicate item ID "WRK-034" found at indices [0, 5]`
   - `config_location`: `BACKLOG.yaml → items`
   - `suggested_fix`: `Remove or rename the duplicate item so each ID is unique`
5. **Return** — Return `Vec<PreflightError>` with one entry per duplicate ID
6. **Continue** — `run_preflight` continues to Phase 4 (dependency graph), accumulating all errors before returning

**Edge cases:**
- Empty backlog — HashMap is empty, no duplicates, returns empty vec
- Single item — Only one entry, `len() == 1`, no duplicates
- Three-way duplicate — Same ID at indices [0, 3, 7] — single error: `Duplicate item ID "WRK-001" found at indices [0, 3, 7]`
- Multiple distinct duplicates — Each duplicate ID produces its own error, sorted by first occurrence
- All items same ID — Single error listing all indices (e.g., `[0, 1, 2, 3]`)

---

## Technical Decisions

### Key Decisions

#### Decision: Function Signature — `&[BacklogItem]` vs `&BacklogFile`

**Context:** Existing phases use both patterns: Phase 3 takes `(&OrchestrateConfig, &BacklogFile)`, Phase 4 takes `&[BacklogItem]`.

**Decision:** Use `fn validate_duplicate_ids(items: &[BacklogItem]) -> Vec<PreflightError>`

**Rationale:** The function only needs the items slice, not the full BacklogFile. Taking `&[BacklogItem]` matches Phase 4's pattern, makes the data dependency explicit, and simplifies unit testing (no need to construct a full BacklogFile).

**Consequences:** Consistent with Phase 4; call site passes `&backlog.items`.

#### Decision: HashMap<&str, Vec<usize>> Over HashSet::insert()

**Context:** Phase 1 uses `HashSet::insert()` for duplicate phase name detection, which only identifies the second occurrence.

**Decision:** Use `HashMap<&str, Vec<usize>>` to track all indices per ID.

**Rationale:** PRD requires reporting all indices of all duplicates (e.g., `[0, 5]`), not just the second occurrence. The HashMap approach satisfies this in a single pass. The `&str` keys borrow from the input slice and are valid for the function's lifetime.

**Consequences:** Slightly more code than HashSet but strictly more informative error messages. Intentional divergence from Phase 1's pattern, well justified by different requirements. A code comment should explain this divergence for future maintainers.

#### Decision: Unconditional Execution (Not Gated on Prior Phase Success)

**Context:** Phase 2 is gated on Phase 1 success. Phases 3 and 4 run unconditionally.

**Decision:** Run the duplicate check unconditionally, like Phases 3 and 4.

**Rationale:** Duplicate IDs are a backlog data issue independent of config structure or workflow file existence. Backlog parse errors are caught at YAML deserialization time (before preflight runs), so this function always receives a valid `&[BacklogItem]`. Reporting duplicate IDs even when other errors exist gives operators the full picture in a single run.

**Consequences:** Operators see all error categories at once; no unnecessary re-runs.

#### Decision: Deterministic Error Ordering

**Context:** HashMap iteration order is non-deterministic in Rust, which would make error output order non-deterministic.

**Decision:** Sort duplicate entries by their first index before generating errors.

**Rationale:** Deterministic output makes tests reliable without depending on HashMap iteration order. Sorting by first index provides a natural reading order (errors appear in the order duplicates first appear in the backlog). The sort operates only on the filtered duplicate entries (typically 0-2 entries), not the full HashMap, so the cost is negligible.

**Consequences:** Adds a `sort_by_key` call after filtering. Negligible cost for small backlogs.

#### Decision: Defer Nice-to-Have Item Titles in Error Messages

**Context:** PRD lists including item titles alongside IDs in error messages as a Nice-to-Have.

**Decision:** Defer to a future enhancement. Error messages report only ID and indices.

**Rationale:** Current implementation reports indices which operators can cross-reference with `BACKLOG.yaml`. Adding titles increases error message complexity (long titles, formatting concerns) for marginal benefit. Can be added later if operators report difficulty identifying items in large backlogs.

**Consequences:** Simpler error format. Future enhancement possible without changing function signature.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Diverge from Phase 1 pattern | Inconsistency with Phase 1's HashSet approach | Complete duplicate information (all indices) | PRD requires reporting all indices; Phase 1's pattern can't deliver this |
| Vec allocation per unique ID | Slightly more memory than HashSet | Full index tracking per duplicate | Backlog sizes are <1000 items; memory is negligible |
| Titles omitted from error messages | Less context in error output | Simpler initial implementation | Indices sufficient for cross-reference; titles can be added later |

---

## Alternatives Considered

### Alternative: HashSet::insert() (Phase 1 Pattern)

**Summary:** Use the same `HashSet::insert()` returning `false` pattern as Phase 1's duplicate phase name detection.

**How it would work:**
- Insert each ID into a HashSet
- When `insert()` returns `false`, report the current index as a duplicate

**Pros:**
- Consistent with existing Phase 1 pattern
- Slightly less code

**Cons:**
- Only reports the index of the second occurrence, not the first
- Cannot report three-way duplicates with all indices
- Fails to meet PRD requirement: "the 0-based array indices of all items sharing that ID"

**Why not chosen:** Does not satisfy the PRD's explicit requirement to report all duplicate indices.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Non-deterministic error order from HashMap | Flaky tests | Medium | Sort duplicates by first occurrence index before generating errors |

---

## Integration Points

### Existing Code Touchpoints

- `preflight.rs:run_preflight()` (line 52-55) — Add one `errors.extend(validate_duplicate_ids(&backlog.items))` line between Phase 3 and Phase 4
- `preflight.rs` — Add private `validate_duplicate_ids` function (new ~20-line function)
- `preflight.rs` doc comment (line 28-34) — Update phase listing to include duplicate ID validation
- `preflight_test.rs` — Add test cases for duplicate ID validation (empty, single, no duplicates, one pair, multiple distinct, three-way)

### External Dependencies

None. Uses only `std::collections::HashMap` (already imported) and existing types.

---

## Open Questions

None.

---

## Assumptions

- Running autonomously; no human available for questions. All decisions above were made using PRD requirements and tech research findings as guides.
- `BacklogItem.id` is a required `String` field (enforced by serde). Items with missing IDs fail at YAML parse time before preflight runs.
- The function validates all items regardless of status — IDs must be globally unique across New, Scoping, Ready, InProgress, Done, and Blocked items.
- Backlog parse/deserialization errors are caught before preflight runs, so `validate_duplicate_ids` always receives a valid `&[BacklogItem]`.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Initial design draft | Single-function addition with HashMap-based duplicate detection; deterministic error ordering |
| 2026-02-13 | Self-critique (7 agents) + auto-fix | Added: status-independence clarification, indices sorting guarantee, deferred Nice-to-Have rationale, function visibility spec, three-way/all-same-ID edge cases, backlog parse safety assumption, Phase 4 continuation flow |
