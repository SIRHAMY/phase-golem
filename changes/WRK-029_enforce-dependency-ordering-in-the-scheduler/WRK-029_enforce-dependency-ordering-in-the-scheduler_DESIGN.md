# Design: Enforce Dependency Ordering in the Scheduler

**ID:** WRK-029
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-029_enforce-dependency-ordering-in-the-scheduler_PRD.md
**Tech Research:** ./WRK-029_enforce-dependency-ordering-in-the-scheduler_TECH_RESEARCH.md
**Mode:** Medium

## Overview

Add dependency enforcement to the orchestrator's scheduler by inserting a lightweight filtering layer into `select_actions()` and `select_targeted_actions()`, plus graph validation in preflight. The design introduces two new pure functions — `has_unmet_dependencies()` for runtime scheduling and `validate_dependency_graph()` for preflight — keeping them separate because "absent ID" has opposite meanings in each context (met at runtime, error at preflight). No new types, statuses, or external crates are needed.

---

## System Design

### High-Level Architecture

The change adds two independent capabilities to existing modules:

```
┌─────────────────────────────────────────────────┐
│                   Preflight                      │
│  run_preflight()                                 │
│    ├── validate_structure()    (existing)         │
│    ├── validate_workflows()   (existing)         │
│    ├── validate_items()       (existing)         │
│    └── validate_dependency_graph()  ◄── NEW      │
│         ├── detect_dangling_refs()                │
│         └── detect_cycles()  (DFS three-color)   │
└─────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────┐
│                   Scheduler                      │
│  select_actions()                                │
│    ├── destructive lock check  (existing)        │
│    ├── promotion: Ready → InProgress             │
│    │     └── has_unmet_dependencies() ◄── NEW    │
│    ├── InProgress phase selection                 │
│    │     └── has_unmet_dependencies() ◄── NEW    │
│    ├── Scoping phase selection                    │
│    │     └── has_unmet_dependencies() ◄── NEW    │
│    └── Triage (New items)                        │
│          └── has_unmet_dependencies() ◄── NEW    │
│                                                  │
│  select_targeted_actions()                       │
│    └── has_unmet_dependencies() ◄── NEW          │
└─────────────────────────────────────────────────┘
```

The two capabilities are independent: preflight validates graph structure at startup (cycles, dangling refs), while the scheduler filters items at runtime based on dynamic completion state.

### Component Breakdown

#### Component: `has_unmet_dependencies()` (scheduler.rs)

**Purpose:** Runtime check — determines if an item's dependencies are all satisfied in the current snapshot.

**Responsibilities:**
- Check each dependency ID in `item.dependencies` against snapshot items
- An ID absent from the snapshot means "archived/completed" → dependency met
- An ID present with status `Done` → dependency met
- An ID present with any other status → dependency unmet
- Empty `dependencies` list → no unmet dependencies

**Interfaces:**
- Input: `item: &BacklogItem`, `all_items: &[BacklogItem]`
- Output: `bool` (true = has unmet deps, skip this item)

**Dependencies:** None — pure function using only its arguments.

#### Component: `validate_dependency_graph()` (preflight.rs)

**Purpose:** Preflight structural validation — catches cycles and dangling references before execution starts.

**Responsibilities:**
- Build item ID set from `BacklogFile.items`
- Check each item's dependency list for IDs not in the set (dangling references)
- Run DFS three-color cycle detection across all non-Done items
- Accumulate all errors into `Vec<PreflightError>` (existing pattern)

**Interfaces:**
- Input: `items: &[BacklogItem]`
- Output: `Vec<PreflightError>`

**Dependencies:** None — pure function.

#### Sub-component: `detect_cycles()` (preflight.rs, private helper)

**Purpose:** DFS three-color algorithm that finds all cycles in the dependency graph.

**Responsibilities:**
- Traverse the dependency graph using three states: Unvisited, InStack, Done
- When a back-edge is found (dependency points to an InStack node), extract the cycle path from the explicit path vector
- Self-dependencies are naturally handled: when visiting node A, A is marked InStack before checking its deps. If A lists itself, the InStack check triggers back-edge detection — no explicit pre-check needed.
- Only traverse edges that point to known item IDs (skip dangling refs — those are caught separately)

**Implementation approach:** Recursive DFS with an explicit `path: Vec<&str>` maintained alongside the recursion to enable cycle path extraction. Each node is pushed to `path` on entry and popped on exit. When a back-edge is found, the cycle path is extracted from `path` starting at the back-edge target.

**Interfaces:**
- Input: `items: &[BacklogItem]` (non-Done items only)
- Output: `Vec<Vec<String>>` (each inner vec is a cycle path like `["A", "B", "C", "A"]`)

**Dependencies:** Uses `HashSet` and `HashMap` from std (no external crates).

### Data Flow

**Preflight path (startup, once):**
1. `run_preflight()` calls `validate_dependency_graph(&backlog.items)` as a new phase
2. `validate_dependency_graph()` builds an ID set, checks for dangling refs, then runs `detect_cycles()` on non-Done items
3. Errors are accumulated into the existing `Vec<PreflightError>` and returned with all other preflight errors

**Runtime path (each scheduler iteration):**
1. `select_actions()` is called with `&BacklogSnapshot`
2. At each of the four scheduling points (promotion, InProgress, Scoping, Triage), before adding an action, call `has_unmet_dependencies(item, &snapshot.items)`
3. If unmet → skip the item (debug log if logging is added)
4. If met → proceed with existing logic (running check, phase selection, etc.)

### Key Flows

#### Flow: Normal Scheduling with Dependencies

> Item B depends on Item A. A completes, then B becomes schedulable.

1. **Preflight** — `validate_dependency_graph()` checks that B's dependency "A" exists in the backlog. No cycles. Passes.
2. **Iteration 1** — A is `Ready`, B is `Ready`. Scheduler promotes A to InProgress (deps: none). B has unmet dep on A (A is not Done). B is skipped.
3. **Iteration 2** — A is `InProgress`, phase is assigned. B still skipped (A not Done).
4. **Iteration N** — A reaches `Done`. `has_unmet_dependencies(B, items)` returns false (A is Done). B is promoted and scheduled normally.

**Edge cases:**
- A is archived between iterations → A absent from snapshot → dependency met → B schedulable
- B has multiple deps [A, C] → B only schedulable when ALL are Done or absent

#### Flow: Preflight Cycle Detection

> Items A → B → C → A form a cycle.

1. `validate_dependency_graph()` builds ID set {A, B, C}
2. `detect_cycles()` starts DFS at A: A→InStack, visit dep B→InStack, visit dep C→InStack, visit dep A→already InStack (back-edge!)
3. Extract cycle path: ["A", "B", "C", "A"]
4. Create `PreflightError` with condition: "Circular dependency detected: A → B → C → A"
5. Preflight fails. Execution does not start.

**Edge cases:**
- Self-dependency: A depends on A → DFS marks A as InStack, checks dep A → InStack → cycle ["A", "A"]
- Multiple independent cycles → all detected and reported
- Cycle involving a Blocked item → still detected (all non-Done items participate)

#### Flow: Preflight Dangling Reference Detection

> Item B depends on "WRK-099" which doesn't exist in the backlog.

1. `validate_dependency_graph()` builds ID set from all items
2. For item B, checks each dep: "WRK-099" not in ID set
3. Create `PreflightError` with condition: "Item 'B' has dependency on 'WRK-099' which does not exist in the backlog"
4. Suggested fix: "Remove 'WRK-099' from B's dependencies, or add the missing item to the backlog"
5. Preflight fails.

**PreflightError format for dangling references:**
- `condition`: `"Item '{item_id}' depends on '{dep_id}' which does not exist in the backlog"`
- `config_location`: `"BACKLOG.yaml → items → {item_id} → dependencies"`
- `suggested_fix`: `"Remove '{dep_id}' from {item_id}'s dependencies, or add the missing item to the backlog"`

**PreflightError format for cycles:**
- `condition`: `"Circular dependency detected: {path joined with ' → '}"`
- `config_location`: `"BACKLOG.yaml → items → dependencies"`
- `suggested_fix`: `"Remove one dependency in the cycle to break it: {cycle items}"`

**Edge cases:**
- Item whose dependency was recently archived → preflight correctly flags this as dangling. Operator should clean up the stale reference.

#### Flow: Transitive Dependency Chain

> Items C → B → A form a chain (C depends on B, B depends on A).

1. **Preflight** — No cycle (A→B→C is a DAG). All IDs exist. Passes.
2. **Iteration 1** — A is schedulable (no deps). B is skipped (A not Done). C is skipped (B not Done).
3. **Iterations 2-N** — A progresses through phases. B and C remain skipped.
4. **Iteration N+1** — A reaches Done. B becomes schedulable (A is Done). C still skipped (B is not Done).
5. **Iterations N+2 to M** — B progresses through phases.
6. **Iteration M+1** — B reaches Done. C becomes schedulable. Transitive ordering emerges naturally without explicit transitive resolution.

#### Flow: Targeted Mode with Unmet Dependencies

> User runs `orchestrate run --target WRK-028` but WRK-028 depends on WRK-026 which is InProgress.

1. `select_targeted_actions()` finds WRK-028 in snapshot
2. Calls `has_unmet_dependencies(WRK-028, items)` → WRK-026 is InProgress, not Done → returns true
3. Returns empty actions vec
4. Scheduler loop continues. If WRK-026 is being worked on by another running task, the loop waits.
5. If no running tasks and no actions → exits via existing `AllDoneOrBlocked` halt logic
6. If "Should Have" logging is implemented, the halt summary includes which dependencies blocked WRK-028

---

## Technical Decisions

### Key Decisions

#### Decision: Separate functions for preflight vs. runtime dependency checking

**Context:** Both preflight and runtime need to check dependency IDs, but the semantics of "absent ID" differ: error in preflight, met in runtime.

**Decision:** Two separate functions — `validate_dependency_graph()` (preflight) and `has_unmet_dependencies()` (runtime) — with no shared code between them.

**Rationale:** Tech research identified this as a critical area: "The preflight validation function and the runtime filtering function must use **different logic** for absent IDs. This should be two separate functions, not a shared one with a mode flag." A shared function would be fragile and obscure the semantic difference. The functions are small enough (~10 lines for runtime, ~40-50 for preflight including cycle detection) that duplication of the ID lookup is negligible.

**Consequences:** If the "met" definition changes, both functions must be updated independently. This is acceptable because the definitions are unlikely to diverge further and the risk of inconsistency is low given the small code size.

#### Decision: DFS three-color for cycle detection (no external crate)

**Context:** Need to detect cycles and report the exact cycle path for actionable error messages.

**Decision:** Custom recursive DFS with three states (Unvisited, InStack, Done). No petgraph or other crate.

**Rationale:** The algorithm is ~40 lines, well-understood, O(V+E), and reports cycle paths. Adding petgraph for this would be overkill (adds a dependency for functionality we can implement simply). Recursive DFS is safe for our scale (<100 items, no deep chains).

**Consequences:** We own the cycle detection code and must test it ourselves. Mitigated by comprehensive unit tests covering self-deps, multi-node cycles, and multiple independent cycles.

#### Decision: Dependency filter is a scheduling concern, not a state machine concern

**Context:** Could introduce a `DependencyBlocked` status or reuse the existing `Blocked` status for items with unmet dependencies.

**Decision:** Items with unmet dependencies stay in their current status (New, Ready, Scoping, InProgress) but are silently skipped by the scheduler. No status changes.

**Rationale:** PRD explicitly requires this — dependency satisfaction is automatic and the `Blocked` status requires manual unblock. Adding a new status would change the state machine, affect serialization, and require migration. Filtering is simpler and reversible.

**Consequences:** Items with unmet dependencies "look" schedulable in the YAML but aren't — this could confuse operators. Mitigated by the "Should Have" debug logging and halt summary features.

#### Decision: Include all non-Done items in cycle detection

**Context:** PRD open question — should Blocked items participate in cycle detection?

**Decision:** Yes. All items except Done participate in cycle detection.

**Rationale:** A cycle involving a Blocked item (e.g., A→B→A where B is Blocked) would prevent both items from ever completing, even after B is unblocked. Catching this at preflight is correct. Done items are excluded because their dependencies are irrelevant (they're already complete).

**Consequences:** A Blocked item in a cycle will prevent startup. The operator must either remove the cycle or remove the item. This is the correct behavior — the cycle would cause permanent deadlock otherwise.

#### Decision: Block triage for items with unmet dependencies

**Context:** Tech research flagged that blocking triage delays assessment data. PRD explicitly requires it.

**Decision:** Follow the PRD — New items with unmet dependencies are not triaged.

**Rationale:** The PRD explicitly lists this as Must Have. While triage only assigns assessments and doesn't execute feature work, the PRD author made a deliberate choice to prevent any scheduling activity for dependency-blocked items. This provides a consistent mental model: unmet deps = completely inert.

**Consequences:** An item waiting on dependencies won't have size/risk/impact assessments when its dependencies clear. It will need to be triaged first, adding one iteration of latency before it starts executing. For most items this is negligible.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Stale reference on archival | Operator must clean up dependency references to archived items | Strict preflight validation that catches typos | Stale refs are a data hygiene issue, not a correctness issue. The runtime heuristic handles them correctly. |
| No triage while dependency-blocked | Slight latency in getting assessments | Consistent "completely inert" model for blocked items | One iteration delay is negligible; consistency reduces confusion |
| Redundant ID lookups per iteration | O(n*m) work each `select_actions()` call | Simplicity — no pre-computed ready set to maintain | At ~30 items × ~3 deps, this is microseconds |
| Custom cycle detection | We maintain ~40 lines of graph code | No external dependency for trivial functionality | Well-tested, well-understood algorithm at our scale |

---

## Alternatives Considered

### Alternative: Pre-computed Dependency Satisfaction Set

**Summary:** Build a `HashSet<String>` of "schedulable" item IDs at the start of `select_actions()` by computing which items have all dependencies met, then filter using set membership instead of calling `has_unmet_dependencies()` at each scheduling point.

**How it would work:**
- At the top of `select_actions()`, iterate all items and build `schedulable_ids: HashSet<&str>`
- At each scheduling point, check `schedulable_ids.contains(&item.id)` instead of calling the helper

**Pros:**
- Single dependency evaluation pass instead of repeated checks
- Slightly cleaner call sites (one set lookup vs. function call with items slice)

**Cons:**
- More setup code at the top of `select_actions()`
- Marginal performance gain at our scale (microseconds saved)
- Less explicit — the reader must trace back to the set construction to understand what "schedulable" means

**Why not chosen:** The per-item helper is clearer, easier to test in isolation, and the performance difference is negligible. The helper approach also makes it trivial to add debug logging at the check site (you can log exactly which deps are unmet for which item).

### Alternative: Kahn's Algorithm for Cycle Detection

**Summary:** Use BFS-based topological sort (Kahn's algorithm) for cycle detection, which also produces an execution order.

**How it would work:**
- Compute in-degree for all items
- Process items with in-degree 0, decrementing neighbors
- If not all items processed, remaining items form cycles

**Pros:**
- Also produces topological order (could be used for scheduling priority)
- Well-understood, same O(V+E) complexity

**Cons:**
- Doesn't naturally report the specific cycle path — only detects that cycles exist among remaining items
- We don't need topological order (the scheduler already has its own priority system)
- Slightly more setup (in-degree map construction)

**Why not chosen:** The PRD requires reporting "the complete cycle path identified." DFS three-color naturally tracks the path via the recursion stack. Kahn's would require additional work to extract the specific cycle, negating its simplicity advantage.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Typo in dependency ID introduced after startup (e.g., via future inbox) | Item silently treated as dependency-met at runtime (absent = met heuristic) | Low | Preflight catches at next restart; operators can manually edit BACKLOG.yaml |
| Recursive DFS stack overflow on very deep chains | Stack overflow crash | Very Low | Max ~100 items, max realistic chain depth ~10. Not a concern at current scale. |
| Operator confusion when items "look schedulable" in YAML but aren't | Wasted debugging time | Medium | "Should Have" debug logging and halt summary mitigate this |

---

## Integration Points

### Existing Code Touchpoints

- `scheduler.rs:select_actions()` — Add `has_unmet_dependencies()` filter at four logical points:
  1. **Before Ready→InProgress promotion** — filter `ready_items` before `.take(promotions_needed)` so items with unmet deps don't consume WIP slots
  2. **Before InProgress phase assignment** — after `running.is_item_running()` check, before `build_run_phase_action()`
  3. **Before Scoping phase assignment** — same pattern as InProgress
  4. **Before Triage action for New items** — after `running.is_item_running()` check, before pushing `Triage` action
- `scheduler.rs:select_targeted_actions()` — Add `has_unmet_dependencies()` check after finding target item by ID, before the status-based dispatch. If target has unmet deps, return empty vec.
- `preflight.rs:run_preflight()` — Add `validate_dependency_graph()` call as a new validation phase (Phase 4), after `validate_items()`. Runs unconditionally (not gated on previous phases passing) to report all errors at once.
- `tests/scheduler_test.rs` — Add unit tests using existing `make_item()` (set `dependencies` field) and `make_snapshot()` helpers
- `tests/preflight_test.rs` — Add unit tests using existing `make_item()` and `make_backlog()` helpers

### External Dependencies

- None. No new crates, no new I/O, no new configuration.

### Known Limitations

- **Runtime YAML edits bypass validation.** If an operator edits `BACKLOG.yaml` during execution to add dependencies (including cycles or dangling refs), these are not validated until the next `orchestrate run` invocation. Cycles introduced mid-run cause items to be permanently unschedulable for the remainder of that run with no error. This matches the PRD's explicit out-of-scope statement: "Runtime dependency validation for items added mid-run... occurs at preflight only."
- **Snapshot consistency assumption.** The `has_unmet_dependencies()` function relies on `BacklogSnapshot` being an immutable point-in-time view. This invariant is guaranteed by the existing architecture (snapshot is `&BacklogSnapshot`, no interior mutability).

---

## Open Questions

- [x] Should `Blocked` items participate in cycle detection? → **Yes**, all non-Done items (see Decision above)
- [x] Should triage be blocked by unmet dependencies? → **Yes**, per PRD (see Decision above)
- [x] Should dependency-blocked iterations count against the circuit breaker's exhaustion counter? → **No, and no change needed.** Tech research confirmed the existing code path (`actions.is_empty() && running.is_empty()` → `AllDoneOrBlocked`) already handles this correctly. The circuit breaker only counts consecutive failed phase executions, not empty scheduling iterations.
- [x] Should a distinct `HaltReason` variant exist for "all items dependency-blocked"? → **No.** The existing `AllDoneOrBlocked` halt is sufficient. The "Should Have" halt summary provides the diagnostic detail. A new variant adds enum complexity for minimal benefit.

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
| 2026-02-12 | Initial design draft | Two-component design (preflight validation + runtime filtering), medium mode, DFS three-color for cycles, per-item helper for runtime |
| 2026-02-12 | Self-critique (7 agents) and auto-fixes | Fixed terminology consistency (GRAY→InStack), added PreflightError format spec, added transitive dependency chain flow, closed open questions with tech research findings, replaced line-number refs with logical descriptions, added known limitations section, referenced tech research pitfall in separate-functions decision |
