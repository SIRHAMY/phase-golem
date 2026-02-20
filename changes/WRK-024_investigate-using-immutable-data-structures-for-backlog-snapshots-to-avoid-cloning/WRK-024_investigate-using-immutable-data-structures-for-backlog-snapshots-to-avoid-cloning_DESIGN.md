# Design: Snapshot Caching for Backlog Snapshot Optimization

**ID:** WRK-024
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-024_investigate-using-immutable-data-structures-for-backlog-snapshots-to-avoid-cloning_PRD.md
**Tech Research:** ./WRK-024_investigate-using-immutable-data-structures-for-backlog-snapshots-to-avoid-cloning_TECH_RESEARCH.md
**Mode:** Medium

## Overview

The recommended design is **snapshot caching in the scheduler**: fetch one `BacklogFile` snapshot at the top of each scheduler tick and pass it by reference to all handler functions within that tick. This eliminates 2-3 redundant `get_snapshot()` calls per tick with zero new dependencies, zero API changes to data structures, and minimal code modification. The design preserves all existing `&[BacklogItem]` slice APIs and Vec-based patterns unchanged. Persistent data structures (`imbl::Vector`) are explicitly deferred as not cost-justified at current or projected scale (5-30 items, up to ~100 with growth).

---

## System Design

### High-Level Architecture

The current architecture has a coordinator actor that exclusively owns backlog state and a scheduler that requests snapshots via `coordinator.get_snapshot()`. Each snapshot is a full deep clone of `BacklogFile` (containing `Vec<BacklogItem>` with 22 fields per item, ~15 heap-allocated).

**Current flow (per tick):**
```
Scheduler tick
  -> get_snapshot() [clone #1]
  -> dispatch to handler (e.g., handle_triage)
    -> get_snapshot() [clone #2]
    -> execute_phase()
      -> get_snapshot() [clone #3]
```

**Proposed flow (per tick):**
```
Scheduler tick
  -> get_snapshot() [clone #1, cached for tick]
  -> dispatch to handler (passes &snapshot)
    -> uses cached &snapshot
    -> execute_phase(passes &snapshot)
      -> uses cached &snapshot
  -> [optional] re-snapshot after mutations
```

The change is entirely within `scheduler.rs`. The coordinator, types, backlog, and all other modules remain unchanged.

### Component Breakdown

#### Scheduler Main Loop (modified)

**Purpose:** Orchestrates one tick of scheduler work — fetch snapshot, select action, dispatch handler, manage executor tasks.

**Responsibilities:**
- Fetch a single snapshot at tick start
- Pass snapshot by reference to handler functions
- Re-fetch snapshot only when a mutation has occurred and a subsequent operation needs fresh data

**Interfaces:**
- Input: `CoordinatorHandle` (for `get_snapshot()` and mutation commands)
- Output: Dispatched phase executions, worklog entries

**Dependencies:** `CoordinatorHandle`, `AgentRunner`

#### Handler Functions (modified signatures)

**Purpose:** Process specific scheduler actions (triage, promote, merge, block, unblock, etc.)

**Responsibilities:**
- Receive snapshot by reference instead of calling `get_snapshot()` internally
- Perform read-only operations against the provided snapshot
- Return control to main loop (which may re-snapshot if mutations occurred)

**Interfaces:**
- Input: `&BacklogFile` (snapshot reference) instead of `CoordinatorHandle` for reads
- Output: Action results, mutation requests via `CoordinatorHandle`

**Dependencies:** Still depends on `CoordinatorHandle` for mutation operations (update_item, add_item, etc.)

### Data Flow

1. **Tick starts** — Scheduler calls `coordinator.get_snapshot().await` once, storing the result as `tick_snapshot: BacklogFile`
2. **Action selection** — Main loop uses `&tick_snapshot` for filtering, sorting, and selecting the next action
3. **Handler dispatch** — Selected handler receives `&tick_snapshot` instead of calling `get_snapshot()` independently
4. **Handler execution** — Handler reads from `&tick_snapshot`, issues mutations through `CoordinatorHandle` as needed
5. **Post-mutation refresh** — If a handler mutates the backlog and subsequent logic in the same tick needs fresh data, a targeted re-snapshot is performed. This is the exception, not the rule.
6. **Tick ends** — `tick_snapshot` is dropped, memory freed

### Key Flows

#### Flow: Standard Scheduler Tick (Happy Path)

> Scheduler processes one action per tick using a single cached snapshot.

1. **Fetch snapshot** — `let tick_snapshot = coordinator.get_snapshot().await?;`
2. **Apply filters** — `apply_filters(&tick_snapshot.items, ...)` using cached snapshot
3. **Select action** — Evaluate ready/in-progress/new items from cached snapshot
4. **Dispatch handler** — e.g., `handle_triage(&tick_snapshot, coordinator, runner, ...)`
5. **Handler reads** — Handler uses `&tick_snapshot.items` for sorting, validation, worklog
6. **Handler mutates** — Handler calls `coordinator.update_item(...)` for state changes
7. **Tick completes** — Snapshot dropped, next tick will fetch fresh

**Edge cases:**
- **Handler needs post-mutation data** — If a handler mutates the backlog (via `coordinator.update_item()` etc.) and then needs to read the updated state within the same handler call, it calls `coordinator.get_snapshot()` explicitly at that point. Based on the call site catalog below, most handlers follow a read-then-mutate-then-return pattern and do not need post-mutation reads. The SPEC phase must audit each handler to confirm this. Any handler that does mutate-then-read should be explicitly documented with a comment explaining why a fresh snapshot is needed.
- **Concurrent executor completion** — Executor tasks that complete during a tick have their own snapshot from when they were spawned. The main loop's cached snapshot may be stale relative to executor-triggered mutations, but the next tick will fetch fresh data. This bounded staleness (one tick window) is acceptable because the scheduler's sequential processing means no decision within a single tick depends on another concurrent executor's mutations.

#### Flow: Executor Task Spawning

> When spawning a phase executor, the executor gets its own snapshot for its lifetime.

1. **Main loop identifies ready work** — Using cached `tick_snapshot`
2. **Spawn executor** — Calls `spawn_phase_task(coordinator.clone(), runner.clone(), ...)`
3. **Executor fetches own snapshot** — Inside the spawned task, `coordinator.get_snapshot()` is called independently. This is correct because the executor runs concurrently and needs an independent view.
4. **Executor runs** — Uses its own snapshot for the duration of the phase

**Edge case:** The executor's snapshot call is intentionally NOT cached from the main loop. Executors run concurrently and may need data that was mutated after the tick's snapshot was taken.

---

## Technical Decisions

### Key Decisions

#### Decision: Cache at the scheduler tick level, not in the coordinator

**Context:** Snapshot caching could be implemented either in the coordinator (cache the last snapshot and return clones of it) or in the scheduler (fetch once per tick, pass by reference).

**Decision:** Cache in the scheduler by passing `&BacklogFile` to handler functions.

**Rationale:**
- The coordinator's job is to provide fresh, consistent snapshots. Adding caching there couples caching policy to the data owner.
- The scheduler knows its own access pattern (multiple reads within one tick). It's the right place to optimize that pattern.
- Passing by reference avoids even the cost of cloning the cached value — handlers borrow the tick's snapshot at zero cost.
- No changes needed to `CoordinatorHandle` API or coordinator internals.

**Consequences:** Handler function signatures change to accept `&BacklogFile`. This is a local refactor within `scheduler.rs`.

#### Decision: Do not adopt imbl::Vector or Arc<Vec> at this time

**Context:** Tech research identified three approaches: snapshot caching, Arc<Vec<T>>, and imbl::Vector. The PRD framed this as an investigation item.

**Decision:** Recommend snapshot caching only. Defer Arc<Vec> and imbl::Vector.

**Rationale:**
- At 5-30 items, a single BacklogFile clone costs low single-digit microseconds. Even 4 clones per tick is well under 100us — negligible in a system doing network I/O and LLM API calls.
- Snapshot caching reduces clone count from ~3-4 to 1 per tick, which is a 3-4x improvement with zero complexity.
- Arc<Vec> adds complexity at mutation sites (`Arc::make_mut`) for marginal benefit when clone count is already 1/tick.
- imbl::Vector requires adapting 9+ functions that take `&[BacklogItem]` slices, which is a significant API migration cost for zero measurable benefit at current scale.
- The crossover point where persistent structures outperform Vec is in the hundreds of items — a scale this project is unlikely to reach.

**Consequences:** If the backlog grows beyond ~200 items and benchmarks show clone cost becoming meaningful, Arc<Vec> can be layered on as a follow-up. The snapshot caching design does not preclude this.

#### Decision: Executor tasks fetch their own snapshots independently

**Context:** Executor tasks (`spawn_phase_task`) run concurrently with the main scheduler loop. Should they use the cached tick snapshot?

**Decision:** No. Executor tasks continue to call `coordinator.get_snapshot()` independently.

**Rationale:**
- Executors run on their own async task and may outlive the tick that spawned them.
- They need a snapshot reflecting any mutations that occurred between tick start and executor spawn.
- The lifetime of `&BacklogFile` from the main loop cannot be shared into a spawned task without ownership transfer (which would defeat the purpose).

**Consequences:** The executor's `get_snapshot()` calls remain as-is. This is 1 clone per executor spawn, which is appropriate.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Stale data within a tick | Handler reads may be slightly stale if mutations occurred earlier in the same tick | Zero-cost snapshot sharing within a tick via borrowing | The coordinator processes commands sequentially. Within a single tick, the scheduler issues at most one mutation before reading. The staleness window is negligible (microseconds), and correctness is maintained because the next tick fetches fresh data. |
| No per-clone cost reduction | Each clone is still O(n) — we're only reducing clone count, not cost | Simplicity — no new types, no API migration, no dependencies | At 30 items, one O(n) clone costs ~2-5us. Reducing from 4 to 1 clone saves ~6-15us per tick. This is the right optimization granularity for current scale. |
| Not future-proofing for 500+ items | If backlog ever reaches hundreds of items, a per-clone optimization would be needed | Staged optimization — address frequency first, then per-clone cost only if measured | This is a staged strategy: (1) snapshot caching addresses clone *frequency* now with zero complexity, (2) if benchmarks show the remaining 1-clone-per-tick becomes a bottleneck at >100 items, `Arc<Vec>` is a straightforward addition. **Re-evaluation trigger:** if backlog size exceeds 100 items or executor spawn frequency exceeds 8/tick, revisit with benchmarks. |

---

## Alternatives Considered

### Alternative: Arc<Vec<BacklogItem>> with Copy-on-Write

**Summary:** Wrap `BacklogFile.items` (or the entire `BacklogFile`) in `Arc`. Snapshot distribution uses `Arc::clone()` (O(1)). Mutations use `Arc::make_mut()` for copy-on-write.

**How it would work:**
- `BacklogFile` stores `items: Arc<Vec<BacklogItem>>` instead of `items: Vec<BacklogItem>`
- `handle_get_snapshot()` returns `Arc::clone()` — O(1) atomic refcount increment
- Mutation functions use `Arc::make_mut(&mut self.items)` which gives `&mut Vec<BacklogItem>` directly if refcount is 1

**Pros:**
- Snapshot distribution is O(1) regardless of collection size
- Zero new dependencies (stdlib only)
- Inner `Vec<T>` retains all standard APIs — `&[T]` slices, indexing, contiguous iteration
- `Arc::make_mut` is zero-cost when no outstanding snapshots (typical in sequential actor model)

**Cons:**
- Adds complexity at every mutation site in `backlog.rs` (7+ sites need `Arc::make_mut`)
- Changes `BacklogFile` struct definition, affecting serialization considerations
- Marginal benefit when combined with snapshot caching (which already reduces clones to 1/tick)
- At refcount=1 (the common case), `Arc::make_mut` is free but adds cognitive overhead

**Why not chosen:** With snapshot caching reducing clones to 1/tick, the remaining single clone is ~2-5us at 30 items. The complexity of Arc wrapping at 7+ mutation sites is not justified by saving microseconds. This remains a viable follow-up if scale increases significantly.

### Alternative: imbl::Vector Persistent Data Structure

**Summary:** Replace `Vec<BacklogItem>` with `imbl::Vector<BacklogItem>`, a persistent data structure using structural sharing. Cloning is O(1) via reference-counted tree nodes.

**How it would work:**
- `BacklogFile` stores `items: imbl::Vector<BacklogItem>`
- All clone operations become O(1) refcount increments
- Mutations create new tree nodes for modified paths, sharing unmodified nodes

**Pros:**
- Clone is O(1) regardless of collection size — truly eliminates clone cost
- Structural sharing means mutations only copy the modified subtree
- Serde-compatible via feature flag

**Cons:**
- **9+ functions taking `&[BacklogItem]` need adaptation** — `imbl::Vector` has no `Deref<Target=[T]>` because data is non-contiguous in memory
- Indexed access is O(log n) vs O(1) for Vec
- Iteration involves pointer chasing (cache-unfriendly)
- Higher constant factors at small scale (<100 items) — im/imbl docs note "Vec beats Vector at small sizes"
- Adds `imbl` + transitive dependencies (`sized-chunks`, `bitmaps`, etc.)
- `im` crate is unmaintained; must use `imbl` fork
- Migration touches 9+ files, 103 `.items` access sites

**Why not chosen:** The API migration cost is disproportionate to the benefit. At 5-30 items, persistent structures are actually slower than Vec for iteration and mutation due to constant-factor overhead. The 9+ functions taking `&[BacklogItem]` slices would each need adaptation (either to iterators or to temporary Vec materialization, which negates the clone savings). This is a net-negative trade at current and projected scale.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Stale snapshot causes incorrect handler behavior | Handler acts on outdated data after a mutation within the same tick | Low | The coordinator processes commands sequentially. Handlers that both mutate and read within the same call can explicitly re-fetch. Document the pattern clearly. |
| Refactoring handler signatures introduces bugs | Passing `&BacklogFile` instead of fetching internally could miss a case where fresh data is needed | Low | Each handler call site is audited during implementation. The current 11 `get_snapshot()` calls are well-cataloged. Handlers that mutate-then-read are identified and handled explicitly. |
| Optimization deemed unnecessary | At 5-30 items, the savings (~6-15us/tick) may not be worth the refactoring effort | Medium | The refactoring is small (signature changes within one file) and improves code clarity by making the snapshot lifecycle explicit. Even if performance savings are negligible, the design is cleaner. |
| Future handler incorrectly uses stale data | A new handler added later could mutate-then-read without re-fetching | Low | The SPEC must produce a per-handler classification (read-only / mutate-then-return / mutate-then-read). Code review should check new handlers against this pattern. Comments at the handler function level should document the snapshot freshness contract. |

---

## Integration Points

### Existing Code Touchpoints

- `src/scheduler.rs` — **Primary change site.** Handler function signatures modified to accept `&BacklogFile`. Main loop caches snapshot. See call site catalog below.
- `src/coordinator.rs` — **No changes.** `handle_get_snapshot()` and `CoordinatorHandle` API remain unchanged.
- `src/types.rs` — **No changes.** `BacklogFile` and `BacklogItem` structs unchanged.
- `src/backlog.rs` — **No changes.** Mutation APIs unchanged.
- `src/filter.rs` — **No changes.** `apply_filters()` takes `&BacklogFile`, which can be passed directly from the cached snapshot.
- `src/prompt.rs` — **No changes.** `build_backlog_summary()` takes `&[BacklogItem]`, provided via `&tick_snapshot.items`.
- `src/preflight.rs` — **No changes.** Validation functions take `&[BacklogItem]`, provided via `&tick_snapshot.items`.

#### get_snapshot() Call Site Catalog (scheduler.rs)

The following call sites currently call `coordinator.get_snapshot()` and need evaluation during SPEC. Not all execute per tick — the scheduler dispatches to one handler per tick.

| Line | Context | Can Use Cached? | Notes |
|------|---------|-----------------|-------|
| 607 | Main loop top | **Source** — this becomes the cached snapshot | Fetched once, stored as `tick_snapshot` |
| 679-680 | Filter application | Yes | Reads only, uses cached snapshot |
| 873 | Execute phase (executor spawn) | **No** — executor runs concurrently | Executor needs independent snapshot |
| 1103 | `handle_promote()` | Yes | Reads item state for promotion |
| 1156 | `handle_merge()` | Yes | Reads for merge validation |
| 1247 | `handle_unblock()` | Yes | Reads blocked item state |
| 1293 | `handle_block()` | Yes | Reads item state |
| 1322 | `handle_triage()` | Yes | Reads for triage dispatch |
| 1364 | `run_phase()` | Yes | Reads for phase execution |
| 1464 | `execute_phase()` | Yes | Reads for worklog |
| 1502 | Additional handler | Yes | Reads item state |
| 1585 | Triage executor spawn | **No** — executor runs concurrently | Executor needs independent snapshot |
| 1676 | Post-triage completion | **Needs audit** — may need fresh data after triage mutations |

**Summary:** Of ~13 call sites, 1 is the cache source, 2 are in executor spawns (must remain independent), ~9 can use the cached snapshot, and 1 needs SPEC-phase audit to determine if it requires a fresh post-mutation snapshot.

### External Dependencies

- None. This design uses only existing types and patterns.

---

## Open Questions

- [x] Which handler functions need explicit re-snapshot after mutations? — Identified during design: handlers that call `coordinator.update_item()` and then need to read updated state should re-fetch. Most handlers follow read-then-mutate-then-return pattern. The SPEC phase must audit each handler (see call site catalog) and classify as: (A) read-only, (B) mutate-then-return, or (C) mutate-then-read. Only type (C) requires explicit re-snapshot.
- [x] Is the PRD's serde gate requirement applicable? — No. The serde gate was a prerequisite for evaluating `im::Vector`/`imbl::Vector` round-trip compatibility. Since this design recommends snapshot caching (no data structure changes), serde verification is not needed. The `BacklogFile` and `Vec<BacklogItem>` types remain unchanged.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements — Provides clear recommendation (snapshot caching), evaluates alternatives (Arc<Vec>, imbl::Vector), catalogs affected call sites, and includes migration scope assessment
- [x] Key flows are documented and make sense — Standard tick flow and executor spawning flow documented with edge cases
- [x] Tradeoffs are explicitly documented and acceptable — Stale data, no per-clone reduction, and no future-proofing tradeoffs documented with rationale
- [x] Integration points with existing code are identified — Only scheduler.rs is modified; all other modules unchanged
- [x] No major open questions remain — Handler re-snapshot question resolved during design

---

## Assumptions

*Decisions made without human input during autonomous design:*

- **Mode selection:** Used `medium` mode based on the item's assessments (small size, low complexity). Medium mode provides 1-2 alternatives with comparison, which is appropriate for evaluating the 3 approaches identified in tech research.
- **Recommended approach:** Selected snapshot caching over Arc<Vec> and imbl::Vector based on tech research findings that clone cost at current scale (5-30 items) is negligible and the primary multiplier is call frequency, not per-call cost.
- **Scope of change:** Confirmed the design is limited to `scheduler.rs` changes only. The investigation PRD asked for a recommendation, and this design provides both the recommendation and a concrete implementation approach.
- **Executor independence:** Decided executor tasks should continue fetching their own snapshots rather than receiving cached data, based on lifetime and concurrency constraints.
- **Self-critique directional decisions:** During self-critique, agents raised concerns about handler signature specificity and re-snapshot enforcement mechanisms. Decided these belong in the SPEC phase rather than the design: the design establishes the approach (pass `&BacklogFile` by reference), and the SPEC will define exact signatures and per-handler audit results. Added call site catalog and handler classification guidance to bridge the gap.

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Initial design draft | Recommended snapshot caching; documented Arc<Vec> and imbl::Vector as alternatives not chosen; identified scheduler.rs as sole change target |
| 2026-02-20 | Self-critique (7 agents) | Applied auto-fixes: added call site catalog (13 sites), fixed filter.rs description, added serde gate dismissal, reframed YAGNI as staged strategy with re-evaluation triggers, strengthened mutation-then-read edge case documentation, added stale-data risk mitigation. Deferred to SPEC: exact handler signatures, per-handler mutate-then-read audit, benchmark baseline. |
