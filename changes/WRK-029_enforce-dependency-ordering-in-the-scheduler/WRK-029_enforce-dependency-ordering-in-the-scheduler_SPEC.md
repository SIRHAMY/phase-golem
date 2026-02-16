# SPEC: Enforce Dependency Ordering in the Scheduler

**ID:** WRK-029
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-029_enforce-dependency-ordering-in-the-scheduler_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

The orchestrator's `BacklogItem` has a `dependencies: Vec<String>` field that is fully serialized/deserialized but never read by the scheduler. Items with unmet dependencies are scheduled identically to items with no dependencies, leading to out-of-order execution. This is already a concrete problem: WRK-028 depends on WRK-026 but could be executed before WRK-026 completes.

This change adds two capabilities: (1) runtime dependency filtering in the scheduler so items with unmet dependencies are skipped, and (2) preflight validation to catch cycles and dangling references at startup.

**PRD open questions resolved:**
- Blocked items participate in cycle detection — yes, all non-Done items (a cycle involving a Blocked item would prevent completion after unblock)
- Distinct halt reason for dependency-blocked — no, use existing `AllDoneOrBlocked`; Phase 3 logging provides diagnostics
- Circuit breaker interaction — no change needed; the circuit breaker counts consecutive failed phase executions, not empty scheduling iterations; `AllDoneOrBlocked` halt fires when `actions.is_empty() && running.is_empty()`, which is correct behavior

## Approach

Two independent pure functions, placed in the modules where they're used:

- **`has_unmet_dependencies(item, all_items) -> bool`** in `scheduler.rs` — Runtime filter. Checks each dependency ID against snapshot items. Absent IDs = met (archived). Done IDs = met. Anything else = unmet. Called at all four scheduling points plus targeted mode.

- **`validate_dependency_graph(items) -> Vec<PreflightError>`** in `preflight.rs` — Preflight validation. Builds an ID set, checks for dangling references, then runs DFS three-color cycle detection on non-Done items. Called from `run_preflight()` as Phase 4.

These are separate functions (not shared with a mode flag) because "absent ID" has opposite meanings: met at runtime, error at preflight.

**Patterns to follow:**

- `src/scheduler.rs:252-314` — Sorting helper pattern (`sorted_ready_items`, etc.): pure functions, filter + sort, `Vec<&BacklogItem>` return type
- `src/scheduler.rs:172-176` — Promotion loop: apply `running.is_item_running()` check before action creation — dependency check follows same pattern
- `src/preflight.rs:226-303` — `validate_items()`: loop over items, accumulate errors into `Vec<PreflightError>`, use `continue` to skip non-applicable items
- `src/preflight.rs:36-59` — `run_preflight()`: Phase 3 (`validate_items`) runs unconditionally — Phase 4 follows same pattern
- `tests/scheduler_test.rs:72-97` — `make_item()` helper: `dependencies: Vec::new()` already present, set via `item.dependencies = vec![...]`
- `tests/preflight_test.rs:14-39` — `make_item()` helper: same pattern, different signature (no title param)

**Implementation boundaries:**

- Do not modify: `ItemStatus` enum, `BacklogItem` struct, `BacklogSnapshot` struct, `Blocked` status handling
- Do not refactor: existing sorting helpers, `validate_items()`, `run_preflight()` structure
- Do not add: new item statuses, external crates, CLI commands, transitive resolution

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Preflight Dependency Validation | Med | Add `validate_dependency_graph()` with cycle detection and dangling reference checks to preflight |
| 2 | Runtime Dependency Filtering | Med | Add `has_unmet_dependencies()` and integrate at all scheduler decision points |
| 3 | Observability & Polish | Low | Add debug logging for skipped items and dependency info in halt summaries |

**Ordering rationale:** Phase 1 (preflight) is independent and catches structural problems. Phase 2 (runtime) is the core behavioral change. Phase 3 (logging) depends on both Phase 1 and 2 being in place to add observability.

---

## Phases

Each phase should leave the codebase in a functional, stable state. Complete and verify each phase before moving to the next.

---

### Phase 1: Preflight Dependency Validation

> Add cycle detection and dangling reference validation to preflight startup checks

**Phase Status:** complete

**Complexity:** Med

**Goal:** Detect circular dependencies (including self-deps) and dangling references in the dependency graph at orchestrator startup, preventing execution when graph errors exist.

**Files:**

- `src/preflight.rs` — modify — Add `validate_dependency_graph()` function with `detect_cycles()` private helper, and call from `run_preflight()`
- `tests/preflight_test.rs` — modify — Add tests for cycle detection, self-dependency, dangling references, and edge cases

**Patterns:**

- Follow `validate_items()` in `src/preflight.rs:226-303` for error accumulation pattern: loop over items, push `PreflightError` with condition/config_location/suggested_fix
- Follow `run_preflight()` in `src/preflight.rs:36-59` Phase 3 pattern (unconditional) for adding Phase 4 call
- Follow test pattern in `tests/preflight_test.rs:112-119` for success/failure assertions

**Tasks:**

- [x] Add `use std::collections::HashMap;` import to `preflight.rs` (already has `HashSet`)
- [x] Implement `validate_dependency_graph(items: &[BacklogItem]) -> Vec<PreflightError>` in `preflight.rs`:
  - Build `HashSet<&str>` of all item IDs
  - For each item, check each dependency ID against the set; push `PreflightError` for missing IDs with format: condition=`"Item '{item_id}' depends on '{dep_id}' which does not exist in the backlog"`, config_location=`"BACKLOG.yaml → items → {item_id} → dependencies"`, suggested_fix=`"Remove '{dep_id}' from {item_id}'s dependencies, or add the missing item to the backlog"`
  - Filter to non-Done items, call `detect_cycles()`, convert cycle paths to `PreflightError` with format: condition=`"Circular dependency detected: {path joined with ' → '}"`, config_location=`"BACKLOG.yaml → items → dependencies"`, suggested_fix=`"Remove one dependency in the cycle to break it: {cycle items}"`
- [x] Implement `detect_cycles(items: &[BacklogItem]) -> Vec<Vec<String>>` as private helper:
  - DFS three-color algorithm (Unvisited, InStack, Done) using `HashMap<&str, VisitState>`
  - Maintain explicit `path: Vec<&str>` alongside recursion for cycle path extraction
  - Only traverse edges to known item IDs (skip dangling refs — caught separately)
  - Self-dependencies handled naturally: node marked InStack before checking deps, self-edge triggers back-edge detection
  - Return each cycle as `Vec<String>` like `["A", "B", "C", "A"]`
- [x] Add `validate_dependency_graph()` call in `run_preflight()` after line 52 (`validate_items`): `errors.extend(validate_dependency_graph(&backlog.items));` — runs unconditionally like Phase 3
- [x] Write tests for dangling reference detection:
  - `preflight_dangling_dependency_fails` — Item depends on non-existent ID → error with condition containing the missing ID
  - `preflight_multiple_dangling_references` — Multiple items with dangling deps → multiple errors
  - `preflight_valid_dependencies_passes` — All deps exist → no errors from dependency validation
- [x] Write tests for cycle detection:
  - `preflight_self_dependency_fails` — Item depends on itself → cycle error with path `["A", "A"]`
  - `preflight_two_node_cycle_fails` — A→B→A → cycle error, assert error condition contains `"A → B → A"` format
  - `preflight_three_node_cycle_fails` — A→B→C→A → cycle error, assert error condition contains full path in `" → "` format
  - `preflight_multiple_independent_cycles` — Two separate cycles → both detected, assert error count equals 2 and both cycle paths present
  - `preflight_cycle_with_blocked_item_detected` — Cycle involving Blocked item → detected (all non-Done items participate)
  - `preflight_done_items_excluded_from_cycle_detection` — Done item in dependency chain → not included in DFS
  - `preflight_diamond_dag_no_false_positive` — A→B, A→C, B→D, C→D (diamond) → no cycle errors (reconvergent paths are not cycles)
  - `preflight_transitive_chain_no_cycle` — C→B→A chain (valid DAG) → no errors
  - `preflight_no_dependencies_passes` — Items with empty deps → no errors

**Verification:**

- [x] All new preflight tests pass
- [x] Existing preflight tests still pass (no regressions)
- [x] `cargo build` succeeds
- [x] `cargo test` passes
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-029][P1] Feature: Add preflight dependency graph validation`

**Notes:**

The `detect_cycles()` function uses recursive DFS which is safe at our scale (<100 items). The algorithm is ~40-50 lines. All non-Done items participate in cycle detection — this catches cycles involving Blocked items that would cause permanent deadlock.

**Followups:**

- Duplicate ID validation in preflight (currently not checked by any phase; would be a separate validation concern)
- Pre-build HashMap for O(1) item lookup in cycle detection (optimization for large backlogs)

---

### Phase 2: Runtime Dependency Filtering

> Add dependency check to scheduler so items with unmet dependencies are skipped at all scheduling points

**Phase Status:** complete

**Complexity:** Med

**Goal:** Prevent items with unmet dependencies from being promoted, assigned phases, or triaged. Items with all dependencies met (Done or absent from snapshot) are scheduled normally.

**Files:**

- `src/scheduler.rs` — modify — Add `has_unmet_dependencies()` helper function and integrate at 5 scheduling points (4 in `select_actions`, 1 in `select_targeted_actions`)
- `tests/scheduler_test.rs` — modify — Add comprehensive tests for dependency filtering at each scheduling point

**Patterns:**

- Follow `sorted_ready_items()` in `src/scheduler.rs:252-264` for pure helper function pattern
- Follow promotion loop in `src/scheduler.rs:172-176` for filtering: `running.is_item_running()` check before action → add `has_unmet_dependencies()` check in same pattern
- Follow test pattern in `tests/scheduler_test.rs:309-330` for scheduler test structure: setup snapshot/running/config, call `select_actions()`, filter actions by type, assert

**Tasks:**

- [x] Implement `has_unmet_dependencies(item: &BacklogItem, all_items: &[BacklogItem]) -> bool` as private function in `scheduler.rs` (place near other helpers, after line 314):
  - If `item.dependencies.is_empty()`, return `false`
  - For each dep ID in `item.dependencies`: find in `all_items` by ID. If not found → met (absent = archived). If found with status `Done` → met. If found with any other status → return `true` (unmet).
  - Return `false` if all met
- [x] Integrate in `select_actions()` at Ready→InProgress promotion (line 172): filter `ready_items` before `.take(promotions_needed)` by adding `.filter(|item| !has_unmet_dependencies(item, &snapshot.items))` to the iterator chain. **Critical:** the filter MUST be applied before `.take()` so items with unmet deps don't count toward or consume WIP slots
- [x] Integrate in `select_actions()` at InProgress phase assignment (lines 183-186): add `if has_unmet_dependencies(item, &snapshot.items) { continue; }` after the `running.is_item_running()` check
- [x] Integrate in `select_actions()` at Scoping phase assignment (lines 194-197): same pattern as InProgress
- [x] Integrate in `select_actions()` at Triage/New items (lines 205-208): same pattern — `if has_unmet_dependencies(item, &snapshot.items) { continue; }`
- [x] Integrate in `select_targeted_actions()` (after line 650, after finding target item but before status dispatch): `if has_unmet_dependencies(target, &snapshot.items) { return Vec::new(); }`
- [x] Write tests for `has_unmet_dependencies()` logic via `select_actions()`:
  - `test_ready_item_with_unmet_dep_not_promoted` — Ready item depends on Ready item → not promoted
  - `test_ready_item_with_met_dep_promoted` — Ready item depends on Done item → promoted
  - `test_ready_item_with_absent_dep_promoted` — Ready item depends on ID not in snapshot → promoted (absent = met)
  - `test_ready_item_with_partial_deps_not_promoted` — Item depends on [A, B], A is Done, B is Ready → not promoted
  - `test_ready_item_with_blocked_dep_not_promoted` — Ready item depends on Blocked item → not promoted (Blocked ≠ Done)
  - `test_ready_item_with_in_progress_dep_not_promoted` — Ready item depends on InProgress item → not promoted
  - `test_in_progress_with_unmet_dep_no_phase_action` — InProgress item with unmet dep → no RunPhase action
  - `test_in_progress_with_met_dep_gets_phase_action` — InProgress item with met dep → RunPhase action
  - `test_scoping_with_unmet_dep_no_phase_action` — Scoping item with unmet dep → no RunPhase action
  - `test_new_item_with_unmet_dep_not_triaged` — New item with unmet dep → no Triage action
  - `test_new_item_with_met_dep_triaged` — New item with met dep → Triage action
  - `test_no_deps_scheduled_normally` — Item with empty dependencies → scheduled as before
  - `test_unmet_dep_does_not_consume_wip_slot` — Ready item with unmet dep skipped, next eligible Ready item still promoted. Use `max_wip=1`, create 2 Ready items where first has unmet dep and second doesn't; verify only the second is promoted
- [x] Write tests for targeted mode:
  - `test_targeted_with_unmet_dep_returns_empty` — Target item has unmet dep → empty actions
  - `test_targeted_with_met_dep_returns_action` — Target item has met dep → action returned
  - `test_targeted_with_absent_dep_returns_action` — Target item dep absent → action returned

**Verification:**

- [x] All new scheduler tests pass
- [x] Existing scheduler tests still pass (no regressions)
- [x] `cargo build` succeeds
- [x] `cargo test` passes (all tests including Phase 1)
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-029][P2] Feature: Add runtime dependency filtering to scheduler`

**Notes:**

The `has_unmet_dependencies()` function is ~15 lines. Integration at each scheduling point is 1-3 lines. Made `select_targeted_actions` pub to enable direct unit testing.

**Followups:**

- Pre-build HashMap for O(1) dependency lookups (optimization for large backlogs, low priority at current scale)
- Parameterize dependency filtering tests to reduce duplication (test code quality improvement)

---

### Phase 3: Observability & Polish

> Add debug logging when items are skipped due to unmet dependencies and include dependency info in halt summaries

**Phase Status:** complete

**Complexity:** Low

**Goal:** Make dependency blocking visible to operators through debug-level logging and enhanced halt summaries, so operators can diagnose why items aren't being scheduled.

**Files:**

- `src/scheduler.rs` — modify — Add `log_debug!` calls at each dependency skip point, and include dependency-blocked items in halt summary

**Patterns:**

- Follow existing `log_debug!` usage in `src/scheduler.rs` for log formatting
- Follow `RunSummary` struct (lines 27-33) for halt summary data

**Tasks:**

- [x] Implement `pub fn unmet_dep_summary(item: &BacklogItem, all_items: &[BacklogItem]) -> Option<String>` helper (~15 lines) that iterates `item.dependencies`, looks up each in `all_items`, and returns `Some(comma-separated list)` of `"dep_id (status)"` for each unmet dependency, or `None` if all are met
- [x] Implement `fn skip_for_unmet_deps(item, all_items) -> bool` helper that combines dependency check and debug logging into a single call, eliminating duplicate traversal at each skip point
- [x] At each of the 5 integration points, call `skip_for_unmet_deps()` which logs `"Item {} skipped: unmet dependencies: {}"` at debug level when deps are unmet
- [x] In the scheduler's main loop (`run()` function), when the loop exits via `AllDoneOrBlocked` and non-Done items with dependencies exist, add a `log_info!` call listing each dependency-blocked item and its unmet deps. This is implemented as logging (not a `RunSummary` struct change) to keep the approach lightweight — the `AllDoneOrBlocked` halt reason already conveys the high-level outcome.
- [x] Write unit tests for `unmet_dep_summary()` helper covering: no unmet deps (None), single unmet dep, multiple unmet deps, mix of met/unmet deps

**Verification:**

- [x] All new and existing tests pass
- [x] `cargo build` succeeds
- [x] `cargo test` passes
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-029][P3] Feature: Add dependency-blocking observability logging`

**Notes:**

This phase implements the PRD's "Should Have" requirements. The logging is debug-level (per-item skip) and info-level (halt summary) to provide diagnostic info without noise. The halt summary is implemented via logging rather than a `RunSummary` struct change — the existing `AllDoneOrBlocked` halt reason is sufficient for programmatic use, and the log message provides the human-readable detail.

Implementation evolved from the SPEC: `unmet_dep_summary()` returns `Option<String>` instead of `String` (follows Result/Option pattern from style guide), and `has_unmet_dependencies()` was replaced by `skip_for_unmet_deps()` which combines the check and logging to avoid duplicate dependency traversal (addressed in code review).

**Followups:**

- Parameterize `unmet_dep_summary` tests to reduce boilerplate (test code quality)

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met (test name mapping):
  - [x] `select_actions()` excludes items with unmet dependencies → `test_ready_item_with_unmet_dep_not_promoted`, `test_in_progress_with_unmet_dep_no_phase_action`, `test_scoping_with_unmet_dep_no_phase_action`, `test_new_item_with_unmet_dep_not_triaged`
  - [x] Dependency filter applies before promotion across all four categories → `test_unmet_dep_does_not_consume_wip_slot`
  - [x] `select_targeted_actions()` applies the same filter → `test_targeted_with_unmet_dep_returns_empty`
  - [x] Preflight detects circular dependencies with cycle path → `preflight_self_dependency_fails`, `preflight_two_node_cycle_fails`, `preflight_three_node_cycle_fails`
  - [x] Preflight detects dangling references → `preflight_dangling_dependency_fails`
  - [x] Preflight runs at startup before first scheduler iteration → verified by `run_preflight()` call structure
  - [x] Empty dependencies treated as no dependencies → `test_no_deps_scheduled_normally`, `preflight_no_dependencies_passes`
  - [x] Unit tests cover all required scenarios → test list covers: met deps, unmet deps, absent deps, partial satisfaction, self-dep, cycles, dangling refs, targeted mode, promotion filtering
- [x] `cargo test` passes (all tests, no regressions) — 352 tests pass
- [x] `cargo clippy` passes without new warnings (3 pre-existing warnings in executor.rs and scheduler.rs)
- [x] Verify against current BACKLOG.yaml: WRK-028→WRK-026 dependency is valid (both exist), no dangling references or cycles

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| P1: Preflight Dependency Validation | complete | [WRK-029][P1] | Added validate_dependency_graph() with DFS cycle detection, dangling ref checks, 12 tests. Code review: replaced unwrap→expect, strengthened test assertions. |
| P2: Runtime Dependency Filtering | complete | [WRK-029][P2] | Added has_unmet_dependencies() (~15 lines), integrated at 5 scheduling points, 16 new tests. Made select_targeted_actions pub for testing. Code review passed clean. |
| P3: Observability & Polish | complete | [WRK-029][P3] | Added unmet_dep_summary() returning Option<String>, skip_for_unmet_deps() helper combining check+log, halt diagnostic logging. Refactored to eliminate duplicate traversal. 4 new tests. Code review: fixed duplicate traversal, extracted helper. |

## Followups Summary

### Critical

### High

### Medium

### Low

- Duplicate ID validation in preflight (currently not checked by any phase; would be a separate validation concern)
- Pre-build HashMap for O(1) item lookup in dependency checks (optimization for large backlogs, negligible at current ~30 items)
- Parameterize dependency filtering tests to reduce duplication (test code quality)
- Parameterize `unmet_dep_summary` tests to reduce boilerplate (test code quality)

## Design Details

### Key Types

No new types are introduced. Existing types used:

```rust
// Already exists — src/types.rs:148-188
pub struct BacklogItem {
    pub id: String,
    pub dependencies: Vec<String>,  // Line 177 — the field we're reading
    pub status: ItemStatus,
    // ...
}

// Already exists — src/preflight.rs:8-16
pub struct PreflightError {
    pub condition: String,
    pub config_location: String,
    pub suggested_fix: String,
}

// Internal to detect_cycles() — not exported
enum VisitState { Unvisited, InStack, Done }
```

### Architecture Details

The two functions have deliberately different semantics for absent IDs:

| Context | Absent ID Meaning | Rationale |
|---------|-------------------|-----------|
| `has_unmet_dependencies()` (runtime) | Dependency met | Archived items are removed from snapshot; archival implies completion |
| `validate_dependency_graph()` (preflight) | Error (dangling ref) | Preflight validates against full backlog file; absent = typo or stale reference |

This is why they are separate functions, not a shared one with a mode flag.

### Design Rationale

- **No new item status:** Dependency filtering is a scheduling concern, not a state machine concern. Items keep their current status but are silently skipped. This avoids migration, serialization changes, and interaction with the manual `Blocked` status.
- **DFS three-color for cycles:** Reports exact cycle paths (PRD requirement). ~40 lines, O(V+E), no external crate needed.
- **Per-item check (not pre-computed set):** Clearer at call sites, trivially testable in isolation, negligible performance cost at ~30 items.
- **All non-Done items in cycle detection:** Catches cycles involving Blocked items that would cause permanent deadlock after unblock.

---

## Retrospective

### What worked well?

- Three-phase decomposition was clean: preflight (structural), runtime (behavioral), observability (diagnostic) — each left the codebase stable
- Code review caught a real issue (duplicate dep traversal) that was addressed by combining `has_unmet_dependencies` + `unmet_dep_summary` into `skip_for_unmet_deps` + `unmet_dep_summary` returning `Option<String>`
- Existing test patterns (`make_item`, `make_snapshot`) made adding new tests fast

### What was harder than expected?

- Nothing significantly harder than expected — Phase 3 was straightforward as designed

### What would we do differently next time?

- SPEC could have specified `Option<String>` return type upfront instead of `String` — the code review caught this but it could have been designed correctly from the start
