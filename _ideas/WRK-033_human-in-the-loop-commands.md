# WRK-033: Add Human-in-the-Loop Commands for Non-Autonomous Item Management

## Problem Statement

The orchestrator currently has two interaction modes: fully autonomous (`orchestrate run`) and basic manual manipulation (`unblock`, `advance`). Three common workflows have no supported path:

1. **Human-in-the-loop execution** — When an item is blocked by guardrails (e.g., risk exceeds `max_risk`), a human who has reviewed the risk and wants to proceed has no way to say "run one phase at a time with my approval between steps." The only options are raising guardrails globally, manually editing YAML, or blindly unblocking.

2. **Human-completed work** — When a human does work outside the orchestrator (writes code manually, runs their own build), there's no way to tell the orchestrator the item is done or should skip phases. Requires manual YAML editing.

3. **Item cancellation** — When work is no longer needed, there's no `close` or `cancel` command. Items linger in the backlog indefinitely.

## Proposed Approach

Add 3-4 new CLI commands following existing patterns (`advance`, `unblock`):

### Commands

1. **`orchestrate step <ID>`** — Run exactly one phase for the item, bypassing guardrail checks, then stop. Works on blocked items by implicitly unblocking. Human reviews output before running `step` again. This is the primary human-in-the-loop mechanism.

2. **`orchestrate complete <ID>`** — Mark an item as Done. Accepts a `--summary` flag or prompts for what was done. Transitions item to Done status, writes worklog entry, commits to git.

3. **`orchestrate close <ID> --reason "..."`** — Archive an item with a "closed" notation (distinct from "done"). Removes from active backlog. Useful for cancelling work that's no longer needed.

4. **`--override-guardrails` flag on `unblock`** (optional) — Sets a per-item guardrail exception that persists through re-triage, so the item doesn't re-block on the same threshold.

### Implementation Scope

**Files to modify:**
- `main.rs` — Add clap subcommands for `step`, `complete`, `close`; add handler functions
- `types.rs` — Possibly add `Closed` status variant or a closed flag; add guardrail override field to BacklogItem
- `backlog.rs` — Add `close_item()` function, update `transition_status()` if new status added
- `coordinator.rs` — Add coordinator commands if step needs to interact with running orchestrator
- `worklog.rs` — Handle worklog entries for closed/completed items
- `executor.rs` — `step` command needs single-phase execution with guardrail bypass

**Patterns to follow:**
- Existing `handle_advance()` and `handle_unblock()` in main.rs for command structure
- `backlog::save()` for atomic persistence
- `git::stage_paths()` + `git::commit()` for git integration (unlike existing advance/unblock which don't commit)

### Edge Cases to Consider

- `step` on an item that's mid-phase (agent currently running) — should reject or wait?
- `complete` on an item that has in-flight work — should cancel the agent?
- `close` on an item with dependents — should warn or block?
- `step` advancing through destructive phases — should still enforce exclusive execution?
- Interaction between `step` and a concurrently running `orchestrate run` — likely should require the orchestrator not be running (check lock file)

## Assessment

| Dimension  | Rating | Rationale |
|------------|--------|-----------|
| Size       | Large  | 3-4 new CLI commands, each with state transitions, git commits, and edge cases. Touches 5+ files. |
| Complexity | Medium | Each command follows existing patterns, but they must compose with existing status transitions, the coordinator actor, and each other. The `step` command is the most complex as it involves single-phase execution with guardrail bypass. |
| Risk       | Low    | All commands are additive. No existing behavior changes. Default autonomous mode is unaffected. |
| Impact     | High   | Completes the human-orchestrator interaction model. Without it, blocked items are dead ends, externally completed work can't be recorded, and items can't be cancelled. |

## Assumptions

- The `step` command should NOT work while `orchestrate run` is active (use existing lock file to enforce this). It runs a single phase synchronously, similar to how `advance` works but with actual execution.
- `Closed` should be a distinct terminal status (like `Done`) rather than reusing `Done` with metadata, to keep the semantic distinction clear.
- Each command should commit its changes to git (unlike current `advance`/`unblock` which only save to disk).
- The `--override-guardrails` flag is lower priority and could be deferred to a follow-up item to reduce scope.

## Recommendation

This item is well-scoped with a clear description. The large size suggests it may benefit from being split into 2-3 smaller items (e.g., `step` as one item, `complete`+`close` as another, guardrail overrides as a third). However, the commands are conceptually related and designing them together ensures they compose well.

Recommend proceeding through the feature pipeline. The `step` command is the highest-value piece and could be prioritized if splitting.
