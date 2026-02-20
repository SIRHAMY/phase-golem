# WRK-075: Auto Re-Triage Stale Pre-Phase Items After Configurable Idle Period

## Problem Statement

Items that enter the `Scoping` status (pre-phase execution) can sit idle indefinitely if they get blocked or deprioritized. There is no mechanism to detect that an item has been stuck in pre-phase work for an extended period and should be re-evaluated. This can lead to backlog items silently stalling without visibility.

## Current State

- **Timestamps:** `BacklogItem` has `created` and `updated` fields, but `updated` is generic (refreshed on any mutation) and doesn't track when an item entered a specific phase or status.
- **Existing staleness:** The codebase has artifact-level staleness detection (git history divergence for destructive phases in `executor.rs`), but no item-level idle detection.
- **Scheduler:** Items in `New` status get triaged once. After triage, items in `Scoping` proceed through pre-phases until completion or blocking. No re-triage path exists.

## Proposed Approach

### 1. Add Phase Entry Timestamp
Add `phase_entered_at: Option<String>` (RFC3339) to `BacklogItem` in `src/types.rs`. Set it whenever:
- Status transitions to `Scoping` (via `transition_status()`)
- A new phase begins within `Scoping`

### 2. Add Configuration
Add `retriage_idle_period_days: Option<u32>` to `ExecutionConfig` in `src/config.rs`. When `None`, feature is disabled (opt-in).

### 3. Scheduler Detection
In `select_actions()` in `src/scheduler.rs`:
- Check items with `status == Scoping` where `(now - phase_entered_at) > idle_period`
- Generate a re-triage action that resets the item to `New` or runs the triage phase again
- Consider whether re-triaged items should be marked differently (e.g., `retriage_count` field)

### 4. Re-Triage Behavior
Options to decide during design:
- **Reset to New:** Simple — item goes back through triage as if newly added
- **Dedicated re-triage phase:** More complex — could provide context about why the item stalled
- **Should `Ready` items also be re-triaged?** Or only `Scoping` items?

## Files Likely Affected

1. `src/types.rs` — Add `phase_entered_at`, possibly `retriage_count`
2. `src/config.rs` — Add `retriage_idle_period_days` to `ExecutionConfig`
3. `src/scheduler.rs` — Idle detection and re-triage action generation
4. `src/backlog.rs` — Update `transition_status()` to set `phase_entered_at`
5. Tests for all of the above

## Assessment

| Dimension  | Rating | Rationale |
|------------|--------|-----------|
| Size       | Medium | 4-5 files + tests |
| Complexity | Medium | Design decisions around re-triage semantics, what "stale" means per status |
| Risk       | Medium | Modifies `BacklogItem` (widely used type) and scheduler logic (core orchestration) |
| Impact     | Medium | Prevents silent item stalling; useful operational improvement but not blocking |

## Assumptions

- The feature should be opt-in (disabled by default) to avoid surprising existing users.
- Only `Scoping` items are candidates for re-triage initially (not `Ready` or `Blocked`).
- Re-triage resets the item to `New` status rather than introducing a separate re-triage phase.
- A backlog migration will be needed to add the new `phase_entered_at` field (nullable, so backward-compatible).

## Open Questions (for human review)

1. Should `Ready` items that have been idle also be re-triaged, or only `Scoping`?
2. Should there be a maximum retriage count to prevent infinite loops?
3. Should re-triage produce a different prompt/context than initial triage (e.g., "this item was previously triaged N days ago and stalled")?
