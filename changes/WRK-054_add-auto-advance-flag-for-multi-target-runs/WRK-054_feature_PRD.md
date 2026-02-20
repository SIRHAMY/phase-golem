# Change: Add --auto-advance flag for multi-target runs

**Status:** Proposed
**Created:** 2026-02-19
**Author:** phase-golem (autonomous)

## Problem Statement

When running `phase-golem run --target WRK-005 --target WRK-010 --target WRK-003`, the orchestrator processes targets sequentially. If any target becomes Blocked — whether from a guardrail rejection (a block imposed by a safety or policy check), an agent crash, or retry exhaustion — the entire run halts with a `TargetBlocked` halt reason. The remaining targets sit untouched until the user manually intervenes and re-invokes with a corrected target list.

This creates a mismatch between user intent and system behavior. Specifying multiple targets expresses "I want these things done," not "stop everything if one gets stuck." The halt-on-block behavior was a correct conservative MVP default (from WRK-030, the multi-target support feature, already shipped), but for batch-style multi-target runs it forces unnecessary babysitting proportional to the block rate, not to the amount of work requiring human judgment.

The real-world backlog demonstrates this pain: 7+ items are currently in Blocked state, many from infrastructure failures (e.g., "Result file not found" when an agent crashes before writing output). A user running a target list containing any of these would halt immediately and repeatedly.

## User Stories / Personas

- **Solo developer running batch work** — Has 3-10 items to progress while doing other work. Wants to queue them up and come back to results. Currently, if item 1 of 5 blocks, items 2-5 get zero processing time until the human re-invokes.

- **Developer with mixed-risk target lists** — Knows some targets may block (new features hitting guardrails) while others are straightforward (small fixes). Wants the straightforward items to complete regardless of whether risky items block.

## Desired Outcome

When a user passes `--auto-advance` with a multi-target run, the orchestrator skips blocked targets and continues processing the next target in the list instead of halting. At the end of the run, the summary clearly reports which targets completed and which were blocked. Blocked targets remain in their Blocked state in the backlog for later resolution (users can consult the backlog YAML for detailed block reasons per item).

The user can queue a batch of targets, walk away, and return to find all non-blocked targets processed — with clear reporting of what succeeded and what needs attention.

## Success Criteria

### Must Have

- [ ] `--auto-advance` flag accepted on the `run` subcommand alongside `--target`
- [ ] When active and a target blocks at runtime, the orchestrator logs the skip and advances to the next target instead of halting
- [ ] Blocked targets remain Blocked in the backlog (no auto-unblocking)
- [ ] Run summary lists completed targets and blocked targets separately
- [ ] Existing halt-on-block behavior is unchanged when `--auto-advance` is not passed (backward compatible)
- [ ] The flag is accepted without error when used with a single target; if that target blocks, the run ends normally (target list exhausted) rather than halting with `TargetBlocked`
- [ ] The blocked target's state is committed to git before advancing to the next target (ensures durability if a subsequent crash occurs)
- [ ] The circuit breaker counter (`consecutive_exhaustions`) is reset when auto-advancing past a blocked target, so that failures across independent targets do not accumulate and trip the circuit breaker
- [ ] Process exits 0 when at least one target completed; exits non-zero when all targets blocked and none completed

### Should Have

- [ ] Log message at skip time identifies which target blocked and which target is next (e.g., `[target] WRK-005 blocked (1/3). Auto-advancing to WRK-010.`)
- [ ] Run summary output distinguishes "all targets completed successfully" from "all targets blocked" (e.g., different messaging or summary line when `items_completed` is empty and `items_blocked` is non-empty)

### Nice to Have

- [ ] Config-file default for auto-advance — deferred to a follow-up item; involves schema changes to `ExecutionConfig`, `Default` impl, and test fixtures

## Scope

### In Scope

- Adding `--auto-advance` CLI flag to the `run` subcommand
- Conditional skip logic at the existing block-detection site in the scheduler (lines 608-631)
- Threading the flag through `RunParams` (including CLI enum variant, `handle_run` body, and `RunParams` struct)
- Resetting the circuit breaker counter on auto-advance
- Git commit of blocked target state before advancing
- Updated summary/log messaging for the auto-advance path
- Integration tests for the new behavior

### Out of Scope

- Interaction with `--only` filter mode (filter mode already continues past blocked items via its own mechanism; `--auto-advance` is specific to `--target` mode)
- Auto-unblocking of items (flag skips blocked targets; it does not change their status)
- Parallel target processing (this is sequential cursor advancement only)
- Retry-before-advance policy (would complicate interaction with existing circuit breaker and `max_retries`)
- Making auto-advance the default behavior (backward compatibility preserved; can be revisited as a separate item)
- Changes to `advance_to_next_active_target()` function — this function handles pre-existing Blocked items (items already Blocked before the run starts) by checking the backlog snapshot status. Runtime blocks (items that become Blocked during the current run) are tracked separately via `items_blocked` in `SchedulerState`. The auto-advance logic operates on runtime blocks only.
- Surfacing detailed block reasons in the run summary (block reasons are stored on the backlog item's `blocked_reason` field; the summary reports target IDs only)

## Non-Functional Requirements

- **Performance:** The change introduces no additional I/O, network calls, or data structures beyond a single boolean field on `RunParams` and a conditional branch at the existing halt point
- **Observability:** Each skip is logged with target ID, position in list, and the target being advanced to

## Constraints

- Must not change default behavior — existing multi-target runs without `--auto-advance` must halt on block as before
- Must use existing `items_blocked` tracking in `SchedulerState` — no new state tracking needed
- The flag only applies to `--target` mode. Since `--only` already has `conflicts_with = "target"` in the CLI definition, `--auto-advance` without any `--target` arguments is silently ignored (the flag has no effect in non-target scheduling modes)

## Dependencies

- **Depends On:** WRK-030 (multi-target support) — already shipped
- **Blocks:** Nothing

## Risks

- [ ] Circuit breaker sequencing: The circuit breaker check runs at the top of the scheduler loop (line 572), before the block-detection / auto-advance branch (line 609). If a target exhausts retries and increments `consecutive_exhaustions`, the reset must happen inside the auto-advance branch on the same loop iteration — before the next iteration's circuit breaker check fires. Mitigation: implementation must reset `consecutive_exhaustions` immediately when taking the auto-advance branch, and include a test case verifying two consecutively retry-exhausted targets with `--auto-advance` produce `TargetCompleted` (not `CircuitBreakerTripped`).
- [ ] Ambiguous halt reason: when all targets are blocked under `--auto-advance`, the run exits via `TargetCompleted` (target list exhausted). The existing `items_completed` and `items_blocked` lists in the run summary provide the data to distinguish this from a fully successful run. The Should Have criterion covers adding distinct summary messaging for this case.

## Open Questions

- [ ] Should duplicate target IDs in the target list (e.g., `--target WRK-005 --target WRK-005`) be deduplicated at parse time, or allowed to pass through? The existing `advance_to_next_active_target()` will silently skip the second occurrence based on snapshot status (Done or Blocked), but the user may find fewer summary entries than expected.
- [ ] Should `items_blocked` in `RunSummary` be deduplicated? Multiple code paths can push the same item ID (guardrail rejection and retry exhaustion for the same target). The summary should list each target at most once.

## Assumptions

Decisions made without human input:

- **PRD depth: medium** — This is a well-understood enhancement explicitly deferred from WRK-030. The scope is small and clear, but edge cases (circuit breaker, halt reason) warrant moderate exploration. "Medium" refers to the PRD creation workflow depth level.
- **Flag not default** — Preserving backward compatibility per WRK-030's explicit design decision. Can be revisited as a separate item.
- **Single-target is a no-op** — Accepting `--auto-advance` with a single target silently rather than erroring, to avoid annoying users who have it set in scripts or aliases. If the single target blocks, the run ends via normal target-list exhaustion.
- **No interaction with `--only`** — The flag is silently ignored in non-target modes since filter mode already handles blocks by continuing past them.
- **Circuit breaker reset decided** — Reset `consecutive_exhaustions` when auto-advancing. Each target is independent work; consecutive failures across different targets should not accumulate into a circuit breaker trip.
- **Join set drain not needed** — Target mode uses `max_wip=1`, so there are no in-flight tasks when a block is detected. The existing loop structure handles advancement without explicit drain.
- **Exit code specified** — Exit 0 for partial success (at least one target completed), non-zero for total failure (all targets blocked). This matches standard batch processing conventions.

## References

- WRK-030 PRD: `changes/WRK-030_add-target-priority-list-and-only-flag-for-orchestrator-run/WRK-030_feature_PRD.md`
- WRK-030 Design: `changes/WRK-030_add-target-priority-list-and-only-flag-for-orchestrator-run/WRK-030_feature_DESIGN.md`
- Block detection code: `src/scheduler.rs` lines 608-631
- Target advancement: `src/scheduler.rs` lines 468-510
- Circuit breaker check: `src/scheduler.rs` line 572
- `RunParams` struct: `src/scheduler.rs` lines 49-59
- CLI argument definition: `src/main.rs` lines 56-66
