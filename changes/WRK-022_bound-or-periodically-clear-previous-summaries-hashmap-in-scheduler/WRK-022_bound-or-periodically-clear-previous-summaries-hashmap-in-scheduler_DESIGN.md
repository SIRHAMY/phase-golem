# Design: Bound previous_summaries HashMap in Scheduler

**ID:** WRK-022
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-022_bound-or-periodically-clear-previous-summaries-hashmap-in-scheduler_PRD.md
**Tech Research:** ./WRK-022_bound-or-periodically-clear-previous-summaries-hashmap-in-scheduler_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add `HashMap::remove()` calls at every terminal state transition (Done, Blocked) so that `previous_summaries` only retains entries for active items. The cleanup logic is centralized in a single helper function (`cleanup_terminal_summary`) called from all terminal transition points. The change touches four handler functions in `scheduler.rs` — modifying `handle_phase_success` to conditionally remove instead of insert, and passing `previous_summaries` into `handle_phase_failed` and `handle_phase_blocked` so they can also clean up via the helper. A DEBUG-level log is added when the HashMap exceeds `max_wip * 20` entries.

Note: `handle_triage_success` is not modified because triage is always the first phase for an item — no `previous_summaries` entry exists at triage time, so there is nothing to clean up even if triage sets Blocked.

---

## System Design

### High-Level Architecture

No new components. The change is entirely within the existing `scheduler.rs` module. The `previous_summaries` HashMap remains a local variable in `run_scheduler()`, passed by `&mut` reference to handler functions. The only architectural change is extending the parameter list of two handlers, adding a centralized cleanup helper, and calling it at terminal transition points.

**Handler contract:** Any handler that can transition an item to a terminal state (Done or Blocked) MUST receive `previous_summaries: &mut HashMap<String, String>` and call the cleanup helper after the transition. This contract ensures future terminal-state handlers don't accidentally omit cleanup.

### Component Breakdown

#### `cleanup_terminal_summary` (new helper function)

**Purpose:** Centralized cleanup of `previous_summaries` entries when an item reaches a terminal state.

**Signature:** `fn cleanup_terminal_summary(item_id: &str, previous_summaries: &mut HashMap<String, String>)`

**Behavior:** Calls `previous_summaries.remove(item_id)`. This is a no-op if the item has no entry (safe for items that reach terminal state on their first phase). All three terminal transition paths call this single function, ensuring cleanup logic is maintained in one place.

**Rationale:** Centralizing cleanup avoids inconsistency across handlers and makes future changes (e.g., adding logging or metrics on cleanup) require modification in only one place.

#### `handle_phase_success` (existing, modified)

**Purpose:** Processes successful phase completions and resolves state transitions.

**Changes:**
- After the `for update in updates` loop that processes transitions, the function currently unconditionally inserts the summary at line 1021. Instead: track whether the item reached a terminal state using a `let mut is_terminal = false;` flag, set to `true` inside the loop's `TransitionStatus(Done)` and `SetBlocked(_)` match arms. After the loop, if `is_terminal` is true, call `cleanup_terminal_summary(item_id, previous_summaries)`. Otherwise, insert the summary as before.

**Rationale:** This handler already has access to `previous_summaries` and already knows whether the item reached Done or Blocked from the `updates` loop. The boolean flag is set within existing match arms, requiring minimal code change.

#### `handle_phase_failed` (existing, modified)

**Purpose:** Handles phase execution failures (retries exhausted), transitions item to Blocked.

**Changes:**
- Add `previous_summaries: &mut HashMap<String, String>` parameter.
- Call `cleanup_terminal_summary(item_id, previous_summaries)` as the final operation after setting Blocked status.

#### `handle_phase_blocked` (existing, modified)

**Purpose:** Handles phase-reported blocks, transitions item to Blocked.

**Changes:**
- Add `previous_summaries: &mut HashMap<String, String>` parameter.
- Call `cleanup_terminal_summary(item_id, previous_summaries)` as the final operation after setting Blocked status.

#### `handle_task_completion` (existing, modified)

**Purpose:** Dispatches completion results to the appropriate handler.

**Changes:**
- Pass `previous_summaries` to `handle_phase_failed` and `handle_phase_blocked` call sites (lines 914, 917).

#### Observability (new logic, no new component)

**Purpose:** Warn if HashMap grows unexpectedly large.

**Changes:**
- After each `previous_summaries.insert()` call (in `handle_phase_success` and `handle_subphase_complete`), check if `previous_summaries.len() > max_wip * 20` and emit a `log_debug!()` message. The threshold is derived from `max_wip` so it scales with configuration rather than being a magic number. At `max_wip=5`, this triggers at 100 entries; at `max_wip=1`, it triggers at 20.

### Data Flow

1. Phase completes successfully → `handle_phase_success` resolves transitions
2. If transition is Done or Blocked → `previous_summaries.remove(item_id)` (no insert)
3. If transition is to next phase → `previous_summaries.insert(item_id, summary)` as before
4. Phase fails (retries exhausted) → `handle_phase_failed` sets Blocked → `previous_summaries.remove(item_id)`
5. Phase reports blocked → `handle_phase_blocked` sets Blocked → `previous_summaries.remove(item_id)`

### Key Flows

#### Flow: Item completes all phases (Done)

> An item finishes its final phase and transitions to Done.

1. **Phase completes** — `handle_phase_success` receives the result
2. **Resolve transitions** — `resolve_transition` returns `TransitionStatus(Done)`
3. **Apply updates** — coordinator archives the item, state tracks completion
4. **Cleanup summary** — `previous_summaries.remove(item_id)` instead of insert
5. **Result** — HashMap entry removed; no stale data remains

**Edge cases:**
- Item has no previous summary entry (first phase was Done) — `remove()` is a no-op, safe

#### Flow: Item blocked by phase failure

> A phase fails after exhausting retries, transitioning the item to Blocked.

1. **Phase fails** — `handle_phase_failed` receives the failure reason
2. **Set Blocked** — coordinator updates item status
3. **Cleanup summary** — `previous_summaries.remove(item_id)`
4. **Result** — HashMap entry removed

**Edge cases:**
- Item had no previous summary (failed on first phase) — `remove()` is a no-op, safe

#### Flow: Item blocked by phase report

> A phase explicitly reports that it's blocked (needs human input).

1. **Phase reports blocked** — `handle_phase_blocked` receives the block reason
2. **Set Blocked** — coordinator updates item status
3. **Cleanup summary** — `previous_summaries.remove(item_id)`
4. **Result** — HashMap entry removed

#### Flow: Subphase completes (non-terminal re-entry, no cleanup)

> A subphase completes; the item will re-enter the same phase. Subphases do not transition items to terminal states — they represent re-entry into the same phase, so cleanup does not occur here by design.

1. **Subphase completes** — `handle_subphase_complete` receives the result
2. **Insert summary** — `previous_summaries.insert(item_id, summary)` (unchanged)
3. **Result** — Summary available for the next execution of the same phase

#### Flow: Item unblocked and re-blocked

> An item was previously Blocked (summary removed), is manually unblocked, re-enters the pipeline, then becomes Blocked again.

1. **Item blocked** — summary removed via `cleanup_terminal_summary`
2. **Item unblocked** — re-enters pipeline; no summary exists (removed in step 1). The `unblock_context` field (separate mechanism) provides continuity context for the agent prompt
3. **Item completes a phase** — new summary inserted via `previous_summaries.insert()`
4. **Item blocked again** — summary removed via `cleanup_terminal_summary`
5. **Result** — lifecycle is correct; each block/unblock cycle manages its own summary independently

---

## Technical Decisions

### Key Decisions

#### Decision: Remove-on-terminal vs. conditional-insert

**Context:** Two implementation approaches exist for `handle_phase_success`: (A) track whether the item reached a terminal state and skip the insert + call remove, or (B) always insert then remove in the terminal branches.

**Decision:** Track terminal state with a boolean flag, then conditionally insert or remove after the update loop.

**Rationale:** Avoids inserting then immediately removing (wasteful allocation). The update loop already matches on `TransitionStatus(Done)` and `SetBlocked`, making it natural to set a flag. The flag also prevents the summary from being briefly present after a terminal transition.

**Consequences:** Slightly more code in the update loop (one boolean), but cleaner semantics — the HashMap never contains stale entries, even transiently.

#### Decision: Cleanup placement after coordinator updates

**Context:** The PRD specifies cleanup must occur after all coordinator status updates.

**Decision:** Place `remove()` calls as the final operation in each handler, after all coordinator updates.

**Rationale:** Matches the PRD constraint. If a coordinator update fails (returns `Err`), the handler returns early via `?` before reaching the `remove()` call, so the summary persists — which is correct, since the item didn't actually reach the terminal state.

**Consequences:** If a coordinator update fails, the summary is retained (safe — item retries or is manually resolved).

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Manual cleanup discipline | Must remember to call `cleanup_terminal_summary` for any future terminal state transitions | Simplicity — no LRU cache, no periodic sweep, no external dependencies | Only 3 terminal transition paths exist; all are in the same file. The centralized helper makes adding cleanup to a new path a single function call. The handler contract (documented above) and the `max_wip * 20` observability log provide guardrails. |
| No auto-shrink | HashMap internal allocation doesn't shrink after removals | Avoiding `shrink_to_fit()` call overhead and complexity | At `max_wip` scale (typically 1-5 entries), internal allocation overhead is negligible. |

---

## Alternatives Considered

### Alternative: Replace HashMap with LRU cache

**Summary:** Use a bounded LRU cache (e.g., `moka` or `lru` crate) that automatically evicts entries.

**How it would work:**
- Replace `HashMap<String, String>` with `LruCache<String, String>` bounded to `max_wip * 2`
- No explicit cleanup needed — cache self-bounds

**Pros:**
- Auto-bounded without manual cleanup
- No risk of forgetting cleanup for new terminal states

**Cons:**
- Adds external dependency for a trivial operation
- LRU eviction policy doesn't match the lifecycle semantics (may evict active items if bound is too low)
- Over-engineering for a ~5 entry map

**Why not chosen:** The problem has clear lifecycle transitions that make explicit cleanup the simpler, more correct approach. An LRU cache trades semantic correctness for automation that isn't needed here.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Forgetting `cleanup_terminal_summary` for a future terminal state | Summary entries leak for that transition path | Low — only one file, three established paths, centralized helper | The handler contract (documented in architecture), the centralized helper (single function call), and the `max_wip * 20` observability log provide layered guardrails. A periodic `retain()` sweep can be added later as defense-in-depth. |
| Coordinator update fails, summary retained | Summary persists for an item that didn't actually transition | Low — coordinator errors are rare and usually transient | Summaries are intentionally retained if coordinator updates fail, since the item didn't actually reach the terminal state. The item will be retried or manually resolved, and the summary provides context for the next attempt. |

---

## Integration Points

### Existing Code Touchpoints

- `scheduler.rs` — New `cleanup_terminal_summary` helper function (added near the handler functions)
- `scheduler.rs:handle_phase_success` (line 934) — Add `is_terminal` flag in update loop, conditionally call cleanup helper or insert
- `scheduler.rs:handle_phase_failed` (line 1073) — Add `previous_summaries` parameter, call cleanup helper
- `scheduler.rs:handle_phase_blocked` (line 1100) — Add `previous_summaries` parameter, call cleanup helper
- `scheduler.rs:handle_task_completion` (line 894) — Update call sites at lines 914, 917 to pass `previous_summaries`
- `scheduler.rs:handle_phase_success` (line 1021) and `handle_subphase_complete` (line 1068) — Add observability check after insert

**Note:** `handle_triage_success` (line 1127) does NOT need modification. Triage is always the first phase for an item, so no `previous_summaries` entry exists at triage time. Even if triage sets Blocked via `apply_triage_result`, there is nothing to clean up.

**Note:** `drain_join_set` (line 1400) already receives `previous_summaries` and passes it through to `handle_task_completion`, so no signature change is needed there.

### External Dependencies

None. Uses only `std::collections::HashMap::remove()`.

---

## Open Questions

None. The approach is straightforward and fully validated by tech research.

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
| 2026-02-13 | Initial design draft | Point-of-transition removal with handler signature changes; light mode design complete |
| 2026-02-13 | Self-critique (7 agents) | Auto-fixed: centralized cleanup helper, dynamic observability threshold, unblock-reblock flow, handler contract documentation, triage handler exclusion rationale, boolean flag specifics. No directional issues. |
