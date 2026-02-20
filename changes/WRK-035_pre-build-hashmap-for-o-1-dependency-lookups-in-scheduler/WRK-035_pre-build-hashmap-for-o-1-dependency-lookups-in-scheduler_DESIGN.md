# Design: Pre-build HashMap for O(1) Dependency Lookups in Scheduler

**ID:** WRK-035
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-035_pre-build-hashmap-for-o-1-dependency-lookups-in-scheduler_PRD.md
**Tech Research:** ./WRK-035_pre-build-hashmap-for-o-1-dependency-lookups-in-scheduler_TECH_RESEARCH.md
**Mode:** Light

## Overview

Replace linear-scan item-by-ID lookups in the scheduler with O(1) HashMap lookups. A new helper function `build_item_lookup` constructs a `HashMap<&str, &BacklogItem>` from the items slice. Each top-level scheduling function (`select_actions`, `select_targeted_actions`) builds its own map and threads it to callees. These functions are in a three-way dispatch (targets vs. filter vs. normal), so at most one builds a map per cycle; the diagnostic logging block may build a second. No new components or architectural changes — this is a mechanical signature refactoring within `src/scheduler.rs` and its test file.

---

## System Design

### High-Level Architecture

No new components. The change adds one helper function and threads a lookup map through existing function signatures:

```
build_item_lookup(&[BacklogItem]) -> HashMap<&str, &BacklogItem>
        │
        ├─ select_actions() builds map, passes to skip_for_unmet_deps()
        │       └─ skip_for_unmet_deps() passes to unmet_dep_summary()
        │               └─ unmet_dep_summary() uses map.get() instead of .iter().find()
        │
        ├─ select_targeted_actions() builds map, uses map.get() for target lookup,
        │       passes to skip_for_unmet_deps()
        │
        ├─ advance_to_next_active_target() receives map, uses map.get() for target lookup
        │
        └─ run_scheduler() diagnostic logging builds map, passes to unmet_dep_summary()
```

### Component Breakdown

#### `build_item_lookup` (new helper)

**Purpose:** Construct a borrowed HashMap from an items slice for O(1) ID lookups.

**Responsibilities:**
- Accept `&[BacklogItem]`, return `HashMap<&str, &BacklogItem>`
- Single source of truth for lookup map construction

**Interfaces:**
- Input: `items: &'a [BacklogItem]`
- Output: `HashMap<&'a str, &'a BacklogItem>`

**Dependencies:** `std::collections::HashMap` (already imported)

### Data Flow

1. Caller builds map via `build_item_lookup(&snapshot.items)` — always from the **full** `snapshot.items`, not filtered subsets, since dependency resolution needs visibility into all items regardless of status
2. Map is passed as `&HashMap<&str, &BacklogItem>` to functions needing item-by-ID lookup
3. Receiving functions call `map.get(dep_id.as_str())` instead of `items.iter().find(|i| i.id == *dep_id)`
4. Map is dropped when the caller's scope ends (same scheduling cycle)

**Lifetime invariant:** The HashMap borrows from the items slice. The Rust borrow checker enforces at compile time that the slice cannot be mutated or moved while the map exists. No runtime checks are needed — any violation is a compile error.

**Lifetime elision:** Receiving functions accept `&HashMap<&str, &BacklogItem>` without explicit lifetime parameters. Since they don't return borrowed data from the map, Rust's lifetime elision rules apply and no `'a` annotation is needed on their signatures. Only `build_item_lookup` itself needs the `'a` annotation.

### Key Flows

#### Flow: select_actions dependency checking

> Build map once, reuse across all four candidate loops in `select_actions`.

1. **Build map** — `let item_lookup = build_item_lookup(&snapshot.items);` at top of `select_actions`
2. **Ready promotions** — `skip_for_unmet_deps(item, &item_lookup)` replaces `skip_for_unmet_deps(item, &snapshot.items)`
3. **InProgress phases** — Same substitution
4. **Scoping phases** — Same substitution
5. **New triage** — Same substitution

**Edge cases:**
- Empty items slice — HashMap is empty, all `.get()` calls return `None`, same behavior as `.iter().find()` returning `None`
- Duplicate IDs — Cannot happen; enforced by `validate_duplicate_ids` at load time. HashMap insert overwrites, but uniqueness is guaranteed.

#### Flow: select_targeted_actions target + dependency check

> Build map once, use for both target-item lookup and dependency checking. Note: `select_targeted_actions` and `select_actions` are in a three-way dispatch — only one is called per cycle, so maps are never built redundantly between them.

1. **Build map** — `let item_lookup = build_item_lookup(&snapshot.items);`
2. **Target lookup** — `item_lookup.get(target_id)` replaces `snapshot.items.iter().find(|i| i.id == target_id)`
3. **Dependency check** — `skip_for_unmet_deps(target, &item_lookup)` replaces `skip_for_unmet_deps(target, &snapshot.items)`

#### Flow: advance_to_next_active_target target lookup

> Caller passes existing map; function uses it for target lookups.

1. **Receive map** — Function signature adds `item_lookup: &HashMap<&str, &BacklogItem>` parameter
2. **Target lookup** — `item_lookup.get(target.as_str())` replaces `snapshot.items.iter().find(|i| i.id == *target)`

**Edge cases:**
- Target not found — `map.get()` returns `None`, same as `.iter().find()` returning `None`. Existing match arm handles this.

#### Flow: Diagnostic logging in run_scheduler

> Build map from full snapshot for diagnostic dependency summary. This is the only case where a second map may be built in the same cycle (the first being from whichever dispatch branch ran). The diagnostic block only executes when `actions.is_empty() && running.is_empty()`, so the cost is negligible.

1. **Build map** — `let item_lookup = build_item_lookup(&snapshot.items);` in the diagnostic block
2. **Dependency summary** — `unmet_dep_summary(i, &item_lookup)` replaces `unmet_dep_summary(i, &snapshot.items)`

---

## Technical Decisions

### Key Decisions

#### Decision: Direct references vs. index-based indirection

**Context:** Two patterns exist for borrowed lookup maps: `HashMap<&str, &T>` (direct references) and `HashMap<String, usize>` (indices into the Vec).

**Decision:** Use direct references (`HashMap<&str, &BacklogItem>`).

**Rationale:** Simpler call sites (no `&items[idx]` indirection), zero-cost borrows, lifetime propagation is minimal (no borrowed data returned from lookups — all usage is local). Tech research confirmed this as the canonical Rust pattern.

**Consequences:** Functions receiving the map need no access to the original items slice for lookups.

#### Decision: Helper function placement

**Context:** The map construction code could be inlined at each call site or extracted to a shared helper.

**Decision:** Extract to `build_item_lookup` helper, placed near the existing sorting helpers (~line 284 area in `scheduler.rs`).

**Rationale:** Avoids duplication across 3 call sites (`select_actions`, `select_targeted_actions`, diagnostic logging). Groups with similar slice-processing helpers (`sorted_ready_items`, `sorted_in_progress_items`, etc.).

**Consequences:** Single place to change if the construction logic ever needs adjustment.

#### Decision: Use `.collect()` idiom vs. explicit `for` loop with `with_capacity`

**Context:** HashMap can be built via `items.iter().map(...).collect()` or an explicit loop with `HashMap::with_capacity(items.len())`.

**Decision:** Use `.iter().map().collect()` idiom.

**Rationale:** More concise and idiomatic. The capacity optimization is negligible at current backlog sizes (tens of items). The `FromIterator` impl for HashMap already allocates reasonable capacity.

**Consequences:** Slightly less control over initial capacity, but no practical impact.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Lifetime annotation on helper | `build_item_lookup` needs `'a`; receiving functions do not (lifetime elision applies) | Zero-allocation borrowed lookups | Only one function carries the annotation; callers use `&HashMap<&str, &BacklogItem>` without `'a` |
| Map build per dispatch + diagnostics | At most 2 × O(n) construction per cycle (one from dispatch, one from diagnostic block) | Self-contained functions — no need to thread map through `run_scheduler` | O(n) on tens-to-hundreds of items is trivial; diagnostic build only happens on idle cycles |

---

## Alternatives Considered

### Alternative: Index-based HashMap (`HashMap<String, usize>`)

**Summary:** Store Vec indices instead of direct references.

**How it would work:**
- Build `HashMap<String, usize>` mapping item ID to position in the Vec
- Lookup returns an index; caller dereferences via `&items[idx]`

**Pros:**
- No lifetime annotations needed

**Cons:**
- More verbose at every call site (`&items[idx]`)
- Owned `String` keys require cloning (more allocation)
- Index invalidation risk if Vec is mutated (not a concern here, but less safe by construction)

**Why not chosen:** Direct reference approach is simpler, zero-cost, and the lifetime propagation is minimal. Tech research confirmed reference-based is the idiomatic choice for this pattern.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Signature change breaks external callers | Low — only `tests/scheduler_test.rs` calls `unmet_dep_summary` | Low — scope is well-mapped | Update 4 test call sites in the same change |
| Lifetime errors during implementation | Low — contained `'a` lifetime | Low — straightforward pattern | Compiler catches all errors; tech research confirmed no gotchas |

---

## Integration Points

### Existing Code Touchpoints

- `src/scheduler.rs:~284` — New `build_item_lookup` helper function (insertion point near sorting helpers)
- `src/scheduler.rs:358` — `unmet_dep_summary` signature: `all_items: &[BacklogItem]` → `item_lookup: &HashMap<&str, &BacklogItem>`; body: `.iter().find()` → `.get()`
- `src/scheduler.rs:382` — `skip_for_unmet_deps` signature: same substitution; body: pass-through to `unmet_dep_summary`
- `src/scheduler.rs:149` — `select_actions`: add `let item_lookup = build_item_lookup(&snapshot.items);` and pass to `skip_for_unmet_deps` calls
- `src/scheduler.rs:469` — `advance_to_next_active_target` signature: add `item_lookup` parameter; body: `.iter().find()` → `.get()`
- `src/scheduler.rs:1000` — `select_targeted_actions`: build map, use for target lookup and pass to `skip_for_unmet_deps`
- `src/scheduler.rs:772-780` — Diagnostic logging block: build map, pass to `unmet_dep_summary`
- `tests/scheduler_test.rs` — 4 call sites (~lines 1798, 1808, 1824, 1852): each test builds `let lookup = build_item_lookup(&items);` from its test fixture items, then passes `&lookup` to `unmet_dep_summary`. `build_item_lookup` must be `pub` or `pub(crate)` for test access.

### External Dependencies

None. Uses only `std::collections::HashMap` (already imported).

---

## Open Questions

None. All questions resolved during PRD and tech research phases.

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
| 2026-02-20 | Initial design draft (light mode) | Straightforward signature refactoring; one helper, 6 function touchpoints, 4 test sites |
| 2026-02-20 | Self-critique (7 agents) + triage | Auto-fixed: clarified three-way dispatch (no redundant builds), lifetime elision for receivers, snapshot immutability invariant, test update pattern, full-snapshot requirement. No directional or quality items remaining. |
