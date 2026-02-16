# WRK-033 Research Summary: Human-in-the-Loop Commands

## What This Involves

Add 3 new CLI subcommands (`step`, `complete`, `close`) and one flag enhancement (`--override-guardrails` on `unblock`) to the orchestrator binary. These commands give humans explicit control over items that can't proceed autonomously — blocked-by-guardrails items, externally-completed work, and cancelled items.

## Key Files

**Must modify:**
- `orchestrator/src/main.rs` — Add 3 new `Commands` enum variants (`Step`, `Complete`, `Close`) with their handler functions, plus `--override-guardrails` flag on `Unblock`
- `orchestrator/src/types.rs` — Add `Closed` variant to `ItemStatus` (or reuse archival pattern), potentially add `guardrail_override: bool` field to `BacklogItem`
- `orchestrator/src/backlog.rs` — Add `close_item()` function (archive with "closed" notation vs "done"), handle `complete` as external-done
- `orchestrator/src/coordinator.rs` — No changes needed for `step` (reuses existing `execute_phase`); may need new command for `close`
- `orchestrator/src/worklog.rs` — Handle "Closed" and "Completed externally" outcomes in worklog entries
- `orchestrator/src/executor.rs` — `step` reuses `execute_phase()` directly; `passes_guardrails()` needs to check per-item override flag

**May modify:**
- `orchestrator/src/scheduler.rs` — `apply_triage_result()` should respect per-item guardrail overrides so items don't re-block on triage
- `orchestrator/src/config.rs` — No changes expected (guardrail override is per-item, not config)

**Tests:**
- `orchestrator/tests/` — New tests for each command and edge cases

## Approach Sketch

1. **`orchestrate step <ID>`**: Load config/backlog, find item, determine current phase (or first phase if not yet started). If blocked, implicitly unblock. Call `execute_phase()` synchronously for exactly one phase, apply transitions, commit, and exit. Bypasses guardrail checks. Does NOT use the scheduler loop — directly executes like `handle_triage` does.

2. **`orchestrate complete <ID> [--summary "..."]`**: Transition item to Done status, write worklog entry with "Completed externally" outcome, archive item from backlog. Similar to what happens at the end of `handle_phase_success` when transitioning to Done, but without running any phase.

3. **`orchestrate close <ID> --reason "..."`**: Remove item from backlog with a "Closed" worklog entry (distinct from "Done"). Does not transition through Done — directly archives with a "closed" notation. Reason is required to document why.

4. **`--override-guardrails` on `unblock`**: Sets a per-item flag (`guardrail_override: bool`) that `passes_guardrails()` checks. When true, the item bypasses guardrail threshold checks during triage routing and pre-phase completion. The flag persists across re-triage.

## Risks or Concerns

- **`step` on item mid-phase with running orchestrator**: The lock file prevents concurrent `orchestrate run` + `step`. This is safe — `step` acquires the lock just like `run` does.
- **`complete` on item with in-flight work**: Same lock protection. If the orchestrator is running, `complete` will fail to acquire the lock. This is the correct behavior.
- **`close` on item with dependents**: Should warn or error if other items depend on the closed item (their dependencies would never be met).
- **Status transition for `close`**: `Done` is currently terminal and triggers archival. `close` should probably bypass `is_valid_transition()` and go directly to removal, or add a `Closed` status. Simplest approach: skip status transition entirely, just remove from backlog and write worklog.
- **`step` status management**: If the item is `New`, `step` needs to triage first then run the first pre-phase. Or it could require the item to already be triaged. The simpler approach is to require at least `Scoping` status — if `New`, run triage as the "step".

## Assessment

| Dimension | Rating | Justification |
|-----------|--------|---------------|
| **Size** | Large | 3-4 new commands, each with handler function, state transitions, git commit, worklog, and edge case handling. ~11+ files touched including tests. |
| **Complexity** | Medium | Each command individually is straightforward (following existing `advance`/`unblock` patterns), but they must compose correctly with status transitions, the lock, and the scheduler's state model. |
| **Risk** | Low | All commands are additive. They don't change autonomous behavior — they only add new entry points for human interaction. Lock file prevents concurrent access issues. |
| **Impact** | High | Completes the human-orchestrator interaction model. Without these commands, items blocked by guardrails are dead ends, externally-completed work requires manual YAML editing, and there's no way to cancel items. |

## Assumptions

- `step` will acquire the orchestrator lock, preventing concurrent use with `orchestrate run`. This is intentional — human step-by-step execution is an alternative to autonomous execution, not concurrent with it.
- `close` removes the item from `BACKLOG.yaml` entirely (like archive) rather than keeping it with a `Closed` status. This keeps the backlog clean and follows the existing `archive_item` pattern.
- The `--override-guardrails` flag on `unblock` is the lowest-priority feature and could be deferred to a follow-up if the main 3 commands take longer than expected.
