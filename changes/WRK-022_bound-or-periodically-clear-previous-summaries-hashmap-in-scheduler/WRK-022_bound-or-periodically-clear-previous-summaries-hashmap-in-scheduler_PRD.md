# Change: Bound or Periodically Clear previous_summaries HashMap in Scheduler

**Status:** Proposed
**Created:** 2026-02-13
**Author:** Orchestrator (autonomous)

## Definitions

- **Terminal state:** An item state from which no further phase transitions occur. The two terminal states are Done (all phases completed) and Blocked (item cannot proceed without manual intervention).
- **Active item:** An item currently progressing through the phase pipeline (status is InProgress, Ready, or Scoping — any non-terminal state).
- **OOM:** Out-of-memory — process exhausts available memory and is killed by the OS.
- **WIP:** Work-in-progress — the number of items concurrently being processed by the scheduler (controlled by `max_wip` config).
- **LRU cache:** Least-recently-used cache — a bounded data structure that evicts the least recently accessed entries when capacity is reached.

## Problem Statement

The scheduler's `previous_summaries: HashMap<String, String>` in `run_scheduler()` grows unboundedly because entries are inserted on every phase/subphase completion but never removed. Each completed phase adds an entry mapping item ID to a summary string. Items that reach terminal states (Done or Blocked) will never be read from again, yet their summaries persist in memory for the entire scheduler lifetime.

At current scale (~50 items, `max_wip=1`), the practical impact is negligible (~60KB). However, this is a code smell with a clear scaling risk: a long-running session processing hundreds of items across multiple phases could accumulate significant memory overhead for no functional benefit. The unbounded growth pattern should be fixed proactively before the memory accumulation becomes problematic.

## User Stories / Personas

- **Orchestrator operator** - Runs the orchestrator for extended sessions processing large backlogs (up to hundreds of items). Expects the process to handle any backlog size without out-of-memory crashes or degraded performance over time.

## Desired Outcome

After this change, the `previous_summaries` HashMap only retains summaries for active items (those still progressing through phases). Summaries for items that have reached terminal states (Done, Blocked) are removed immediately upon state transition. The HashMap size stays proportional to the number of active items, not the total number of items ever processed.

## Success Criteria

### Must Have

- [ ] Summaries are removed from `previous_summaries` when an item transitions to Done status
- [ ] Summaries are removed from `previous_summaries` when an item transitions to Blocked status
- [ ] Active items still receive their previous phase summary in the agent prompt (no functional regression)
- [ ] Items that fail a phase and retry (within `max_retries`) retain their summary throughout the retry cycle
- [ ] All existing tests pass without modification
- [ ] After processing N items to completion, HashMap contains at most `max_wip` entries (not N entries)

### Should Have

- [ ] DEBUG-level log message when the HashMap exceeds 100 entries, for observability
- [ ] Unit or integration test verifying that summaries are cleaned up on terminal state transitions

### Nice to Have

- [ ] Periodic defensive sweep removing summaries for item IDs not present in the current backlog snapshot

## Scope

### In Scope

- Removing `previous_summaries` entries when items reach Done or Blocked status
- Adding cleanup calls in `handle_phase_success()` (when item transitions to Done after final phase) and `handle_phase_blocked()` / `handle_phase_failed()` (when item becomes Blocked)
- Adding `previous_summaries: &mut HashMap<String, String>` parameter to `handle_phase_failed()` and `handle_phase_blocked()` (these handlers do not currently receive this parameter)
- Updating call sites of modified handlers to pass the HashMap reference
- Basic observability logging for HashMap size
- Cleanup must occur as the final operation in each handler, after all coordinator status updates have completed successfully

### Out of Scope

- Replacing HashMap with LRU cache or other bounded data structure (unnecessary given lifecycle-based cleanup)
- Persisting summaries to disk or sharing across scheduler invocations (the scheduler runs as a single session per `orchestrate run` invocation; summaries are ephemeral)
- Adding configurable size limits or TTL-based eviction
- Truncating or bounding individual summary string lengths
- Moving `previous_summaries` into `SchedulerState` or restructuring scheduler data model
- Cross-phase summary chaining (storing multiple phase summaries per item)

## Non-Functional Requirements

- **Performance:** Cleanup adds O(1) `.remove()` call per terminal state transition. No measurable performance impact.
- **Observability:** DEBUG-level log of HashMap size helps diagnose any future memory concerns.

## Context: Summary Mechanisms

The orchestrator uses three distinct context mechanisms for passing information between phases. This change only affects the first:

1. **`previous_summaries`** (this change): Summaries from successfully completed phases/subphases, passed to the next phase execution. Lifetime: from phase completion until item reaches a terminal state.
2. **`failure_context`**: Failure messages from retry attempts within a single phase execution. Local to the executor's retry loop, not persisted. Not affected by this change.
3. **`unblock_context`**: User-provided notes when manually unblocking an item. Stored on `BacklogItem`, cleared on status transition. Not affected by this change.

## Constraints

- The HashMap is a local variable in `run_scheduler()`, passed by mutable reference to handlers. Cleanup must happen within this existing structure.
- Only terminal states (Done, Blocked) are safe cleanup points. Removing summaries for items that may retry or continue would break the context chain.
- Summary availability is optional in the executor path (`Option<&str>`), so missing summaries degrade gracefully — but we should not remove summaries for active items.
- Summaries are `.cloned()` before being passed to spawned async tasks (line 620), so cleanup cannot affect in-flight phase executions.

## Dependencies

- **Depends On:** Understanding of item lifecycle (which handler triggers Done vs. Blocked transitions)
- **Blocks:** Nothing

## Risks

- [ ] **Premature removal during retries:** If an item fails and retries, the previous summary should still be available. Mitigation: Only remove on terminal Done/Blocked, not on transient failures that lead to retries. The retry loop uses `failure_context` (separate mechanism), not `previous_summaries`.
- [ ] **Race between cleanup and read:** The scheduler is single-threaded for HashMap access (async but not multi-threaded), so no concurrent mutation risk exists. Summaries are cloned before being passed to spawned tasks.

## Assumptions

- The backlog description identifies this as the primary OOM risk for long-running sessions. The actual risk at current scale is low, but the fix requires minimal implementation effort and prevents future issues.
- The `handle_phase_success` handler already knows when an item reaches Done (it calls `coordinator.archive_item()`), making it the natural cleanup point.
- The `handle_phase_failed` and `handle_phase_blocked` handlers already know when items become Blocked, making them natural cleanup points as well.
- Item IDs are unique within a scheduler session (enforced by backlog validation). No ID reuse occurs during a single run.
- When a Blocked item is unblocked and re-enters the pipeline, it generates a fresh summary on its next phase completion. The `unblock_context` field (separate from `previous_summaries`) provides continuity context for the agent prompt.

## References

- `scheduler.rs` — HashMap definition (line 481), insert sites (lines 904, 951), read site (line 620)
- `executor.rs` — Summary passed through `execute_phase()` as `Option<&str>`
- `prompt.rs` — Summary rendered as `## Previous Phase Summary` in agent prompt
