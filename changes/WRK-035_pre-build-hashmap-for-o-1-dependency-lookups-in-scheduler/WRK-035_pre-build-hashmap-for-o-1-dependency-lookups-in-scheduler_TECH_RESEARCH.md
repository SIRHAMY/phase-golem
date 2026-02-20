# Tech Research: Pre-build HashMap for O(1) Dependency Lookups in Scheduler

**ID:** WRK-035
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-035_pre-build-hashmap-for-o-1-dependency-lookups-in-scheduler_PRD.md
**Mode:** Light

## Overview

Researching the approach for replacing `.iter().find()` linear scans with a pre-built `HashMap<&str, &BacklogItem>` in the scheduler's dependency-checking and target-lookup functions. The change is a straightforward algorithmic refactoring — the main questions are around Rust lifetime patterns for borrowed HashMaps and confirming codebase integration points.

## Research Questions

- [x] What is the idiomatic Rust pattern for `HashMap<&str, &T>` built from a slice?
- [x] Are there gotchas with lifetime propagation through function signatures?
- [x] What are the exact integration points in the current codebase?
- [x] What test changes are needed?

---

## External Research

### Landscape Overview

Building a `HashMap<&'a str, &'a BacklogItem>` from a slice once per cycle and passing it by shared reference through call chains is idiomatic Rust. The borrow checker natively enforces the key invariant: the slice the map borrows from cannot be moved or mutated while the map exists, making this pattern safe by construction.

### Common Patterns & Approaches

#### Pattern: Build-once, share-by-reference

**How it works:** Create a helper `fn build_item_lookup<'a>(items: &'a [BacklogItem]) -> HashMap<&'a str, &'a BacklogItem>`. The lifetime `'a` ties both the key (`item.id.as_str()`) and the value (`&item`) to the same source slice. Build once at the top of the scheduling cycle, pass as `&HashMap<&str, &BacklogItem>` to all functions that need it.

**When to use:** Any time you have a stable slice and need repeated O(1) lookups by a borrowed key within a bounded operation.

**Tradeoffs:**
- Pro: Zero allocation of item data (keys and values are references)
- Pro: O(n) construction, O(1) lookup
- Pro: Borrow checker prevents use of stale maps at compile time
- Con: Lifetime annotation propagates to receiving functions (minimal in this case since lookups are local)

**References:**
- [Rust std::collections::HashMap](https://doc.rust-lang.org/std/collections/struct.HashMap.html) — Official docs
- [Rust Users Forum: Lifetimes storing a reference in a HashMap](https://users.rust-lang.org/t/lifetimes-storing-a-reference-of-an-object-in-a-hashmap/72650) — Community patterns

#### Pattern: Index-based indirection (alternative)

**How it works:** Store `usize` indices into the original Vec (`HashMap<String, usize>`) rather than references.

**When to use:** When lifetime annotations cause friction across many function signatures, or when source data may be mutated.

**Tradeoffs:**
- Pro: Avoids lifetime propagation through signatures
- Con: More verbose at call sites (`&items[idx]`)
- Con: Less zero-cost than direct references

**Not recommended for this use case** — the direct reference approach is simpler and the lifetime propagation is minimal.

### Technologies & Tools

| Technology | Purpose | Relevance |
|------------|---------|-----------|
| `std::collections::HashMap` | Standard library hash map (backed by hashbrown) | The only tool needed; no external crates required |
| `HashMap::with_capacity(n)` | Pre-sized construction | Avoids rehashing; use `items.len()` as capacity |
| `Borrow` trait | Ergonomic lookups | `HashMap<&str, V>` accepts `&str` at `.get()` via `Borrow` |

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid | Applies Here? |
|---------|-------------------|--------------|---------------|
| Using `into_iter()` instead of `iter()` | Consumes the Vec, destroying the source | Use `.iter()` which yields `&T` | Yes — must use `iter()` |
| Lifetime annotations becoming infectious | Functions returning borrowed data must propagate `'a` | Lookups are local; no return of borrowed data | No — not an issue |
| `get_mut()` variance issues | Cryptic lifetime errors with mutable access | Use `get()` only (read-only map) | No — read-only |
| Mutating source while map exists | Borrow checker prevents this | Compile-time enforced | No — borrow checker handles it |
| Iterating HashMap is O(capacity) | Visits empty buckets | Don't iterate; use point lookups | No — only point lookups |

### Key Learnings

- The PRD's proposed approach is the canonical Rust pattern. No surprises.
- `with_capacity` is a minor but idiomatic optimization.
- The `Borrow` trait makes `dep_id.as_str()` ergonomic at `.get()` call sites.
- Aligns with the project's style guide: standalone pure function, data/behavior separation.

---

## Internal Research

### Existing Codebase State

The scheduler uses linear scans (`.iter().find()`) for all item-by-ID lookups. The codebase already uses `HashMap` patterns extensively and demonstrates comfort with lifetime annotations on borrowed references.

**Relevant files/modules:**

- `src/scheduler.rs` (1879 lines) — All functions to modify
- `src/types.rs` — `BacklogItem` (line 185, `id: String`) and `BacklogFile` (line 228, `items: Vec<BacklogItem>`)
- `tests/scheduler_test.rs` — Test functions calling `unmet_dep_summary`

**Existing patterns in use:**

- `HashMap<String, PipelineConfig>` passed by reference (`&HashMap<...>`) throughout scheduler — line 153
- `HashMap<String, String>` for `previous_summaries` tracking — line 549
- `sorted_in_progress_items<'a>()` returns `Vec<&'a BacklogItem>` — demonstrates lifetime patterns with borrowed items (line 304)
- `sorted_ready_items()` returns `Vec<&BacklogItem>` — borrowed item references (line 287)
- `use std::collections::HashMap;` already imported at line 1

### Function Signatures and Call Graph

```
run_scheduler() [line 528]
  ├─ select_actions() [line 149] (called at lines 764, 766)
  │   ├─ skip_for_unmet_deps() [line 382] × 4 calls (lines 186, 204, 218, 232)
  │   │   └─ unmet_dep_summary() [line 358]
  │   └─ build_run_phase_action() [line 435]
  ├─ select_targeted_actions() [line 1000] (called at line 756)
  │   ├─ skip_for_unmet_deps() [line 382] (line 1014)
  │   │   └─ unmet_dep_summary() [line 358]
  │   └─ build_run_phase_action() [line 435]
  ├─ advance_to_next_active_target() [line 469] (called at line 657)
  │   └─ [linear scan for target item, line 478]
  └─ [diagnostic logging] (lines 772-780)
      └─ unmet_dep_summary() [line 358] (line 777)
```

**Current function signatures:**

| Function | Current Signature | Line |
|----------|------------------|------|
| `unmet_dep_summary` | `pub fn unmet_dep_summary(item: &BacklogItem, all_items: &[BacklogItem]) -> Option<String>` | 358 |
| `skip_for_unmet_deps` | `fn skip_for_unmet_deps(item: &BacklogItem, all_items: &[BacklogItem]) -> bool` | 382 |
| `advance_to_next_active_target` | `pub fn advance_to_next_active_target(targets: &[String], current_index: usize, items_completed: &[String], snapshot: &BacklogFile) -> usize` | 469 |
| `select_targeted_actions` | `pub fn select_targeted_actions(snapshot: &BacklogFile, running: &RunningTasks, _config: &ExecutionConfig, pipelines: &HashMap<String, PipelineConfig>, target_id: &str) -> Vec<SchedulerAction>` | 1000 |

**Linear scan locations (in scope):**

| Location | Line | Current Code |
|----------|------|-------------|
| `unmet_dep_summary` | 366 | `all_items.iter().find(\|i\| i.id == *dep_id)` |
| `advance_to_next_active_target` | 478 | `snapshot.items.iter().find(\|i\| i.id == *target)` |
| `select_targeted_actions` | 1008 | `snapshot.items.iter().find(\|i\| i.id == target_id)` |

### Reusable Components

- `std::collections::HashMap` already imported in `scheduler.rs`
- Existing lifetime patterns in `sorted_*_items` helpers provide precedent for `<'a>` annotations
- No existing `build_lookup` helper to reuse — this will be new

### Constraints from Existing Code

- `unmet_dep_summary` is `pub` — signature change affects `tests/scheduler_test.rs` (4 call sites at lines 1798, 1808, 1824, 1852)
- `advance_to_next_active_target` is `pub` — but only called internally from `run_scheduler`
- `BacklogItem.id` is `String` — `as_str()` needed for `&str` keys
- Snapshot loaded once per cycle (line 607), immutable during cycle — borrow checker enforces this

---

## PRD Concerns

No significant concerns. The PRD's approach aligns perfectly with idiomatic Rust patterns.

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| `HashMap<&str, &BacklogItem>` concrete type | Confirmed as the canonical pattern | None — approach is correct |
| Build from snapshot items slice | Standard `.iter().map().collect()` idiom | Minor enhancement: use `with_capacity` |
| Tests updated in same change | 4 test call sites confirmed | Straightforward signature updates |
| No convenience wrapper needed | Correct — only test file is external caller | Keeps API minimal |

---

## Critical Areas

No critical areas identified. This is a mechanical refactoring with well-understood patterns and clear integration points. The Rust compiler will catch any signature mismatches or lifetime errors at compile time.

---

## Deep Dives

None needed for light mode research. The pattern is well-understood.

---

## Synthesis

### Open Questions

None. All questions from the PRD and research are resolved.

### Recommended Approaches

#### HashMap Construction

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `items.iter().map(\|i\| (i.id.as_str(), i)).collect()` | Concise, idiomatic | `with_capacity` requires separate variable | Simplicity preferred |
| `for` loop with `HashMap::with_capacity` | Explicit capacity, clear | Slightly more verbose | Performance optimization preferred |

**Initial recommendation:** Either approach works. The `.collect()` form is more idiomatic Rust and the capacity optimization is negligible at current backlog sizes. Prefer `.collect()` for brevity unless the style guide favors explicitness.

#### Helper Function Placement

**Initial recommendation:** Place `build_item_lookup` near the existing sorting helpers (around line 284 in `scheduler.rs`), grouping it with the other slice-processing utility functions.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Rust std::collections::HashMap](https://doc.rust-lang.org/std/collections/struct.HashMap.html) | Official docs | `.collect()` patterns, `Borrow`-based `.get()` |
| [Rust Users: HashMap keys & lifetimes](https://users.rust-lang.org/t/hashmap-keys-lifetimes/47899) | Forum | Variance, `get()` vs `get_mut()` with borrowed keys |
| [Rust Users: Lifetimes in HashMap](https://users.rust-lang.org/t/lifetimes-storing-a-reference-of-an-object-in-a-hashmap/72650) | Forum | Direct references vs index-based alternatives |
| [pretzelhammer: Common Rust lifetime misconceptions](https://github.com/pretzelhammer/rust-blog/blob/master/posts/common-rust-lifetime-misconceptions.md) | Blog | Deep reference on lifetime annotations |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Light external research: Rust HashMap borrowed patterns | Confirmed PRD approach is idiomatic; no gotchas for this use case |
| 2026-02-20 | Light internal research: scheduler codebase analysis | Mapped all integration points, confirmed 4 test call sites, documented call graph |
| 2026-02-20 | PRD analysis | No concerns; PRD well-aligned with research findings |

## Assumptions

- **Autonomous mode decisions:** No human was available for Q&A. All PRD assumptions were validated by research; no deviations or concerns to escalate. The light mode designation was appropriate — no deep dives were needed.
