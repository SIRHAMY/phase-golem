# Tech Research: Add Duplicate Item ID Validation to Preflight

**ID:** WRK-034
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-034_add-duplicate-item-id-validation-to-preflight_PRD.md
**Mode:** Light

## Overview

Researching the implementation approach for detecting duplicate item IDs in the orchestrator's preflight validation. The PRD proposes a `HashMap<&str, Vec<usize>>`-based approach inserted between Phase 3 (item validation) and Phase 4 (dependency graph). This research verifies that approach against the existing codebase patterns and identifies any concerns.

## Research Questions

- [x] What duplicate-detection pattern does the codebase already use?
- [x] Does the HashMap approach align with or diverge from existing patterns?
- [x] Where exactly does the new function integrate in `run_preflight`?
- [x] Are there any lifetime or ownership concerns with `&str` keys?

---

## External Research

### Landscape Overview

Duplicate detection in ordered collections is a solved problem with two standard approaches in Rust: `HashSet::insert()` for simple presence checks, and `HashMap<K, Vec<V>>` for tracking all occurrence positions. Both are O(n) time. The choice depends on error reporting requirements — the HashMap approach is strictly more informative.

### Common Patterns & Approaches

#### Pattern: HashSet::insert() Early Detection

**How it works:** Insert each element into a HashSet. `insert()` returns `false` if the value already exists. Report the duplicate at the point of detection.

**When to use:** When you only need to know *that* a duplicate exists and report the current index, not all indices.

**Tradeoffs:**
- Pro: Minimal code, idiomatic Rust
- Pro: Already used in the codebase (Phase 1 phase-name uniqueness)
- Con: Only reports the second occurrence's index, not the first

#### Pattern: HashMap<K, Vec<usize>> Full Tracking

**How it works:** Build a map from key to list of indices. After the pass, filter for entries with `len() > 1`.

**When to use:** When error messages need to report *all* positions of each duplicate, not just the second one.

**Tradeoffs:**
- Pro: Complete duplicate information (all indices)
- Pro: Single pass, O(n) time and space
- Con: Slightly more code than HashSet approach
- Con: Slightly more memory (Vec allocation per unique key)

### Standards & Best Practices

- **Error accumulation over early-exit**: Collect all errors before reporting, so operators see every problem in a single run. The codebase already follows this pattern.
- **Validation phase ordering**: Earlier phases should establish invariants consumed by later phases. Duplicate detection before dependency graph validation ensures the graph sees unique IDs.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Reporting only first duplicate found | Operators fix one, re-run, find another — poor UX | Use HashMap to collect all duplicates in one pass |
| Off-by-one in index reporting | Confusing error messages | Use `enumerate()` directly — indices are 0-based as the PRD specifies |
| Silent deduplication via HashSet/collect | Masks data integrity issues (current Phase 4 behavior) | Explicit check before any deduplication occurs |

### Key Learnings

- The HashMap approach is the right choice here because the PRD explicitly requires reporting all indices of all duplicate IDs, not just the first collision.
- This is a well-understood pattern with no meaningful risk.

---

## Internal Research

### Existing Codebase State

The preflight system in `preflight.rs` has 4 validation phases called from `run_preflight()` (lines 36-62). Each phase is a function returning `Vec<PreflightError>`, merged via `errors.extend()`. Phase 2 is gated on Phase 1 success; Phases 3 and 4 run unconditionally.

**Relevant files/modules:**
- `orchestrator/src/preflight.rs` — All 4 validation phases, `PreflightError` struct, `run_preflight()` entry point
- `orchestrator/src/types.rs` — `BacklogFile`, `BacklogItem` (id: String), `ItemStatus`
- `orchestrator/tests/preflight_test.rs` — ~691 lines of tests using helpers from `tests/common/mod.rs`
- `orchestrator/tests/common/mod.rs` — `make_item()`, `make_backlog()` helpers

**Existing patterns in use:**
- **Duplicate detection (Phase 1, lines 102-136):** Uses `HashSet::insert()` returning `false` to detect duplicate phase names. Reports at second-occurrence index only.
- **Error construction:** `PreflightError { condition, config_location, suggested_fix }` — all three fields are `String`.
- **Error collection:** Each validation function returns `Vec<PreflightError>`, collected via `errors.extend()`.
- **Item ID access:** `item.id.as_str()` for `&str` references.

### Reusable Components

- `PreflightError` struct — use directly, no changes needed
- `errors.extend()` pattern in `run_preflight` — add one line for the new phase
- `make_item()` / `make_backlog()` test helpers — create items with specific IDs for testing
- `HashMap` and `HashSet` already imported at line 1

### Constraints from Existing Code

- `run_preflight` signature is fixed: `(&OrchestrateConfig, &BacklogFile, &Path) -> Result<(), Vec<PreflightError>>`
- All parameters are immutable references — no mutation allowed
- `BacklogItem.id` is `String` — convert to `&str` via `.as_str()` for HashMap keys
- `HashMap` is already imported (line 1) — no new imports needed

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Use `HashMap<&str, Vec<usize>>` | Existing Phase 1 duplicate detection uses `HashSet::insert()` instead | PRD approach is correct — it provides richer error messages (all indices). The Phase 1 pattern doesn't meet the PRD's requirement to report all indices. No concern, just a noted divergence from existing pattern. |
| Check runs unconditionally | Phase 2 is gated on Phase 1 (`if errors.is_empty()`), but Phases 3 and 4 are unconditional | Correct — new phase should run unconditionally like Phases 3 and 4. No concern. |
| Function takes `&BacklogFile` | Phase 4 takes `&[BacklogItem]` directly | Minor style choice. Taking `&BacklogFile` is fine and consistent with Phase 3. Either works. |

---

## Critical Areas

### Correct Integration Point

**Why it's critical:** Phase 4 builds `HashSet<&str>` which silently deduplicates. If duplicates slip past, dependency graph validation operates on corrupted data.

**Why it's easy to miss:** The code at line 55 looks harmless — `validate_dependency_graph(&backlog.items)` — but the HashSet collection inside silently masks the issue.

**What to watch for:** The new `errors.extend(validate_duplicate_ids(...))` call must appear on the line between current lines 52 and 55 (between Phase 3 and Phase 4).

---

## Deep Dives

### HashSet vs HashMap Pattern Choice

**Question:** Should we use the existing `HashSet::insert()` pattern (Phase 1) or the PRD's `HashMap<&str, Vec<usize>>` approach?

**Summary:** The HashSet pattern only reports the index of the *second* occurrence of a duplicate. The PRD requires reporting *all* indices (e.g., "found at indices [0, 5]"). The HashMap approach is the only way to satisfy this requirement.

**Implications:** The new function will diverge from the Phase 1 pattern, but for good reason. This is the correct choice.

---

## Synthesis

### Open Questions

None. The approach is well-defined and the codebase integration point is clear.

### Recommended Approaches

#### Duplicate Detection Strategy

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| HashMap<&str, Vec<usize>> | Reports all indices; single pass; meets PRD requirements | Slightly more code than HashSet | You need complete duplicate information (this case) |
| HashSet::insert() | Simpler; matches Phase 1 pattern | Only reports second occurrence | You only need to flag duplicates, not report all positions |

**Initial recommendation:** HashMap<&str, Vec<usize>> — required by PRD to report all duplicate indices. Single-pass, O(n) time and space. No external dependencies.

#### Function Signature

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `fn validate_duplicate_ids(backlog: &BacklogFile) -> Vec<PreflightError>` | Consistent with Phase 3 pattern | Passes full struct when only items needed | Consistency with Phase 3 |
| `fn validate_duplicate_ids(items: &[BacklogItem]) -> Vec<PreflightError>` | Minimal data, consistent with Phase 4 | Slightly different parameter style | Minimal surface area |

**Initial recommendation:** Either works. Taking `&BacklogFile` is consistent with Phase 3; taking `&[BacklogItem]` is consistent with Phase 4. Both are correct. Lean toward `&[BacklogItem]` for minimal surface area since only `items` is accessed.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Rust HashMap docs](https://doc.rust-lang.org/std/collections/struct.HashMap.html) | Official docs | Entry API for `or_insert_with` pattern |
| [Rust HashSet docs](https://doc.rust-lang.org/std/collections/struct.HashSet.html) | Official docs | Alternative simpler approach (not recommended here) |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Light external + internal research | Confirmed PRD approach is sound; identified exact integration point at preflight.rs lines 52-55; noted divergence from Phase 1 HashSet pattern is intentional and correct |
