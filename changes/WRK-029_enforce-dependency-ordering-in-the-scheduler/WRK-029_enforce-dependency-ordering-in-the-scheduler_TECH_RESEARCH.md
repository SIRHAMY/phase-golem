# Tech Research: Enforce Dependency Ordering in the Scheduler

**ID:** WRK-029
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-029_enforce-dependency-ordering-in-the-scheduler_PRD.md
**Mode:** Medium

## Overview

Researching how to implement dependency ordering in the orchestrator's scheduler. The `BacklogItem.dependencies: Vec<String>` field exists but is ignored by `select_actions()` and `select_targeted_actions()`. We need to understand: (1) what cycle detection algorithm to use for preflight validation, (2) how to integrate dependency filtering into the pure scheduling function, and (3) how the existing codebase patterns constrain the implementation.

## Research Questions

- [x] What algorithm should we use for cycle detection in the dependency graph?
- [x] Where exactly should dependency filtering be inserted in `select_actions()`?
- [x] Should we add a third-party graph library (petgraph) or implement cycle detection ourselves?
- [x] How does the existing `Blocked` status differ from dependency-blocking, and what are the implications?
- [x] Does the PRD's "absent = met" heuristic hold up under scrutiny?
- [x] Should triage be blocked by unmet dependencies?

---

## External Research

### Landscape Overview

Dependency-based task scheduling is a well-established domain built on graph theory, used extensively in build systems (Make, Bazel, Cargo), workflow orchestrators (Airflow, Databricks), and package managers. The core problem is managing a Directed Acyclic Graph (DAG) where nodes are tasks and edges are dependencies.

For small-scale systems (<100 items), the consensus is: **prioritize simplicity and correctness over performance**. The two main concerns are:
1. **Graph validation** — cycle detection and dangling reference checks (preflight)
2. **Dependency filtering** — determining which tasks are schedulable at runtime

### Common Patterns & Approaches

#### Pattern: DFS Three-Color Cycle Detection

**How it works:** Each vertex gets one of three colors during depth-first traversal — WHITE (unvisited), GRAY (in current DFS path), BLACK (fully processed). A cycle exists if DFS encounters a GRAY vertex (back-edge to an ancestor).

**When to use:** Preflight validation where you need to detect cycles AND report the specific cycle path.

**Tradeoffs:**
- Pro: Simple to implement, O(V+E) time, can report exact cycle path
- Pro: Natural fit for "find the cycle and report it" use cases
- Con: Only detects cycles — doesn't produce a topological order (not needed for our use case)

**References:**
- [Detect Cycle in directed graph using colors — GeeksforGeeks](https://www.geeksforgeeks.org/dsa/detect-cycle-direct-graph-using-colors/)
- [Finding Cycle in A Graph Using DFS](https://www.thealgorists.com/Algo/CycleDetectionUsingDFS)

#### Pattern: Kahn's Algorithm (BFS Topological Sort)

**How it works:** Compute in-degree for all vertices. Enqueue vertices with in-degree 0. Process queue: remove vertex, add to result, decrement neighbors' in-degree. If not all vertices processed, a cycle exists.

**When to use:** When you need both cycle detection AND a valid execution order in one pass.

**Tradeoffs:**
- Pro: Produces topological order, naturally identifies "ready" tasks (in-degree 0)
- Pro: O(V+E), well-understood
- Con: Doesn't report the specific cycle path (just detects existence)
- Con: Slightly more setup (in-degree computation)

**References:**
- [Kahn's Algorithm — GeeksforGeeks](https://www.geeksforgeeks.org/dsa/topological-sorting-indegree-based-solution/)
- [Scheduling Tasks with Topological Sorting — Bruno Scheufler](https://brunoscheufler.com/blog/2021-11-27-scheduling-tasks-with-topological-sorting)

#### Pattern: Ready Queue with Dependency Satisfaction Check

**How it works:** For each scheduler iteration, check if an item's dependencies are all satisfied (completed/absent). Only schedule items where all dependencies are met. No graph construction needed — just set membership checks.

**When to use:** Runtime filtering where tasks complete dynamically and you need to re-evaluate readiness each iteration.

**Tradeoffs:**
- Pro: Simplest to implement, natural fit for iterative schedulers
- Pro: No graph construction overhead
- Con: Doesn't detect cycles (needs separate validation)
- Con: O(n*m) per iteration (n items, m total dependency edges) — negligible for small backlogs

**References:**
- [System Design for Task Scheduling with Dependencies](https://www.cracksde.com/2023/12/system-design-for-task-scheduling-with-dependencies/)
- [Configure task dependencies — Databricks](https://docs.databricks.com/aws/en/jobs/run-if)

#### Pattern: Dangling Reference Validation

**How it works:** Build a set of all valid task IDs. For each task's dependency list, verify each dependency ID exists in the valid set. Report any mismatches.

**When to use:** Always — as preflight validation to catch typos and stale references.

**Tradeoffs:**
- Pro: O(V+E), trivial to implement
- Con: Must decide scope — validate against snapshot (active items only) or include historical knowledge

**References:**
- [Dependency Graph — Terraform](https://developer.hashicorp.com/terraform/internals/graph)

### Technologies & Tools

#### Rust Graph Libraries

| Technology | Purpose | Pros | Cons | Verdict |
|------------|---------|------|------|---------|
| [petgraph](https://crates.io/crates/petgraph) | General graph algorithms | Mature, `is_cyclic_directed()` built-in | Overkill for our use case, adds dependency | Skip — our graph is too simple |
| [graph-cycles](https://lib.rs/crates/graph-cycles) | Find ALL cycles (Johnson's) | Finds every cycle | More expensive than detection-only | Skip — we only need detection |
| Custom DFS | Cycle detection + path reporting | Zero dependencies, tailored to our needs | Must write and test ourselves | **Recommended** |

### Standards & Best Practices

1. **Preflight validation before execution** — Validate the dependency graph at startup, not during scheduling.
2. **Pure functional design** — Keep scheduling deterministic with no I/O.
3. **Separate validation from filtering** — Preflight catches structural errors (cycles, dangling refs); runtime filtering handles dynamic state (which deps are Done).
4. **Rich error types** — Report specific cycles and dangling references with actionable fix suggestions.
5. **Keep it simple** — For <100 items, O(n*m) is fine. No need for incremental in-degree tracking.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Not validating before execution | Circular dependencies cause silent deadlocks | Run cycle detection in preflight |
| Conflating dependency-blocking with manual blocking | Different semantics: auto vs human-resolved | Keep separate mechanisms (PRD already mandates this) |
| Over-engineering for small scale | Adding petgraph, in-degree caching, etc. for ~30 items | Simple set-based checks are sufficient |
| Not reporting cycle paths | "Cycle detected" with no path is unhelpful | DFS three-color naturally tracks the path |
| Ignoring self-dependencies | Self-dep is a trivial cycle but easy to miss in DFS | Check explicitly or ensure DFS handles it |

### Key Learnings

- DFS three-color is the best fit for our preflight cycle detection: it's O(V+E), reports the specific cycle path, and is straightforward to implement without external dependencies.
- Kahn's algorithm is overkill — we don't need topological ordering, just cycle detection and runtime filtering.
- Ready-queue pattern (set membership check) is the right approach for runtime dependency filtering in `select_actions()`.
- No external library needed — the graph is small and the algorithms are simple.

---

## Internal Research

### Existing Codebase State

**Relevant files/modules:**
- `src/scheduler.rs` — Main scheduling logic. `select_actions()` (lines 140-247) is a pure function. `select_targeted_actions()` (lines 639-686) handles `--target` mode. Both need dependency filtering.
- `src/types.rs` — `BacklogItem` (line 148) has `dependencies: Vec<String>` at line 177. `BacklogSnapshot` (line 221) holds all active items. `ItemStatus` enum (line 7) defines the state machine.
- `src/preflight.rs` — `PreflightError` struct (line 9) with `condition`, `config_location`, `suggested_fix`. `validate_items()` (line 226) iterates items and accumulates errors. `run_preflight()` (line 36) orchestrates all validation phases.
- `src/executor.rs` — `resolve_transition()` determines next status after phase completion. Not dependency-aware — no changes needed.
- `tests/scheduler_test.rs` — Test helpers: `make_item()`, `make_snapshot()`, `default_execution_config()`. Tests use pattern matching on `SchedulerAction`.
- `tests/preflight_test.rs` — Test helpers: `make_item()`, `make_backlog()`, `feature_pipeline_no_workflows()`. Tests validate error conditions.

**Existing patterns in use:**
- **Status-based filtering** — Sorting helpers (`sorted_ready_items`, etc.) filter by status using `.filter(|i| i.status == ...)`.
- **Running task exclusion** — `running.is_item_running(&item.id)` checks before adding actions.
- **Pure function design** — `select_actions()` takes only `&BacklogSnapshot`, `&RunningTasks`, config refs. No I/O.
- **Error accumulation** — Preflight collects all errors into `Vec<PreflightError>` rather than failing on first.

### Reusable Components

- `PreflightError` struct and error accumulation pattern — directly reusable for cycle/dangling errors
- `make_item()` test helper — already has `dependencies: Vec::new()`, just needs a variant with deps
- `BacklogSnapshot.items` — provides the complete item list for dependency lookups
- Sorting helper pattern (`sorted_ready_items`, etc.) — model for creating a `has_unmet_dependencies()` helper

### Constraints from Existing Code

- **`select_actions()` must remain pure** — no I/O, no coordinator calls. All data from `&BacklogSnapshot`.
- **`BacklogSnapshot` only contains active items** — archived items are absent. This is why "absent = met" is the correct heuristic.
- **`Blocked` status is for human-intervention blocks** — uses `blocked_from_status`, `blocked_reason`. Dependency-blocking must NOT use this mechanism (PRD constraint).
- **No graph crates in `Cargo.toml`** — cycle detection must be implemented from scratch (fine for our scale).
- **`ItemStatus` enum cannot change** — no new states. Dependency filtering is a scheduling concern, not a state machine concern.
- **Preflight receives `&BacklogFile`** (not snapshot) — has access to all items including Done ones.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Preflight uses `BacklogFile` items for dangling ref validation | `BacklogFile.items` only contains active (non-archived) items. Archived items are removed from the YAML entirely. | Dangling reference validation correctly catches IDs not in the file. But items that were archived (completed + archived) will also not be in the file. The PRD says preflight should error on dangling refs, but an item whose dependency was recently archived would trigger a false positive. **Resolution:** This is actually correct behavior — if a dependency was archived, the reference is stale and should be cleaned up. The PRD's Assumption #1 ("dangling references are errors") handles this correctly. |
| "absent from snapshot = dependency met" at scheduling time | BacklogSnapshot only contains active items. Archived items are absent. | At runtime (`select_actions`), this heuristic is correct: if a dependency ID is absent from the snapshot, it was archived (which means it completed). Preflight catches the typo case. |
| Dependency filter applies before promotion (Ready→InProgress) | The promotion loop (lines 170-176) currently only checks `running.is_item_running()`. | Adding a dependency check here is straightforward. This prevents WIP slot waste. |
| Triage should be blocked by unmet dependencies | The PRD says "New items with unmet dependencies are not triaged" (Must Have, bullet 4). | **This is questionable.** Triage assigns assessments (size, risk, impact) — it doesn't execute the actual feature work. Triaging a dependent item early is useful because it provides scheduling metadata for when the dependency clears. However, the PRD explicitly requires it, so we implement it. |
| Cycle detection should include `Blocked` items | Open question in PRD. | **Recommendation: Include all non-Done items.** A cycle involving a Blocked item would prevent both items from completing even after unblock. Catching this at preflight is correct. |

---

## Critical Areas

### Correct "Absent = Met" vs Dangling Reference Boundary

**Why it's critical:** The same condition (dependency ID not in items) means different things in different contexts:
- **Preflight** (`BacklogFile`): Absent = error (dangling reference, catch typos)
- **Runtime** (`BacklogSnapshot`): Absent = met (archived = completed)

**Why it's easy to miss:** A single `has_unmet_dependencies()` helper could be used in both contexts, but the semantics differ.

**What to watch for:** The preflight validation function and the runtime filtering function must use **different logic** for absent IDs. This should be two separate functions, not a shared one with a mode flag.

### Triage and Dependency Interaction

**Why it's critical:** The PRD says triage should be blocked by unmet dependencies, but triage only assigns assessments — it doesn't execute feature work.

**Why it's easy to miss:** Blocking triage could delay getting sizing information, which impacts scheduling priority once the dependency clears.

**What to watch for:** This is a product decision. The PRD explicitly requires it, but the design phase should confirm whether this is intentional.

### Circuit Breaker Interaction with Dependency-Blocked Items

**Why it's critical:** If all items are waiting on dependencies being actively worked, the circuit breaker's consecutive exhaustion counter could trip incorrectly.

**Why it's easy to miss:** The circuit breaker counts failed executions (`consecutive_exhaustions`), not empty scheduling iterations. If items are dependency-blocked, `select_actions()` returns empty, and the loop exits via `AllDoneOrBlocked` (line 467-470), not via circuit breaker.

**What to watch for:** The current code path is: `actions.is_empty() && running.is_empty()` → exit with `AllDoneOrBlocked`. If running tasks exist (working on the dependency), this check fails and the loop continues. The circuit breaker only trips on consecutive failed phase executions. **This is already correct** — dependency-blocked items don't interact with the circuit breaker.

### Self-Dependency as Edge Case in DFS

**Why it's critical:** A self-dependency (item depends on itself) is a trivial cycle but could be missed by DFS if the algorithm doesn't visit the current node's edges before marking it as visited.

**Why it's easy to miss:** Standard DFS implementations start by marking the node as GRAY and then visiting neighbors. If the node lists itself as a neighbor, the GRAY check correctly catches it. But some implementations skip self-edges.

**What to watch for:** Ensure the DFS implementation handles self-edges. Alternatively, add an explicit self-dependency check before the DFS (simpler and clearer).

---

## Deep Dives

### Where Exactly to Insert Dependency Filtering in `select_actions()`

**Question:** At which points in the function should dependency filtering be applied?

**Summary:** Five insertion points identified:

1. **Ready→InProgress promotion** (line 172): Filter `ready_items` before `.take(promotions_needed)`. Items with unmet deps should not count toward or consume promotion slots.

2. **InProgress phase selection** (line 183-189): After `running.is_item_running()` check, before `build_run_phase_action()`. Skip items with unmet deps.

3. **Scoping phase selection** (line 194-200): Same pattern as InProgress.

4. **Triage/New item selection** (line 204-209): After `running.is_item_running()` check, before pushing `Triage` action.

5. **`select_targeted_actions()`** (line 639-686): After finding the target item (line 647), before status-based dispatch (line 664). If target has unmet deps, return empty.

**Implications:** The cleanest approach is a single helper function `has_unmet_dependencies(item: &BacklogItem, items: &[BacklogItem]) -> bool` used at all five points. This keeps the filtering logic centralized and testable.

### DFS Cycle Detection Implementation Shape

**Question:** What does the cycle detection implementation look like in Rust?

**Summary:** The implementation uses a recursive DFS with three states per item:

```rust
enum VisitState { Unvisited, InStack, Done }

fn detect_cycles(items: &[BacklogItem]) -> Vec<Vec<String>> {
    let id_set: HashSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
    let mut state: HashMap<&str, VisitState> = ...;
    let mut path: Vec<&str> = Vec::new();
    let mut cycles: Vec<Vec<String>> = Vec::new();

    for item in items {
        if state[item.id.as_str()] == Unvisited {
            dfs(item.id.as_str(), items, &id_set, &mut state, &mut path, &mut cycles);
        }
    }
    cycles
}
```

Self-dependencies are naturally caught: when visiting node A, we check its dependencies. If A lists itself, A is already `InStack`, triggering cycle detection.

**Implications:** No external crate needed. The implementation is ~40-50 lines of Rust. Iterative DFS (using an explicit stack) could be used to avoid stack overflow on deep graphs, but with <100 items this is unnecessary.

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| Should triage be blocked by unmet dependencies? | Triage only assigns assessments, not feature work. Blocking it delays sizing data. | PRD says yes; could be reconsidered in design. |
| Should `Blocked` items participate in cycle detection? | A cycle involving a Blocked item prevents completion after unblock. | Recommended: include all non-Done items. |
| Should halting due to all items being dependency-blocked produce a distinct `HaltReason`? | Better diagnostics but adds enum variant. | Recommended: use existing `AllDoneOrBlocked` — the "Should Have" halt summary already covers diagnostics. |
| Should circuit breaker count dependency-blocked iterations? | Might halt prematurely if all items wait on in-flight work. | **Not an issue** — circuit breaker only counts consecutive failed executions, not empty scheduling iterations. The scheduler exits via `AllDoneOrBlocked` when no actions and no running tasks. |

### Recommended Approaches

#### Cycle Detection Algorithm

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| DFS Three-Color | Reports exact cycle path, simple, O(V+E) | Detection only (no topo order) | Need specific cycle paths for error messages |
| Kahn's Algorithm | Also produces topological order | Harder to extract specific cycle paths | Need both ordering and detection |
| petgraph `is_cyclic_directed()` | Zero implementation effort | Adds crate dependency, doesn't report path | Graph complexity grows beyond current needs |

**Initial recommendation:** DFS Three-Color — it's the simplest approach that reports cycle paths, which matches the PRD's requirement to report "the complete cycle path identified." No external dependency needed.

#### Runtime Dependency Filtering

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Per-item set check | Simplest, O(m) per item | Re-checks each iteration | Small backlogs (<100 items) |
| Pre-computed ready set | One pass builds ready set | Slightly more setup | Larger backlogs or hot loops |

**Initial recommendation:** Per-item set check — build a `HashSet<&str>` of Done item IDs (and note absent IDs as met), then for each item check if all deps are in the set or absent. Rebuild each `select_actions()` call. With ~30 items this is negligible.

#### External Library

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| petgraph | Battle-tested, comprehensive | Adds dependency, overkill for ~30 items | Complex graph operations needed |
| Custom implementation | Zero deps, tailored, educational | Must write and test ourselves | Simple graph needs |

**Initial recommendation:** Custom implementation — the algorithms are well-understood, the scale is tiny, and we avoid adding a dependency for 40 lines of code.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Detect Cycle in directed graph using colors — GeeksforGeeks](https://www.geeksforgeeks.org/dsa/detect-cycle-direct-graph-using-colors/) | Algorithm guide | DFS three-color implementation reference |
| [Scheduling Tasks with Topological Sorting — Bruno Scheufler](https://brunoscheufler.com/blog/2021-11-27-scheduling-tasks-with-topological-sorting) | Article | End-to-end task scheduling with dependencies |
| [Dependency Graph — Terraform](https://developer.hashicorp.com/terraform/internals/graph) | Docs | Real-world DAG validation in infrastructure tool |
| [petgraph — crates.io](https://crates.io/crates/petgraph) | Library | Reference if we ever need more complex graph ops |
| [Dependency Resolution — The Cargo Book](https://doc.rust-lang.org/cargo/reference/resolver.html) | Docs | How Rust's own build system handles dependency resolution |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | External research: dependency scheduling patterns | Identified 4 patterns; DFS three-color recommended for cycle detection |
| 2026-02-12 | Internal research: codebase exploration | Mapped 5 integration points in select_actions(), confirmed no graph crates, identified test patterns |
| 2026-02-12 | PRD analysis and synthesis | Identified 4 critical areas, resolved circuit breaker concern, flagged triage-blocking question |
