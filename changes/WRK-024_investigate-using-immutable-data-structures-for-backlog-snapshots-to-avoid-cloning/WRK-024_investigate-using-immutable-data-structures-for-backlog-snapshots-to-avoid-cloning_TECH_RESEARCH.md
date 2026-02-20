# Tech Research: Persistent Data Structures for Backlog Snapshots

**ID:** WRK-024
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-024_investigate-using-immutable-data-structures-for-backlog-snapshots-to-avoid-cloning_PRD.md
**Mode:** Medium

## Overview

Investigating whether replacing `Vec<BacklogItem>` with a persistent data structure (e.g., `im::Vector` / `imbl::Vector`) or simpler alternative (`Arc<Vec>`, snapshot caching) can reduce the cost of sharing backlog snapshots between the coordinator actor and scheduler. The coordinator deep-clones the entire backlog on every `get_snapshot()` call, and the scheduler makes multiple snapshot requests per loop iteration. At current scale (5-30 items) this is likely cheap, but the pattern scales linearly with backlog size.

## Research Questions

- [x] What patterns exist for cheap snapshot sharing in Rust actor systems?
- [x] Is `im::Vector` (or its maintained fork `imbl`) a viable replacement for `Vec<BacklogItem>` with full serde compatibility?
- [x] How large is the API migration surface — how many call sites use `&[BacklogItem]` slices?
- [x] Would simpler approaches (snapshot caching, `Arc<Vec>`) achieve sufficient benefit with less complexity?
- [x] At what backlog size does the clone cost actually become meaningful?
- [x] Is the `im` crate maintained and suitable for production use?

---

## External Research

### Landscape Overview

The Rust ecosystem offers three main approaches for reducing collection clone costs in snapshot-sharing patterns:

1. **Persistent data structures** — Tree-based collections with structural sharing (O(1) clone via reference counting on shared tree nodes). Trade random access and iteration performance for cheap cloning.
2. **Reference-counted wrappers** — `Arc<Vec<T>>` with `Arc::make_mut` for copy-on-write semantics. Clone is O(1) (atomic refcount increment). Defers deep clone to mutation time, only when outstanding references exist.
3. **Application-level caching** — Reduce clone frequency rather than per-clone cost. Cache one snapshot per tick and reuse across multiple consumers.

The key tension is **constant-factor overhead** at small scale vs **asymptotic advantage** at large scale. For collections under ~100-200 elements, `Vec` typically outperforms persistent structures even for clone-heavy workloads due to CPU cache optimization for contiguous memory.

### Common Patterns & Approaches

#### Pattern: Persistent Data Structures (imbl::Vector)

**How it works:** Replace `Vec<T>` with `imbl::Vector<T>`, a relaxed radix balanced (RRB) tree. Cloning increments a reference count on the shared tree root — no element data is copied. Mutations create new tree nodes for the modified path while sharing unmodified nodes with prior versions. At small sizes (<128 elements), stores elements inline.

**When to use:** High clone/mutation ratio with large collections (hundreds+ elements). Read-heavy snapshot sharing where mutations are infrequent relative to reads.

**Tradeoffs:**
- Pro: Clone is O(1) regardless of collection size
- Pro: Structural sharing means mutations only copy the modified subtree
- Con: Indexed access is O(log n) vs O(1) for Vec
- Con: Iteration involves pointer chasing (cache-unfriendly)
- Con: **No `Deref<Target=[T]>`** — cannot produce `&[BacklogItem]` slices, which is the largest migration cost
- Con: Higher constant factors at small scale (<100 items)
- Con: Adds third-party dependency with transitive deps

**Common technologies:** `imbl` v2.0.2 (maintained fork), `rpds` (alternative)

**References:**
- [imbl GitHub (maintained fork)](https://github.com/jneem/imbl) — actively maintained successor to `im`
- [im::Vector API docs](https://docs.rs/im/latest/im/vector/struct.Vector.html) — detailed performance characteristics
- [im crate docs](https://docs.rs/im/latest/im/) — notes Vec beats Vector at small sizes

#### Pattern: Arc<Vec<T>> with Copy-on-Write

**How it works:** Wrap the collection in `Arc<Vec<T>>`. Snapshot distribution uses `Arc::clone()` (O(1) atomic refcount increment). Mutations use `Arc::make_mut(&mut self.data)` which checks the refcount: if 1 (no outstanding snapshots), gives `&mut Vec<T>` directly at zero cost; if >1, deep-clones first.

**When to use:** When mutations usually happen after snapshot consumers have dropped their references. In an actor model with message passing, this is typically the case since consumers process and drop snapshots within their handler.

**Tradeoffs:**
- Pro: Zero new dependencies (stdlib only)
- Pro: Inner `Vec<T>` retains all standard APIs — `&[T]` slices, indexing, contiguous iteration
- Pro: Mutation is zero-cost when no outstanding snapshots (typical case in sequential actor model)
- Con: When clone does happen, it's a full deep clone (no structural sharing)
- Con: Slightly more complex mutation sites (`Arc::make_mut()` instead of direct access)

**References:**
- [Arc::make_mut documentation](https://doc.rust-lang.org/std/sync/struct.Arc.html#method.make_mut) — copy-on-write semantics

#### Pattern: Snapshot Caching

**How it works:** Cache a single snapshot per scheduler tick and reuse across all internal call sites within that tick. Instead of calling `get_snapshot()` 11+ times, call once and pass the result to helper functions.

**When to use:** When the cost multiplier is call frequency rather than per-clone cost. Reduces total clones from N to 1 per tick.

**Tradeoffs:**
- Pro: Zero dependencies, zero API changes to data structures
- Pro: ~10x reduction in clone count immediately
- Pro: Simplest possible optimization
- Con: Does not reduce per-clone cost
- Con: Snapshot becomes stale within a tick (fine in sequential actor model)

#### Pattern: Hybrid (Arc<Vec<T>> + Snapshot Caching)

**How it works:** Coordinator stores `Arc<Vec<BacklogItem>>`. Each `get_snapshot()` returns `Arc::clone()` (O(1)). Scheduler caches returned Arc for reuse within a tick. Mutations use `Arc::make_mut()`.

**When to use:** When you want both reduced clone frequency and reduced per-clone cost with minimal complexity.

**Tradeoffs:**
- Pro: Zero new dependencies
- Pro: Snapshot distribution O(1) regardless of collection size
- Pro: Internal Vec APIs preserved
- Con: Slightly more complex than pure caching
- Con: No structural sharing (full deep clone when it does happen)

### Technologies & Tools

| Technology | Status | License | Serde | Deref to slice | Pros | Cons |
|------------|--------|---------|-------|----------------|------|------|
| [imbl v2.0.2](https://github.com/jneem/imbl) | Active (maintained fork) | MPL-2.0 | Yes (feature flag) | No | Maintained, O(1) clone, structural sharing | Min Rust 1.85+, transitive deps, no slice deref |
| [im v15.1.0](https://docs.rs/im/latest/im/) | **Unmaintained** (last release Apr 2022) | MPL-2.0 | Yes (feature flag) | No | Mature API, widely used | Outdated deps, no maintenance |
| [rpds](https://github.com/orium/rpds) | Active | MPL-2.0 | Yes | No | no_std, opt-in thread safety | Less adopted for Vector use case |
| `std::sync::Arc` | Stable (stdlib) | MIT/Apache-2.0 | N/A | N/A (inner type) | Zero deps, well-understood | No structural sharing |

**Critical finding:** The PRD references the `im` crate, which has been **unmaintained since April 2022**. The community-recommended successor is `imbl` (maintained fork by jneem). If a persistent data structure is adopted, `imbl` should be used instead.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Optimizing at wrong abstraction level | Reducing per-clone cost (O(n)→O(1)) less impactful than reducing clone count (11x→1x) when n is small | Measure first; try caching before structural changes |
| im::Vector lacks Deref<Target=[T]> | All functions taking `&[BacklogItem]` need refactoring | Catalog slice usage; consider `impl Iterator<Item=&T>` |
| Constant-factor regression at small scale | im/imbl docs note "Vec beats Vector at small sizes due to CPU cache optimization" | Benchmark at actual working size (5-30 items) |
| Using unmaintained `im` instead of `imbl` | Outdated dependencies, no bug fixes | Use `imbl` v2.0.2+ |
| Arc::make_mut with long-lived consumers | Every mutation triggers deep clone if refcount >1 | Ensure consumers drop promptly (natural in actor model) |
| Serde round-trip edge cases | Custom serde attributes (skip_serializing_if, default) may behave differently | Explicit round-trip test before committing |

### Key Learnings

- At 5-30 items with 15+ heap-allocated fields, a single deep clone likely costs low single-digit microseconds. Even 44 clones per scheduler cycle is well under a millisecond total.
- The crossover point where persistent structures outperform Vec for clone-heavy workloads is typically in the "several hundred elements" range.
- Snapshot caching provides ~10x clone reduction with zero complexity cost and should be evaluated first regardless of other decisions.
- The actor model's sequential processing guarantees that `Arc::make_mut` will typically find refcount=1 (no outstanding snapshots), making the copy-on-write path free.

---

## Internal Research

### Existing Codebase State

Phase Golem implements a single-owner coordinator actor that exclusively owns backlog state and processes commands sequentially via an mpsc channel. Snapshots are full deep clones of `BacklogFile` returned to the scheduler for read-only analysis.

**Relevant files/modules:**

- `src/types.rs:184-237` — `BacklogItem` struct (25 fields, derives Clone + Serialize + Deserialize), `BacklogFile` struct (contains `items: Vec<BacklogItem>`)
- `src/coordinator.rs:349-351` — `handle_get_snapshot()`: `state.backlog.clone()` — the primary clone bottleneck
- `src/scheduler.rs` — Primary snapshot consumer; 11+ `get_snapshot()` calls across main loop and handler functions
- `src/backlog.rs:100-472` — Core mutation APIs: push, remove, retain, iter_mut on `backlog.items`
- `src/filter.rs:237-250` — `apply_filters()`: iterates, filters, clones items into new Vec
- `src/prompt.rs:58` — `build_backlog_summary(items: &[BacklogItem], ...)` — slice-based API
- `src/preflight.rs:313,344` — Validation functions taking `&[BacklogItem]` slices
- `src/main.rs` — CLI operations: iter, iter_mut, len, is_empty on items
- `src/migration.rs` — v1→v3 compatibility: constructs `Vec<BacklogItem>` via collect()

**Existing patterns in use:**
- `Arc<T>` already used for `Arc<AtomicBool>` (signal handling) and `Arc<impl AgentRunner>` (shared runner)
- Clone-heavy patterns: filter copies via `.cloned().collect()`, snapshot via `.clone()`
- Slice-based API: 6+ helper functions consistently use `&[BacklogItem]` for read-only access
- Complete serde infrastructure with conditional serialization attributes

### Call Site Catalog

**Snapshot generation (1 site):**
- `coordinator.rs:350` — `state.backlog.clone()` — **PRIMARY BOTTLENECK**

**Snapshot consumption — get_snapshot() calls (11 sites in scheduler.rs):**
1. `scheduler.rs:607` — Main loop snapshot
2. `scheduler.rs:679-680` — Filter application
3. `scheduler.rs:873` — Execute phase fresh snapshot
4. `scheduler.rs:1103` — `handle_promote()`
5. `scheduler.rs:1156` — `handle_merge()`
6. `scheduler.rs:1247` — `handle_unblock()`
7. `scheduler.rs:1293` — `handle_block()`
8. `scheduler.rs:1322` — `handle_triage()`
9. `scheduler.rs:1364` — `run_phase()`
10. `scheduler.rs:1464` — `execute_phase()`
11. `scheduler.rs:1502` — Additional handler

Note: Not all 11 calls execute per loop iteration. The main loop calls once (line 607), then dispatches to handler functions that each call get_snapshot() independently. In a typical iteration, 2-4 snapshot calls is realistic.

**Functions taking `&[BacklogItem]` slices (6 sites):**
- `scheduler.rs:287` — `sorted_ready_items(items: &[BacklogItem])`
- `scheduler.rs:304` — `sorted_in_progress_items(items: &[BacklogItem], ...)`
- `scheduler.rs:323` — `sorted_scoping_items(items: &[BacklogItem], ...)`
- `scheduler.rs:340` — `sorted_new_items(items: &[BacklogItem])`
- `scheduler.rs:358` — `unmet_dep_summary(..., all_items: &[BacklogItem])`
- `scheduler.rs:382` — `skip_for_unmet_deps(..., all_items: &[BacklogItem])`
- `prompt.rs:58` — `build_backlog_summary(items: &[BacklogItem], ...)`
- `preflight.rs:313` — `validate_duplicate_ids(items: &[BacklogItem])`
- `preflight.rs:344` — `validate_dependency_graph(items: &[BacklogItem])`

**Mutation operations on items Vec (7+ sites in backlog.rs):**
- `backlog.rs:153` — `push(item.clone())`
- `backlog.rs:223` — `remove(item_idx)`
- `backlog.rs:227` — `retain()` on dependencies
- `backlog.rs:267` — `push(item.clone())`
- `backlog.rs:346` — `push(item.clone())`
- `backlog.rs:375` — `retain()` on dependencies
- `backlog.rs:419` — `remove(source_idx)`
- `backlog.rs:440` — `iter_mut().find()`
- `backlog.rs:465` — `retain()` on dependencies

**Total `.items` accesses across codebase: ~103 locations** (mix of read, write, iteration, length checks)

### Constraints from Existing Code

1. **Slice API surface is extensive** — 9+ functions take `&[BacklogItem]` slices. `imbl::Vector` does NOT implement `Deref<Target=[T]>`, so all these call sites would need adaptation.
2. **BacklogItem must remain Clone** — Required by existing mutation patterns, serde, and would be required by `imbl::Vector`.
3. **Single-owner actor model** — No concurrent mutation concern; the coordinator processes commands sequentially.
4. **Serde compatibility** — BacklogItem uses conditional serialization attributes (`skip_serializing_if`, `default`). Round-trip must be verified.
5. **Migration code** — `migration.rs` constructs `Vec<BacklogItem>` via `.collect()` operations. Would need adaptation for persistent vectors.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| References `im` crate as primary candidate | `im` is **unmaintained since April 2022**. `imbl` is the maintained community fork. | If persistent structures are adopted, must use `imbl`, not `im`. PRD's references to `im` crate API still mostly apply since `imbl` is API-compatible. |
| "At least 11 times per loop iteration" | Internal research shows 11 `get_snapshot()` call sites, but they are in different handler functions. A typical loop iteration executes 2-4 snapshot calls, not all 11. | The clone multiplier is lower than assumed. Snapshot caching would reduce from ~3-4 to 1 per typical tick, not 11 to 1. |
| "22-44+ deep clones per scheduler cycle" with concurrency | Concurrent executor tasks each get independent snapshots, but each executor runs its own handler which calls get_snapshot() once per execution. Total is closer to 3-5 per tick + 1 per concurrent execution. | Total clone count lower than PRD estimates, weakening the case for persistent structures. |
| Estimates 6-8 affected files | Internal research found 9+ files with `.items` accesses: types.rs, coordinator.rs, scheduler.rs, backlog.rs, filter.rs, prompt.rs, preflight.rs, main.rs, migration.rs | Migration scope slightly larger than estimated. The slice API surface (9+ functions taking `&[BacklogItem]`) is the main cost. |
| Focuses on data structure swap | Simpler alternatives (snapshot caching, Arc<Vec>) may achieve 80-90% of the benefit with 10% of the complexity | Design should evaluate staged approach: caching first, Arc second, persistent structures only if needed. |

---

## Critical Areas

### im::Vector Lacks Deref<Target=[T]>

**Why it's critical:** 9+ functions across the codebase accept `&[BacklogItem]` slices. This is the idiomatic Rust pattern for read-only collection access. `imbl::Vector` cannot produce a contiguous slice because its data is stored in a tree structure (non-contiguous memory).

**Why it's easy to miss:** The PRD acknowledges this but lists it as a risk rather than the primary migration cost. The actual migration work is dominated by adapting these slice-based APIs.

**What to watch for:** Each slice-consuming function would need to either: (a) accept `impl Iterator<Item=&BacklogItem>` instead of `&[BacklogItem]`, (b) collect into a temporary Vec for slice access, or (c) be rewritten to use `imbl::Vector`-specific iteration APIs. Options (a) and (b) have their own tradeoffs — (a) changes function signatures broadly, (b) negates the clone savings by materializing temporary Vecs.

### Clone Cost at Current Scale May Be Negligible

**Why it's critical:** If the optimization provides no measurable benefit, the added complexity has purely negative value.

**Why it's easy to miss:** The PRD frames the investigation around projected scale (100-500 items) but acknowledges current scale (5-30 items) may not benefit. Without benchmarks, it's tempting to optimize speculatively.

**What to watch for:** At 30 items with 15+ heap fields, a single clone likely costs low single-digit microseconds. Even 5 clones per tick at 3-4 ticks/second is well under 100μs total — negligible in a system doing network I/O and LLM API calls.

---

## Deep Dives

### im vs imbl: Maintenance Status

**Question:** Is the `im` crate still viable for production use?

**Summary:** The `im` crate (v15.1.0) has not been updated since April 2022 and is effectively unmaintained. The `imbl` crate is an actively maintained community fork by jneem, API-compatible with im v15. `imbl` v2.0.2 requires Rust 1.85+ and includes bug fixes and performance improvements. Historical soundness issue in `sized-chunks` (im dependency) was fixed in v0.6.4.

**Implications:** Any recommendation involving persistent data structures should specify `imbl`, not `im`. The PRD's references to `im` crate API and behavior still apply since `imbl` maintains API compatibility.

### Actual Snapshot Call Frequency

**Question:** How many times does the scheduler actually call get_snapshot() per loop iteration?

**Summary:** The codebase has 11 `get_snapshot()` call sites in scheduler.rs, but they are distributed across independent handler functions (handle_promote, handle_merge, handle_triage, etc.). In a typical scheduler loop iteration:
1. Main loop calls `get_snapshot()` once (line 607)
2. The selected action handler (e.g., handle_triage) calls it once more (lines 1103-1502)
3. If execution happens, `execute_phase()` calls it once (line 1464)
4. Total per typical tick: **2-4 calls**, not 11

The 11-call scenario would require every handler to execute in a single tick, which doesn't happen in practice.

**Implications:** The clone multiplier is lower than the PRD's estimate of 22-44+. The actual overhead per tick is roughly 2-4 clones × clone cost. This further weakens the case for persistent structures at current scale.

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| What is the realistic long-term backlog size ceiling? | If 30 items is the practical max, optimization provides no value. | Current: 5-30; if >100 ever expected, caching is sufficient. |
| Are there plans for higher-frequency scheduler loops? | More ticks/second = more clones/second. | Current architecture doesn't suggest this is planned. |
| Is MPL-2.0 license acceptable for imbl? | Affects whether persistent structures are even an option. | Likely fine for an internal tool, but should be confirmed. |

### Recommended Approaches

#### Snapshot Clone Reduction Strategy

| Approach | Clone Cost | API Disruption | Dependencies | Best When |
|----------|-----------|----------------|-------------|-----------|
| **Snapshot caching** | Reduces count from ~3-4 to 1/tick; per-clone still O(n) | None | None | Current scale (5-30 items). Should be done regardless. |
| **Arc<Vec<T>> + caching** | 1/tick; O(1) if refcount=1; O(n) deep clone if >1 | Minimal (mutation sites need Arc::make_mut) | None (stdlib) | Moderate scale (30-200 items). Zero-dep solution. |
| **imbl::Vector** | O(1) clone always | **High** — 9+ functions need slice API adaptation | imbl + transitive deps (sized-chunks, bitmaps, etc.) | Large scale (500+ items) with frequent snapshots during outstanding mutations |
| **Do nothing** | Current O(n) × 3-4/tick | None | None | If benchmarks confirm cost is negligible at projected scale |

**Initial recommendation: Snapshot caching (with optional Arc<Vec> follow-up)**

**Rationale:**
1. **Snapshot caching** should be the first step regardless of other decisions. It reduces clone count by ~3-4x with zero API changes, zero dependencies, and minimal code change (scheduler caches one snapshot per tick). This is a pure win.
2. **Arc<Vec<T>>** can be layered on later if measurement shows the remaining 1-clone-per-tick is meaningful at projected scale. It uses only stdlib types, preserves all Vec/slice APIs, and the actor model's sequential processing means `Arc::make_mut` will almost always find refcount=1 (zero-cost mutation path).
3. **imbl::Vector should NOT be adopted** at current or near-term projected scale. The API migration cost (9+ functions taking slices, 7+ mutation sites, migration code) is disproportionate to the benefit for collections of 5-30 items. The constant-factor overhead at small scale could actually regress performance. The crossover point where persistent structures outperform Vec is in the hundreds of items — a scale this project is unlikely to reach.
4. **"Do nothing" is a valid outcome** if benchmarks confirm clone cost is negligible (likely <100μs/tick total at 30 items).

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [imbl GitHub](https://github.com/jneem/imbl) | Library (maintained fork) | If persistent structures ever needed, this is the right crate |
| [Arc::make_mut docs](https://doc.rust-lang.org/std/sync/struct.Arc.html#method.make_mut) | Stdlib docs | Copy-on-write semantics for Arc<Vec> approach |
| [im::Vector API](https://docs.rs/im/latest/im/vector/struct.Vector.html) | API docs | Performance characteristics, API differences from Vec |
| [Rust Performance Book: Heap Allocations](https://nnethercote.github.io/perf-book/heap-allocations.html) | Guide | General allocation optimization guidance |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Parallel external + internal research (medium mode) | Comprehensive findings on 4 patterns, 103 call sites cataloged, im crate maintenance issue identified |
| 2026-02-20 | PRD analysis and synthesis | PRD clone count estimate higher than actual (~3-4/tick not 11); im→imbl switch needed; snapshot caching identified as best first step |
| 2026-02-20 | Research complete | Recommendation: snapshot caching first, Arc<Vec> if needed, imbl only at 500+ items |
