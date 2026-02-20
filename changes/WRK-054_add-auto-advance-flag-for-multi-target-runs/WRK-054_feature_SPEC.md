# SPEC: Add --auto-advance flag for multi-target runs

**ID:** WRK-054
**Status:** Ready
**Created:** 2026-02-19
**PRD:** ./WRK-054_feature_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

When running `phase-golem run --target WRK-005 --target WRK-010`, the scheduler halts the entire run if any target becomes blocked. This forces unnecessary babysitting for batch-style runs where targets are independent. The `--auto-advance` flag makes the scheduler skip blocked targets and continue to the next, matching industry patterns like `make -k` and Argo `continueOn`.

WRK-030 (multi-target support) is already shipped. The scheduler loop, block detection, circuit breaker, and target advancement infrastructure all exist. This change adds a ~15-line conditional branch at the existing block detection site, a boolean field on `RunParams`, a CLI flag, exit code logic, and `items_blocked` deduplication in `build_summary()`.

## Approach

Add `auto_advance: bool` to `RunParams`, parse `--auto-advance` from CLI, and insert a conditional branch at the existing block detection site (`src/scheduler.rs` lines 608-631). When `auto_advance` is true and a target is blocked, the scheduler logs the skip, commits state, resets the circuit breaker counter, increments the target index, and continues the loop. The existing halt behavior is preserved in the `else` branch. Exit code logic in `handle_run()` returns non-zero when all targets blocked and none completed.

**Patterns to follow:**

- `src/scheduler.rs:608-631` — existing block detection site with drain/commit pattern (auto-advance branch mirrors this)
- `src/scheduler.rs:468-510` — `advance_to_next_active_target()` for target advancement patterns and log format
- `src/main.rs:56-66` — existing CLI flag definitions for the `Run` subcommand (follow clap patterns)
- `tests/scheduler_test.rs:1886-1930` — `test_multi_target_halts_on_block()` for multi-target test setup patterns

**Implementation boundaries:**

- Do not modify: `advance_to_next_active_target()` — it handles pre-existing Blocked items; auto-advance operates on runtime blocks only
- Do not modify: `HaltReason` enum — no new variants needed; `TargetCompleted` + non-empty `items_blocked` is sufficient
- Do not modify: `SchedulerState` struct — no new fields needed; existing `items_blocked`, `consecutive_exhaustions`, and `current_target_index` are reused
- Do not refactor: the scheduler loop structure — this is a targeted conditional addition, not a restructure

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Core Implementation | Low | Add `auto_advance` field, CLI flag, conditional block branch, exit code logic, `items_blocked` dedup, and test fixup for new field |
| 2 | Tests | Low | Add integration tests for auto-advance behavior, circuit breaker reset, backward compatibility, and deduplication |

**Ordering rationale:** Phase 1 implements all production code and updates existing test struct literals for compilation. Phase 2 adds new test cases that exercise the Phase 1 logic. Tests are in a separate phase because they can be independently verified.

---

## Phases

Each phase should leave the codebase in a functional, stable state. Complete and verify each phase before moving to the next.

---

### Phase 1: Core Implementation

> Add `auto_advance` field to `RunParams`, CLI flag, conditional block detection branch, exit code logic, and `items_blocked` deduplication

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Implement all production code for `--auto-advance` so that the flag is accepted, the scheduler skips blocked targets when active, the circuit breaker resets between targets, blocked state is committed before advancing, the exit code reflects partial vs. total failure, and `items_blocked` is deduplicated in the summary.

**Files:**

- `src/scheduler.rs` — modify — Add `auto_advance: bool` field to `RunParams` (line 59), add conditional auto-advance branch at block detection site (lines 608-631), add `items_blocked` deduplication in `build_summary()` (line 1776)
- `src/main.rs` — modify — Add `--auto-advance` CLI flag to `Run` subcommand (line 56-66), thread flag into `RunParams` construction (line 624-630), add exit code logic after summary printing (line 738-740)
- `tests/scheduler_test.rs` — modify — Update all existing `RunParams` struct literals to include `auto_advance: false` (required for compilation after adding the field)

**Patterns:**

- Follow `src/scheduler.rs:614-630` for the drain/commit pattern in the auto-advance branch (symmetric with existing halt path)
- Follow `src/scheduler.rs:498-504` for `[target]` log prefix and position counter format
- Follow `src/main.rs:58-62` for clap `#[arg(long)]` flag definition patterns

**Tasks:**

- [ ] Add `pub auto_advance: bool` field to `RunParams` struct in `src/scheduler.rs` (after `config_base` field, line 59)
- [ ] Add `--auto-advance` boolean flag to `Run` variant in `src/main.rs` using `#[arg(long, action = clap::ArgAction::SetTrue)]`. Do not add `conflicts_with`; per PRD, the flag is silently ignored in non-target modes (filter mode already handles blocks via its own mechanism)
- [ ] Thread `auto_advance` value from the parsed `Run` variant into `RunParams` construction in `handle_run()` (`src/main.rs` line 624-630)
- [ ] Implement conditional auto-advance branch at block detection site (`src/scheduler.rs` lines 613-631):
  - If `params.auto_advance`: log skip with format `log_info!("[target] {} blocked ({}/{}). Auto-advancing.", target_id, state.current_target_index + 1, params.targets.len())`, call `drain_join_set()`, call `coordinator.batch_commit()`, reset `state.consecutive_exhaustions = 0`, increment `state.current_target_index += 1`, `continue`
  - Else: existing halt behavior (unchanged)
  - Note: `drain_join_set()` is called defensively despite PRD assumption "drain not needed" (max_wip=1 means join set is empty). The drain mirrors the existing halt path for correctness if max_wip changes in the future.
- [ ] Add `items_blocked` deduplication in `build_summary()` (`src/scheduler.rs` line 1776): call `.sort()` then `.dedup()` on `state.items_blocked` before constructing `RunSummary`. Since `build_summary()` takes `SchedulerState` by value, the mutation is safe (the state is consumed).
- [ ] Add exit code logic in `handle_run()` after the summary is printed (`src/main.rs` after line 738): if `summary.items_completed.is_empty() && !summary.items_blocked.is_empty()`, return `Err("All targets blocked; no items completed".to_string())` to trigger exit 1
- [ ] Update all existing `RunParams` struct literals in `tests/scheduler_test.rs` to include `auto_advance: false`. Note: the helper function `run_params()` at line 194 constructs `RunParams` and needs updating too — updating the helper propagates to all its callers.

**Verification:**

- [ ] `cargo build` succeeds with no errors or warnings
- [ ] `cargo clippy` passes with no new warnings
- [ ] `cargo run -- run --help` shows the `--auto-advance` flag in help output
- [ ] Existing tests pass: `cargo test` (no regressions — test fixup ensures compilation)
- [ ] Verify that `state.consecutive_exhaustions = 0` is written BEFORE the `continue` statement in the auto-advance branch (visual inspection)
- [ ] Code review passes (`/code-review` -> fix issues -> repeat until pass)

**Commit:** `[WRK-054][P1] Feature: Add --auto-advance flag with skip logic, exit codes, and dedup`

**Notes:**

Critical ordering in the auto-advance branch: reset `consecutive_exhaustions` BEFORE the `continue` statement. The circuit breaker check at line 572 runs at the top of the next loop iteration, before reaching the block detection branch. If the reset happens after `continue`, the next iteration's circuit breaker check will see stale state.

The exit code logic is intentionally mode-agnostic: it checks `items_completed` and `items_blocked` regardless of whether `--auto-advance` was passed. This is a deliberate behavior change from the current code, where `handle_run()` always returns `Ok(())` after printing the summary (exit 0 even when all targets blocked). The new logic makes exit 1 consistent for all-blocked runs in both target modes. The Design doc explicitly decided this (see exit code decision matrix).

The `--auto-advance` flag has no `conflicts_with` annotation with `--only`. When passed without `--target`, the flag is silently ignored because the auto-advance branch is gated on `!params.targets.is_empty()` and `state.items_blocked.contains(target_id)`, which only fires in target mode. This matches the PRD constraint: "The flag only applies to `--target` mode."

**Followups:**

---

### Phase 2: Tests

> Add integration tests for auto-advance behavior including skip, circuit breaker reset, backward compatibility, and deduplication

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Add test cases covering all auto-advance flows: single blocked target, skip-and-continue, all targets blocked, circuit breaker reset across targets, backward compatibility (no flag = halt on block), and `items_blocked` deduplication.

**Files:**

- `tests/scheduler_test.rs` — modify — Add new test functions in the multi-target integration test section (after existing multi-target tests, ~line 2447)
- `src/scheduler.rs` — modify — Add unit test for `build_summary()` deduplication (in a `#[cfg(test)]` module if one exists, or inline)

**Patterns:**

- Follow `tests/scheduler_test.rs:1886-1930` (`test_multi_target_halts_on_block`) for multi-target test setup: `setup_test_env()`, `make_in_progress_item()`, `MockAgentRunner::new()`, `phase_complete_result()`/`blocked_result()`, `RunParams` construction with `auto_advance: true`, `run_scheduler()` invocation, assert on `summary.items_completed`, `summary.items_blocked`, `summary.halt_reason`
- Follow `tests/scheduler_test.rs:1839-1884` (`test_multi_target_processes_in_order`) for successful multi-target completion patterns

**Tasks:**

- [ ] Add `test_auto_advance_skips_blocked_target`: two targets, first blocks, second completes. Assert `HaltReason::TargetCompleted`, `items_completed` contains second target, `items_blocked` contains first target
- [ ] Add `test_auto_advance_all_targets_blocked`: two targets, both block. Assert `HaltReason::TargetCompleted` (target list exhausted), `items_completed` is empty, `items_blocked` contains both targets
- [ ] Add `test_auto_advance_single_target_blocked`: single target with auto-advance, target blocks. Assert `HaltReason::TargetCompleted` (not `TargetBlocked`), `items_blocked` contains the target, `items_completed` is empty
- [ ] Add `test_auto_advance_circuit_breaker_not_tripped`: two targets where each target's mock returns failure results (not `blocked_result()`) that trigger retry exhaustion. `CIRCUIT_BREAKER_THRESHOLD` is 2, so configure `max_retries: 1` to produce 2 exhaustions per target (initial attempt + 1 retry). Assert halt reason is `TargetCompleted` (not `CircuitBreakerTripped`), both targets in `items_blocked`. This proves `consecutive_exhaustions` resets between auto-advanced targets.
- [ ] Add `test_auto_advance_backward_compat`: two targets WITHOUT `--auto-advance` (`auto_advance: false`), first blocks. Assert `HaltReason::TargetBlocked`, second target never processed
- [ ] Add `test_build_summary_deduplicates_items_blocked`: unit test that directly calls `build_summary()` with a `SchedulerState` containing duplicate entries in `items_blocked` (e.g., `["WRK-001", "WRK-002", "WRK-001"]`). Assert the resulting `RunSummary.items_blocked` contains each ID exactly once. This is a unit test because triggering duplicate pushes through the mock integration path is unreliable.

**Verification:**

- [ ] All new tests pass: `cargo test`
- [ ] All existing tests still pass (no regressions)
- [ ] Verify all 6 test functions are present: `test_auto_advance_skips_blocked_target`, `test_auto_advance_all_targets_blocked`, `test_auto_advance_single_target_blocked`, `test_auto_advance_circuit_breaker_not_tripped`, `test_auto_advance_backward_compat`, `test_build_summary_deduplicates_items_blocked`
- [ ] Code review passes (`/code-review` -> fix issues -> repeat until pass)

**Commit:** `[WRK-054][P2] Feature: Add tests for --auto-advance flag`

**Notes:**

The circuit breaker test is the most important: it validates that `consecutive_exhaustions` resets between auto-advanced targets. Use failure results (not `blocked_result()`) because `blocked_result()` resets `consecutive_exhaustions` to 0 in its handler, while failure results increment it. The test must produce enough failures per target to approach the `CIRCUIT_BREAKER_THRESHOLD` (currently 2), so that without the reset, the second target would trip the breaker. Configure `max_retries: 1` in the execution config so each target exhausts after 2 attempts (initial + 1 retry), incrementing `consecutive_exhaustions` by 1 each time the retry cap is hit.

The dedup test uses a unit test on `build_summary()` directly because: (a) triggering duplicate `items_blocked` pushes through the mock requires coordinating multiple code paths (guardrail rejection + retry exhaustion), and (b) the dedup logic is in `build_summary()` itself, so testing it directly is more targeted and reliable. The unit test constructs a `SchedulerState` with non-adjacent duplicates (e.g., `["WRK-001", "WRK-002", "WRK-001"]`) to verify that `sort(); dedup()` handles the non-adjacent case correctly.

**Followups:**

---

## Final Verification

- [ ] All phases complete
- [ ] All PRD Must Have criteria met:
  - [ ] `--auto-advance` flag accepted on `run` subcommand alongside `--target`
  - [ ] Blocked targets skipped with log and advance when flag active
  - [ ] Blocked targets remain Blocked in backlog
  - [ ] Run summary lists completed and blocked targets separately
  - [ ] Default halt-on-block behavior unchanged without flag
  - [ ] Flag accepted with single target (no error)
  - [ ] Blocked target state committed to git before advancing (`batch_commit()` called)
  - [ ] Circuit breaker counter reset when auto-advancing
  - [ ] Exit 0 when at least one target completed; exit 1 when all blocked
- [ ] PRD Should Have criteria:
  - [ ] Skip log message includes target ID, position counter, and "Auto-advancing" text
- [ ] Tests pass
- [ ] No regressions introduced
- [ ] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|

## Followups Summary

### Critical

### High

### Medium

- [ ] Config-file default for `--auto-advance` — deferred per PRD; involves `ExecutionConfig` schema changes, `Default` impl, and test fixtures
- [ ] `batch_commit()` call verification — no mock or spy exists for `coordinator.batch_commit()` in the test infrastructure, so the durability guarantee (blocked state committed before advancing) is verified only by code inspection, not by automated test. A `MockCoordinator` that records `batch_commit()` calls would enable asserting it is called once per auto-advanced target.
- [ ] Exit code path integration test — the exit code logic in `handle_run()` is above the `run_scheduler()` layer tested by integration tests. A CLI-level test (e.g., using `assert_cmd`) that asserts the process exit code is non-zero when all targets block would close this gap.

### Low

- [ ] Distinct summary messaging for all-blocked runs — PRD Should Have criterion for different messaging when `items_completed` is empty and `items_blocked` is non-empty. Currently the summary output is the same structure regardless; the exit code is the only differentiation.

## Design Details

### Key Types

```rust
// Modified struct (src/scheduler.rs)
pub struct RunParams {
    pub targets: Vec<String>,
    pub filter: Option<crate::filter::FilterCriterion>,
    pub cap: u32,
    pub root: PathBuf,
    pub config_base: PathBuf,
    pub auto_advance: bool,  // NEW — default false
}

// Modified enum variant (src/main.rs)
Run {
    #[arg(long, action = clap::ArgAction::Append)]
    target: Vec<String>,
    #[arg(long, conflicts_with = "target")]
    only: Option<String>,
    #[arg(long, default_value = "100")]
    cap: u32,
    #[arg(long, action = clap::ArgAction::SetTrue)]  // NEW
    auto_advance: bool,                                // NEW
}
```

### Architecture Details

The auto-advance branch is inserted at the existing block detection site in the scheduler loop:

```
Loop iteration:
  1. Shutdown check (line 554)
  2. Circuit breaker check (line 572) ← sees reset counter from prior auto-advance
  3. Inbox ingestion (line 590)
  4. Snapshot refresh (line 606)
  5. Block detection (line 609-631) ← AUTO-ADVANCE BRANCH HERE
     ├─ auto_advance=true: log, drain, commit, reset breaker, advance, continue
     └─ auto_advance=false: drain, commit, return TargetBlocked (existing)
  6. Target advancement past Done/pre-Blocked (line 634)
  7. Target exhaustion check (line 640)
  8. ... rest of loop (action selection, task spawning, result processing)
```

### Design Rationale

- **Conditional branch over loop restructure**: The block detection site is already the single decision point. Adding a conditional keeps logic co-located and avoids a high-risk refactor of the scheduler loop.
- **No new HaltReason variant**: `TargetCompleted` + non-empty `items_blocked` provides the same information without expanding match arms.
- **Circuit breaker reset**: Each target is independent; failures across targets should not accumulate. Industry implementations (Spring Batch, Argo) treat shared breaker state across independent items as a bug.
- **Binary exit code**: CI-compatible per Better CLI guide. Distinct codes (exit 2 for all-blocked) can be added as a follow-up if needed.
- **Dedup in build_summary()**: Raw `items_blocked` vec can have duplicates during the run (useful for debugging). Only the user-facing `RunSummary` is deduplicated.

## Assumptions

Decisions made without human input:

- **Mode: light** — This is a small, low-complexity change with clear design. Two phases: production code + tests.
- **Two phases, not five** — The dependency analysis suggested 5 phases, but the change is only ~70 lines of production code across 2 files. Splitting into more phases would create artificially small units. Phase 1 (all production code + test fixup) and Phase 2 (new tests) is the natural boundary.
- **Test fixup in Phase 1** — Updating existing `RunParams` struct literals to include `auto_advance: false` is part of Phase 1 (not Phase 2) because the new field breaks compilation of existing tests, and Phase 1's verification requires `cargo test` to pass.
- **Exit code logic is mode-agnostic** — The exit code check (`items_completed.is_empty() && !items_blocked.is_empty()`) applies to all target-mode runs, not just `--auto-advance` runs. This is an intentional behavior change: currently `handle_run()` always exits 0 after printing the summary, even when all targets blocked. The Design doc's exit code decision matrix supports this.
- **No `conflicts_with` on `--auto-advance`** — The flag is silently ignored in non-target modes (filter mode, no-targets mode) per PRD constraints. Adding `conflicts_with = "only"` would error on `--auto-advance --only`, which is unnecessarily restrictive.
- **Duplicate target IDs already handled** — The Design doc resolved this: `handle_run()` (lines 389-401) detects duplicates at parse time using a `HashSet` and rejects the run with an error. No additional handling needed.
- **Dedup test as unit test** — The `items_blocked` deduplication test uses `build_summary()` directly rather than going through the integration test path, because triggering duplicate pushes through the mock requires coordinating multiple code paths that are unreliable to set up.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
