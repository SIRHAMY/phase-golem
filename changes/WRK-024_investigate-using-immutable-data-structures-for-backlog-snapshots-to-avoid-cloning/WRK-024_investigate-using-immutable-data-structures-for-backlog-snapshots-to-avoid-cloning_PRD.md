# Change: Persistent Data Structures for Backlog Snapshots

**Status:** Proposed
**Created:** 2026-02-20
**Author:** Phase Golem (autonomous)

## Problem Statement

The coordinator actor in `coordinator.rs` creates backlog snapshots by deep-cloning the entire `BacklogFile` (including its `Vec<BacklogItem>`) on every call to `handle_get_snapshot()`. Each `BacklogItem` has 23 fields with 15+ heap-allocated values (Strings, Vecs, Options containing Strings). The scheduler calls `get_snapshot()` at least 11 times per loop iteration, and with 2-4 concurrent executor tasks this becomes 22-44+ deep clones per scheduler cycle.

At current backlog sizes (5-30 items), preliminary analysis suggests the clone cost is low (estimated microseconds per clone), but this has not been measured. The design creates a structural inefficiency that scales linearly with both backlog size and concurrency. At projected scales beyond 100 items, the per-iteration allocation overhead could become measurable.

**Key clarification:** The `im` crate provides *persistent* data structures — structures that preserve previous versions when modified through structural sharing (reference-counted tree nodes). Cloning an `im::Vector` is O(1) because it increments a reference count on the shared tree root rather than deep-copying every element. Individual `BacklogItem` values within the tree are shared by reference, not individually cloned.

**Concurrency note:** Concurrent mutation during snapshot reads is not a concern in the current architecture. The coordinator actor owns the canonical backlog state exclusively and processes commands sequentially via an mpsc channel. Snapshot consumers receive an independent (cheap) clone and cannot observe in-flight mutations.

This is an **investigation** item. The primary deliverable is a recommendation on whether to adopt persistent data structures, not an implementation. The investigation may conclude that the optimization is not worth the added complexity at current scale.

## User Stories / Personas

- **Maintainer/Developer** - Wants confidence that the system's core data-sharing pattern (coordinator-to-scheduler snapshots) scales without growing per-iteration allocation overhead proportional to backlog size. Wants the cheap-clone semantics to be explicit in the type system rather than relying on deep cloning.

## Desired Outcome

A clear recommendation on whether and how to replace `Vec<BacklogItem>` with a persistent data structure (e.g., `im::Vector<BacklogItem>`) for the backlog snapshot path. The recommendation document should include:

1. A validated approach (or a justified "do nothing" decision) with rationale
2. Quantified or estimated impact at current scale (5-30 items) and projected scales (100, 500 items)
3. Catalog of affected call sites and API migration scope
4. Comparison of alternatives with pros/cons (persistent data structures vs. snapshot caching vs. `Arc<Vec>`)
5. Risk assessment and any blockers identified

**Why `im::Vector` as the primary candidate:** The `im` crate is the most widely adopted persistent data structure library in Rust, with mature serde support, a stable v15 API, and broad ecosystem adoption. It is the natural first candidate for this class of problem. The investigation should also evaluate simpler alternatives.

## Success Criteria

### Must Have

- [ ] **Serde gate:** Verify `im::Vector<BacklogItem>` round-trip compatibility with existing YAML backlog format as a prerequisite. If serde round-trip fails, document the failure and evaluate whether it is fixable or a blocker.
- [ ] Catalog all call sites that would need adaptation: map every usage of `Vec<BacklogItem>` across the codebase, including slice operations (`as_slice()`, `&[BacklogItem]`), construction patterns (`vec![]`), and mutation APIs (`push`, `retain`, index assignment)
- [ ] Evaluate simpler alternatives: (a) snapshot caching in the scheduler — cache one snapshot per tick and reuse across the 11+ call sites, reducing clones from 11+ to 1 per tick with zero new dependencies; (b) `Arc<Vec<BacklogItem>>` with copy-on-write via `Arc::make_mut` at mutation points
- [ ] Produce a written recommendation document containing: executive summary, technical analysis of each approach, comparison matrix, risk assessment, and clear recommendation with rationale
- [ ] If recommending adoption: include a high-level migration outline (affected files, refactoring patterns, estimated size as small/medium/large). Detailed implementation planning belongs in a follow-up design/spec phase.

### Should Have

- [ ] Benchmark clone cost at current scale (30 items), near-term projected growth (100 items), and estimated ceiling (500 items). Establish a baseline with `Vec` first, then measure `im::Vector` to quantify actual savings.
- [ ] Verify `im::Vector` iteration and mutation performance characteristics vs `Vec` at small scales (5-30 items) to check for constant-factor regression
- [ ] Audit `im` crate's transitive dependency tree (e.g., `sized-chunks`, `rand_core`) for size, license compatibility, and maintenance status

### Nice to Have

- [ ] Prototype the type change on a branch to discover compile errors empirically rather than by code inspection
- [ ] Measure scheduler loop timing before/after to establish a real-world performance baseline

## Scope

### In Scope

- Investigating `im::Vector<BacklogItem>` as a replacement for `Vec<BacklogItem>` in `BacklogFile`
- Evaluating alternative approaches: snapshot caching in the scheduler, `Arc<Vec>` with copy-on-write, reducing snapshot call frequency
- Cataloging the API migration surface across all affected files (estimated 6-8 files based on preliminary analysis)
- Producing a recommendation document with supporting evidence
- Verifying serde compatibility (YAML serialization round-trip with `im::Vector`)

### Out of Scope

- Making individual `BacklogItem` fields use persistent data structures (e.g., `im::Vector<String>` for tags)
- Changing the coordinator's actor model or concurrency architecture (no `RwLock`, no shared mutable state)
- Optimizing other clone paths unrelated to snapshot creation
- Implementing the full migration (that would be a separate work item if recommended)
- Detailed implementation spec or design (belongs in follow-up phases)

## Non-Functional Requirements

- **Performance:** Any adopted solution must not measurably regress mutation or iteration performance at current scale (5-30 items). "Measurably regress" means >10% slower in benchmarks for common operations (iteration, push, individual item access). Clone performance improvement is the goal but must not come at the expense of write-path performance.
- **Compatibility:** Existing serialized YAML backlog files must deserialize correctly with any new data structure. Deserialization of YAML sequences produced by `Vec<BacklogItem>` into `im::Vector<BacklogItem>` (and vice versa) must produce identical logical data. No migration step should be required for persisted data.

## Constraints

- Must not change the coordinator's actor model (single-owner state with message passing)
- Must not introduce shared mutable state (`RwLock`, `Mutex` on backlog data)
- Any new crate dependency should be well-maintained with a stable API (the `im` crate v15 meets this bar)
- The `BacklogItem` type must remain `Clone` (required by `im::Vector` for its structural sharing mechanism, and used in existing code paths beyond snapshots)

## Dependencies

- **Depends On:** None — this is a self-contained investigation
- **Blocks:** A potential follow-up implementation work item (if recommendation is to proceed). If recommendation is favorable, a new work item should be created for the migration.

## Risks

- [ ] **Premature optimization:** Preliminary analysis suggests clone cost is low at current scale. The investigation may conclude the optimization has negative net value (complexity cost exceeds performance benefit). Mitigation: the investigation itself is low-cost and provides clarity either way.
- [ ] **Slice API surface larger than expected:** `im::Vector` does not implement `Deref<Target=[T]>`, so any code passing `&[BacklogItem]` or using slice methods needs adaptation. The actual scope of affected call sites may exceed the estimated 6-8 files. Mitigation: empirical catalog of call sites is a must-have deliverable.
- [ ] **Serde compatibility gap:** While `im::Vector` supports serde via a feature flag, subtle differences in serialization behavior could affect YAML round-trips. Mitigation: serde verification is a prerequisite gate — test early, fail fast.
- [ ] **Small-scale performance regression:** `im::Vector` has higher constant factors than `Vec` for mutation and iteration due to tree structure and pointer indirection. At 5-30 items, the net effect could be negative (slower mutations without meaningful clone savings). Mitigation: benchmarking at current scale is a should-have deliverable.

## Open Questions

- [ ] What is the realistic long-term backlog size ceiling? If 30 items is the practical maximum, the optimization may never provide value. A threshold should be defined (e.g., "worth implementing above N items").
- [ ] Could reducing snapshot frequency (e.g., caching one snapshot per scheduler tick) achieve most of the benefit with zero new dependencies? This is potentially a simpler and higher-impact change.
- [ ] Does the `im` crate's transitive dependency tree conflict with project dependency policies? (No formal dependency policy has been identified for this project.)
- [ ] Should a type alias (e.g., `type BacklogItems = im::Vector<BacklogItem>`) be used to insulate the codebase from the concrete `im` type?
- [ ] What level of deliverable detail is expected? This PRD scopes the investigation to produce a recommendation with a high-level migration outline. If pre-design work (API sketches, prototype branch) is expected as part of this investigation, the scope should be expanded.

## Assumptions

*Decisions made without human input during autonomous PRD creation:*

- **Mode selection:** Used `medium` mode (moderate exploration with 3 discovery agents) based on the item's "small" size and "low" complexity assessments.
- **Investigation framing:** Framed this as an investigation/recommendation item rather than a direct implementation PRD, since the work item title starts with "Investigate" and the existing ideas doc acknowledges the optimization may not be worthwhile.
- **Scope decision:** Kept implementation out of scope for this item. If the recommendation is to proceed, a separate work item should be created for the actual migration.
- **Alternative evaluation:** Included snapshot caching and `Arc<Vec>` as alternatives to evaluate, since the discovery agents identified these as potentially simpler solutions that could achieve similar benefits.
- **Terminology:** Used "persistent data structures" (the technically accurate term for structures with structural sharing) rather than "immutable data structures" to avoid confusion. `im::Vector` is mutable — it supports in-place mutations — but preserves old versions cheaply through structural sharing.

## References

- Existing ideas document: `_ideas/WRK-024_immutable-backlog-snapshots.md`
- `im` crate documentation: https://docs.rs/im/latest/im/
- Coordinator snapshot path: `src/coordinator.rs:349-351` (`handle_get_snapshot`)
- BacklogItem definition: `src/types.rs:184-225`
- Scheduler snapshot usage: `src/scheduler.rs` (11+ call sites)
