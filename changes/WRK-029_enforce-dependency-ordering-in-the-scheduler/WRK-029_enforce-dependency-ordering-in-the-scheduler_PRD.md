# Change: Enforce Dependency Ordering in the Scheduler

**Status:** Proposed
**Created:** 2026-02-12
**Author:** Orchestrator (autonomous)

## Problem Statement

The orchestrator's `BacklogItem` struct defines a `dependencies: Vec<String>` field that references other item IDs. This field is fully serialized, deserialized, migrated from v1 schema, and round-tripped through BACKLOG.yaml. However, the scheduler's two core scheduling functions — `select_actions()` and `select_targeted_actions()` — never read the field. Items with unmet dependencies are scheduled, promoted, and executed identically to items with no dependencies.

Because the scheduler ignores this field, `dependencies` is currently decorative metadata with zero behavioral impact. An item declaring `dependencies: [WRK-026]` will be picked up and executed before WRK-026 is done, potentially producing incorrect or incomplete work products that require rework once the prerequisite lands.

This is already a concrete problem in the live backlog: WRK-028 ("Add structured description format") depends on WRK-026 ("Add inbox file"), and WRK-028's solution explicitly integrates with WRK-026's inbox mechanism (requires the inbox to exist for structured descriptions to be ingested). Without dependency enforcement, WRK-028 could be executed against a non-existent feature.

## User Stories / Personas

- **Orchestrator operator (scheduling)** — A developer running `orchestrate run` who declares inter-item dependencies in BACKLOG.yaml and expects the scheduler to respect them. Today, the operator must manually sequence work via `--target`, manipulate item statuses, or accept that dependency declarations are ignored.

- **Orchestrator operator (diagnosing)** — When the orchestrator produces no actions for multiple iterations and items exist in the backlog, the operator needs to understand which dependencies are blocking which items so they can take corrective action (e.g., prioritize a prerequisite, remove an incorrect dependency).

- **AI agent executing phases** — Receives item context but no signal about whether prerequisite work exists. Agents working on dependent items may produce artifacts that don't integrate with prerequisites that haven't been built yet.

## Desired Outcome

When a backlog item declares dependencies on other item IDs, the scheduler should not schedule that item until all its dependencies have reached `Done` status (or have been archived, which implies completion — see Constraints for archival semantics). Items with unmet dependencies remain in their current status but are excluded from scheduling — such items are not promoted, not triaged, and not assigned phases. When all of an item's dependencies reach `Done` status or are archived, the item becomes schedulable normally with no manual intervention.

Additionally, the preflight validation should detect dependency graph problems before execution begins: circular dependencies (including self-dependencies) that would cause permanent deadlocks, and dangling references to non-existent item IDs that indicate typos or stale data.

## Success Criteria

### Must Have

- [ ] `select_actions()` excludes items whose dependencies include any item ID that is present in the backlog snapshot with a status other than `Done`. Items whose dependency IDs are absent from the snapshot (archived/completed) are treated as having that dependency met.
- [ ] The dependency filter applies BEFORE promotion and scheduling across all four categories: Ready items are not promoted to InProgress if they have unmet dependencies, InProgress items with unmet dependencies are not assigned phases, Scoping items with unmet dependencies are not assigned phases, and New items with unmet dependencies are not triaged.
- [ ] `select_targeted_actions()` applies the same dependency filter. If the target item has unmet dependencies, no action is returned (the scheduler continues polling until dependencies clear or the circuit breaker halts execution).
- [ ] Preflight validation detects circular dependencies (including self-dependencies like `WRK-028: dependencies: [WRK-028]`) and reports them as errors with the complete cycle path identified (e.g., "Cycle detected: WRK-001 → WRK-002 → WRK-003 → WRK-001").
- [ ] Preflight validation detects dangling dependency references (IDs not present in the current backlog) and reports them as errors with both the referencing item ID and the missing dependency ID identified.
- [ ] Preflight validation runs during orchestrator startup before the first scheduler iteration. Validation failures prevent execution start (existing preflight behavior, unchanged).
- [ ] Items with an empty `dependencies` list (`dependencies: []`) or no dependencies field are treated as having no dependencies and are schedulable if they meet other criteria.
- [ ] Unit tests cover: items with met dependencies scheduled normally, items with unmet dependencies excluded, items with archived (absent) dependencies scheduled normally, items with partial dependency satisfaction (some met, some not) excluded, self-dependency detection, circular dependency detection, dangling reference detection, targeted mode with unmet dependencies, and Ready items with unmet dependencies not promoted.

### Should Have

- [ ] Debug-level logging when an item is skipped due to unmet dependencies, including the item ID, each unmet dependency ID, and the unmet dependency's current status (or "absent" if not in snapshot).
- [ ] When the scheduler halts with no actionable items and dependency-blocked items exist, the halt summary lists each dependency-blocked item ID and its unmet dependency IDs.

### Nice to Have

- [ ] (None currently — see Assumptions for rationale on dangling reference handling.)

## Scope

### In Scope

- A pure helper function `has_unmet_dependencies(item, snapshot)` that checks dependency satisfaction
- Integration of the helper as a scheduling filter in `select_actions()` and `select_targeted_actions()`
- Cycle detection in preflight validation using standard graph traversal (depth-first search for back-edge detection)
- Self-dependency detection as a special case of cycle detection
- Dangling dependency reference validation in preflight
- Unit tests for all new logic
- The dependency check treats "item ID absent from snapshot" as "dependency met" (archival implies completion — see Constraints)

### Out of Scope

- New `DependencyBlocked` or `WaitingOnDependency` item status — dependencies are a scheduling filter, not a state machine transition
- Automatic dependency inference from code overlap or semantic analysis
- Partial or soft dependency satisfaction — all dependencies are hard blockers
- Cross-run dependency tracking or persistent completed-item registries
- CLI commands for dependency management (`add-dependency`, `remove-dependency`, `show-graph`)
- Auto-scheduling of dependency chains in targeted mode (if `--target WRK-028` and WRK-028 depends on WRK-026, the scheduler waits rather than auto-scheduling WRK-026)
- Transitive dependency resolution at scheduling time — only direct dependencies are checked, which is sufficient because transitive ordering emerges naturally (if A depends on B and B depends on C, then A won't be scheduled until B is Done, and B won't be scheduled until C is Done)
- Retroactive dependency blocking — once a phase is assigned to an item and execution has started, it runs to completion regardless of dependency changes. Dependency checks apply only at scheduling decision time.
- Runtime dependency validation for items added mid-run (e.g., via future inbox mechanism). Dependency validation occurs at preflight only. Items added during execution bypass preflight checks until the next orchestrator restart.
- Maximum dependency chain depth limits
- Dependency ID format validation beyond existence checks (no enforcement of WRK-XXX pattern on dependency values)

## Non-Functional Requirements

- **Performance:** Dependency filtering adds O(n*m) lookups per scheduler iteration where n is the number of items and m is the total number of dependency edges. With current backlog sizes (~30 items, typically 0-3 dependencies per item), this O(n*m) complexity is negligible. No optimization needed.
- **Correctness:** The dependency filter must not prevent items from ever being scheduled. The "absent = met" heuristic combined with preflight dangling-reference validation ensures items with valid dependencies become schedulable as soon as all dependency items reach `Done` status or are archived.
- **Observability:** Debug logging for skipped items aids troubleshooting when the scheduler produces no actions for multiple iterations but non-Done items exist in the backlog.

## Constraints

- `select_actions()` must remain a pure function — no I/O, no coordinator calls. All dependency data is already available in the `BacklogSnapshot`.
- The `BacklogSnapshot` only contains active (non-archived) items. Items are archived only after reaching `Done` status, ensuring that absent dependency IDs represent completed work. The dependency filter must treat absent IDs as "dependency met."
- The `BacklogSnapshot` is consistent for the duration of a single `select_actions()` call. No concurrent modifications occur between dependency check and action selection within one iteration.
- The existing `Blocked` status with `blocked_reason` / `blocked_from_status` is for manual human-intervention blocks. Dependency-blocked items must NOT use this mechanism, as dependency satisfaction is automatic and the `Blocked` state requires explicit manual `unblock` operation. Items already in `Blocked` status are excluded from scheduling by existing logic; dependency filtering is not evaluated for them.
- No changes to the `ItemStatus` enum or its transition rules. `Done` is the only terminal status indicating completion.
- Dependency IDs reference `BacklogItem` IDs only. Dependencies on specific phases, external systems, or non-backlog entities are not supported.
- Schema validation (serde deserialization) ensures `dependencies` is a `Vec<String>`. Malformed YAML that fails deserialization is handled by existing error paths.

## Dependencies

- **Depends On:** Nothing — the `dependencies` field already exists on `BacklogItem` and is properly serialized/deserialized.
- **Blocks:** Any future work that relies on inter-item ordering, including WRK-028 (structured descriptions, depends on WRK-026) which already declares a dependency.
- **Assumes:** Scheduler circuit breaker behavior in `run()` that halts after 2 consecutive iterations where no items are actionable (exhaustion cycles). This existing behavior is unchanged by this change.
- **Assumes:** `archive_item()` removes items from `BacklogFile.items` and only archives items that have reached `Done` status. This existing behavior is unchanged.

## Risks

- [ ] **Archived item lookup logic** (High): The core heuristic "absent from snapshot = dependency met" is correct for archived items but also silently accepts typos. Preflight dangling-reference validation mitigates this at startup, but a typo introduced after startup (e.g., via future dynamic item addition mechanisms) would not be caught until the next run. Mitigation: operators can manually edit BACKLOG.yaml to remove incorrect dependency declarations as an immediate workaround.
- [ ] **Circular dependencies without preflight** (Medium): Without cycle detection, circular dependencies cause items to be permanently unschedulable with no error. The scheduler's circuit breaker (halts after 2 consecutive iterations with no actionable items) would halt execution, but the operator would get no explanation of what caused the exhaustion. Preflight cycle detection turns this from a silent deadlock into a clear startup error with the cycle path reported.
- [ ] **Target mode confusion** (Medium): A user targeting an item with unmet dependencies sees the scheduler exit (via circuit breaker) with no work done and no explanation unless the "Should Have" dependency-specific logging is implemented.
- [ ] **WIP slot consumption** (Low): If dependency filtering is not applied before Ready→InProgress promotion, items could be promoted to InProgress and then sit idle, consuming a WIP slot (`max_wip` constraint) without doing work. Mitigation: the Must Have criteria explicitly require filtering before promotion.

## Open Questions

- [ ] Should preflight dependency validation include `Blocked` items in cycle detection? Including them catches cycles involving manually blocked items; excluding them avoids validating items that are already suspended. Recommendation: include all non-Done items in cycle detection, since a cycle involving a Blocked item would prevent both items from ever completing even if unblocked.
- [ ] When the scheduler halts because all remaining items are dependency-blocked, should this be a distinct halt reason (e.g., `AllDependencyBlocked`) or folded into the existing `AllDoneOrBlocked` halt? A distinct reason provides better diagnostics but adds a new variant to the halt reason enum.
- [ ] How should the circuit breaker interact with dependency-blocked items? If all items are waiting on dependencies that are being actively worked (not stuck), the circuit breaker would halt after 2 idle iterations even though progress is expected. Should dependency-blocked iterations be excluded from the exhaustion counter?

## Assumptions

Decisions made without human input during autonomous PRD creation:

1. **Dangling references are errors, not warnings.** The initial draft had a contradiction between Must Have (errors) and Nice to Have (warnings) for absent dependency IDs. Resolved: preflight treats all dangling references as errors. This catches typos at startup. If a legitimate dependency has been archived, the operator should remove it from the dependencies list (it's already satisfied and the reference is stale). This is stricter but safer.
2. **Dependencies apply at scheduling time only, not retroactively.** Once a phase is assigned and executing, dependency changes don't halt it. This avoids complexity and race conditions.
3. **Dependency filtering applies before promotion.** Ready items with unmet dependencies are not promoted to InProgress, avoiding WIP slot waste.
4. **Self-dependencies are a cycle.** An item depending on itself is treated as a circular dependency error in preflight.

## References

- `types.rs:177` — `dependencies: Vec<String>` field definition
- `scheduler.rs:140-247` — `select_actions()` function
- `scheduler.rs:639-686` — `select_targeted_actions()` function
- `scheduler.rs:252-314` — Sorting/filtering helpers
- `preflight.rs:226-303` — `validate_items()` function
- `BACKLOG.yaml:284-302` — WRK-028 with live `dependencies: [WRK-026]` declaration
