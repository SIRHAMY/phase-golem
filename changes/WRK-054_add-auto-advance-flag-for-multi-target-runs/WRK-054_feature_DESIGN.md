# Design: Add --auto-advance flag for multi-target runs

**ID:** WRK-054
**Status:** Complete
**Created:** 2026-02-19
**PRD:** ./WRK-054_feature_PRD.md
**Tech Research:** ./WRK-054_feature_TECH_RESEARCH.md
**Mode:** Medium

## Overview

Add a `--auto-advance` flag to the `run` subcommand that makes the scheduler skip blocked targets and continue to the next target instead of halting. The implementation is a conditional branch at the existing block detection site in `scheduler.rs` (lines 608-631), a new boolean field on `RunParams`, a CLI flag definition, and exit code logic in `main.rs`. The design follows the continue-on-error + checkpoint pattern identified in tech research, matches industry precedent (`make -k`, Argo `continueOn`), and requires ~30 lines of new logic across 2 files.

---

## System Design

### High-Level Architecture

The change is entirely within the existing scheduler loop and CLI argument parsing. No new modules, structs, or traits are introduced. The data flow is:

```
CLI (--auto-advance flag)
  → handle_run() threads it into RunParams
    → scheduler loop reads params.auto_advance at block detection site
      → if true: log, reset circuit breaker, advance index
      → if false: halt (existing behavior)
  → handle_run() inspects RunSummary for exit code
```

### Component Breakdown

#### CLI Argument (`src/main.rs`)

**Purpose:** Accept and validate the `--auto-advance` flag.

**Responsibilities:**
- Define `--auto-advance` as a boolean flag on the `Run` subcommand
- Thread the flag value into `RunParams`

**Interfaces:**
- Input: User passes `--auto-advance` on command line
- Output: `RunParams { auto_advance: true, ... }`

**Dependencies:** clap `ArgAction::SetTrue`

#### RunParams Extension (`src/scheduler.rs`)

**Purpose:** Carry the auto-advance configuration into the scheduler loop.

**Responsibilities:**
- Store `auto_advance: bool` field (default `false`)

**Interfaces:**
- Input: Constructed by `handle_run()`
- Output: Read by scheduler loop at block detection site

**Dependencies:** None (simple boolean field)

#### Block Detection Branch (`src/scheduler.rs`, lines 608-631)

**Purpose:** Conditionally skip or halt when a target is blocked at runtime.

**Responsibilities:**
- When `auto_advance` is false: existing halt behavior (return `HaltReason::TargetBlocked`)
- When `auto_advance` is true:
  1. Log the skip with target ID, position, and next target
  2. Drain join set and commit state
  3. Reset `state.consecutive_exhaustions` to 0
  4. Increment `state.current_target_index`
  5. `continue` the loop (re-enters at top, snapshot refresh at line 606, advancement past Done/pre-Blocked via `advance_to_next_active_target()`)

**Pseudocode for auto-advance branch:**

```rust
if state.items_blocked.contains(target_id) {
    if params.auto_advance {
        // 1. Log which target blocked and position
        log_info!("[target] {} blocked ({}/{}). Auto-advancing.",
            target_id, state.current_target_index + 1, params.targets.len());
        // 2. Drain + commit (symmetric with halt path; drain is no-op with max_wip=1)
        drain_join_set(&mut join_set, &mut running, &mut state, ...).await;
        let _ = coordinator.batch_commit().await;
        // 3. Reset circuit breaker BEFORE advancing (critical: next iteration checks
        //    is_circuit_breaker_tripped() at line 572 before reaching this branch)
        state.consecutive_exhaustions = 0;
        // 4. Advance to next target
        state.current_target_index += 1;
        // 5. Re-enter loop (snapshot refreshes at line 606, then
        //    advance_to_next_active_target skips Done/pre-Blocked at line 634)
        continue;
    } else {
        // Existing halt behavior (unchanged)
        drain_join_set(...).await;
        let _ = coordinator.batch_commit().await;
        return Ok(build_summary(state, HaltReason::TargetBlocked));
    }
}
```

**Interfaces:**
- Input: `params.auto_advance`, `state.items_blocked`, `state.current_target_index`
- Output: Either returns `HaltReason::TargetBlocked` or continues the loop

**Dependencies:** Existing `drain_join_set()`, `coordinator.batch_commit()`

#### Exit Code Logic (`src/main.rs`)

**Purpose:** Map `RunSummary` to process exit code when auto-advance is active.

**Responsibilities:**
- Exit 0 when `items_completed` is non-empty (at least one target completed)
- Exit non-zero when `items_completed` is empty and `items_blocked` is non-empty (all targets blocked)
- Unchanged for runs without `--auto-advance`

**Location:** After the summary is printed (after line 738 in `handle_run()`), before the `Ok(())` return at line 740.

**Pseudocode:**

```rust
// In handle_run(), after printing the summary and before Ok(())
if summary.items_completed.is_empty() && !summary.items_blocked.is_empty() {
    return Err("All targets blocked, none completed".to_string());
}
// Returning Err causes main() at line 162-164 to call process::exit(1)
```

This applies regardless of whether `--auto-advance` was passed. Without `--auto-advance`, a blocked target halts with `HaltReason::TargetBlocked` and `items_completed` may or may not be empty (depending on whether earlier targets completed). The exit code logic is mode-agnostic: it only checks the summary state.

**Exit code decision matrix:**

| `items_completed` | `items_blocked` | Exit Code | Scenario |
|-------------------|-----------------|-----------|----------|
| Non-empty | Empty | 0 | All targets completed |
| Non-empty | Non-empty | 0 | Partial success (some completed, some blocked) |
| Empty | Non-empty | 1 | All targets blocked |
| Empty | Empty | 0 | No targets processed (e.g., all pre-Blocked/Done) |

**Interfaces:**
- Input: `RunSummary` (items_completed, items_blocked, halt_reason)
- Output: `Ok(())` (exit 0) or `Err(String)` (exit 1 via `main()` error handler)

**Dependencies:** `RunSummary` struct

### Data Flow

1. User invokes `phase-golem run --target WRK-005 --target WRK-010 --auto-advance`
2. `main.rs` parses `--auto-advance` flag via clap, constructs `RunParams { auto_advance: true, ... }`
3. Scheduler loop processes WRK-005. If WRK-005 becomes blocked:
   a. `state.items_blocked` contains "WRK-005"
   b. Block detection at line 613 fires
   c. Auto-advance branch: log skip, commit state (`coordinator.batch_commit()`), reset circuit breaker, increment target index, `continue`
4. Next iteration: snapshot refresh, `advance_to_next_active_target()` skips Done/pre-Blocked, lands on WRK-010
5. Scheduler processes WRK-010 normally
6. When target list exhausted: returns `HaltReason::TargetCompleted`
7. `handle_run()` prints summary, checks if `items_completed` is empty with `items_blocked` non-empty → exit 1 if so

### Key Flows

#### Flow: Target blocks with auto-advance enabled

> Scheduler detects a runtime-blocked target and skips to the next instead of halting.

1. **Block detected** — `state.items_blocked.contains(target_id)` is true at line 613
2. **Log skip** — `log_info!("[target] WRK-005 blocked (1/3). Auto-advancing to WRK-010.")`
3. **Drain join set** — `drain_join_set()` clears any completed tasks (should be empty with `max_wip=1`)
4. **Commit state** — `coordinator.batch_commit()` persists the blocked target's state to git
5. **Reset circuit breaker** — `state.consecutive_exhaustions = 0`
6. **Advance index** — `state.current_target_index += 1`
7. **Continue loop** — `continue` re-enters the loop, triggering snapshot refresh and `advance_to_next_active_target()`

**Edge cases:**
- Last target blocks — index increments past `targets.len()`, next iteration hits the `>= targets.len()` check at line 640 and returns `TargetCompleted`
- All targets block — same as above, but `items_completed` is empty. Exit code logic in `main.rs` returns non-zero
- Single target with auto-advance — behaves as if auto-advance is a no-op: target blocks, auto-advance increments index, index exceeds target list, returns `TargetCompleted` (normal exhaustion)

#### Flow: Multiple consecutive targets block (circuit breaker interaction)

> Two targets in a row exhaust retries, but auto-advance prevents false circuit breaker trip.

1. **WRK-005 exhausts retries** — `consecutive_exhaustions` increments (e.g., to 2)
2. **WRK-005 detected as blocked** — auto-advance branch fires
3. **Circuit breaker reset** — `consecutive_exhaustions` set to 0 before advancing
4. **WRK-010 processed** — starts fresh with `consecutive_exhaustions = 0`
5. **WRK-010 exhausts retries** — `consecutive_exhaustions` increments again, but from 0, not from 2

**Edge cases:**
- If the same target exhausts retries multiple times within its own processing (before being marked blocked), the circuit breaker can still trip for that single target. Auto-advance only resets between targets.

#### Flow: All targets complete successfully with auto-advance

> Normal multi-target run where no targets block; auto-advance flag has no effect.

1. **WRK-005 completes** — `items_completed` gains "WRK-005", target index advances normally
2. **WRK-010 completes** — same, target list exhausted
3. **Returns `TargetCompleted`** — `items_completed` is non-empty, exit 0

#### Flow: Run summary with mixed results

> Some targets completed, some blocked. Summary distinguishes them.

1. **WRK-005 completes** — logged and added to `items_completed`
2. **WRK-010 blocks** — auto-advance skips it, added to `items_blocked`
3. **WRK-003 completes** — logged and added to `items_completed`
4. **Summary output:**
   ```
   Items completed: WRK-005, WRK-003
   Items blocked: WRK-010
   Halt reason: TargetCompleted
   ```
5. **Exit 0** — at least one target completed

---

## Technical Decisions

### Key Decisions

#### Decision: Conditional branch at existing block detection site

**Context:** Need to add skip-or-halt logic when a target blocks at runtime.

**Decision:** Add an `if params.auto_advance { ... } else { ... }` branch inside the existing block detection check at lines 608-631, rather than creating a new function or restructuring the loop.

**Rationale:** The block detection site is already the single decision point for "what to do when a target blocks." Adding a conditional here keeps the logic co-located and avoids restructuring the loop. The existing `drain_join_set()` and `coordinator.batch_commit()` calls are already at this site.

**Consequences:** The auto-advance branch shares the same drain/commit infrastructure. If the block detection logic moves in the future, auto-advance moves with it.

#### Decision: Reset circuit breaker counter on auto-advance

**Context:** The circuit breaker (`consecutive_exhaustions`) counts consecutive retry exhaustions. If target A exhausts retries and target B also exhausts retries, the counter reaches 2× without reset, potentially tripping the breaker despite the targets being independent.

**Decision:** Reset `state.consecutive_exhaustions = 0` in the auto-advance branch, immediately before incrementing the target index.

**Rationale:** Each target is independent work. The circuit breaker is designed to detect systemic problems (e.g., broken agent), not independent target failures. Tech research confirms every industry implementation treats shared breaker state across independent items as a bug.

**Consequences:** The circuit breaker can still trip within a single target's processing (multiple retries on the same item), but not across auto-advanced targets.

#### Decision: Binary exit code (0/1)

**Context:** The process needs to signal partial success (some targets completed, some blocked) vs. total failure (all blocked).

**Decision:** Exit 0 when `items_completed` is non-empty; exit 1 when `items_completed` is empty and `items_blocked` is non-empty. No distinct exit codes for "all blocked" vs. other errors.

**Rationale:** The Better CLI guide recommends binary 0/non-zero for CI compatibility. The run summary stdout provides granular details. Distinct exit codes (e.g., exit 2 for "all blocked") can be added as a follow-up if users need scriptable distinction.

**Consequences:** Scripts cannot distinguish "all blocked" from "execution error" by exit code alone — they must parse the summary output. This is acceptable for the initial implementation.

#### Decision: Deduplicate `items_blocked` before summary

**Context:** Multiple code paths in the scheduler can push the same item ID to `state.items_blocked` (guardrail rejection, retry exhaustion, status update handler, triage detection). A single target can appear multiple times if it triggers blocks through different mechanisms.

**Decision:** Deduplicate `items_blocked` in `build_summary()` before constructing `RunSummary`. Use `Vec::dedup()` after sorting, or collect through a seen-set. Each target ID appears at most once in the summary.

**Rationale:** The run summary is user-facing output. Duplicate entries confuse users and inflate counts. The exit code logic (`items_completed.is_empty() && !items_blocked.is_empty()`) depends on `items_blocked` being meaningful, not inflated. Deduplication at the summary boundary is the minimal change — it doesn't require changing the `Vec<String>` type in `SchedulerState` or modifying any push sites.

**Consequences:** The raw `items_blocked` vec in `SchedulerState` can still contain duplicates during the run (useful for debugging). Only the externally visible `RunSummary` is deduplicated.

#### Decision: Log format for auto-advance skip messages

**Context:** PRD requires log messages identifying the blocked target and position at skip time.

**Decision:** Use `log_info!()` with the existing `[target]` prefix pattern and position counter:

```rust
log_info!(
    "[target] {} blocked ({}/{}). Auto-advancing.",
    target_id,
    state.current_target_index + 1,
    params.targets.len()
);
```

The message does not name the next target because `advance_to_next_active_target()` may skip multiple targets (Done/pre-Blocked), making the "next" target ambiguous at log time. The position counter `(N/M)` tells the user where in the list the block occurred.

**Rationale:** Matches the existing log patterns (lines 614-618 use the same `[target]` prefix and position format). Avoids a lookahead that could be stale.

**Consequences:** The log doesn't name the next active target. The user can infer it from subsequent log lines when the next target starts processing.

#### Decision: No drain needed before auto-advance

**Context:** When auto-advancing, we need to ensure no in-flight tasks from the blocked target remain.

**Decision:** Still call `drain_join_set()` before auto-advancing (same as the existing halt path), even though `max_wip=1` in target mode means the join set should be empty.

**Rationale:** Defensive correctness. The drain is a no-op in practice but ensures correctness if `max_wip` changes in the future. The existing halt path already calls it; the auto-advance path should be symmetric.

**Consequences:** Negligible performance impact (draining an empty set is nearly free).

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| No distinct "all blocked" exit code | Scripts can't distinguish all-blocked from error by exit code alone | Simpler implementation, CI-compatible binary codes | The summary output provides the detail; distinct codes can be added later |
| Flag is silently ignored without `--target` | User might not realize the flag does nothing when no targets specified | Avoids erroring in scripts/aliases that always pass the flag | `--only` and `--target` are mutually exclusive via clap `conflicts_with`, so `--auto-advance` + `--only` is already impossible. The only remaining case is `--auto-advance` alone (no targets, no filter), which is harmless — the scheduler never enters target mode |
| No config-file default | Users must pass flag every time | Simpler implementation, no config schema changes | Config default is a follow-up item per PRD |

---

## Alternatives Considered

### Alternative: New HaltReason variant for auto-advance

**Summary:** Add a `TargetAutoAdvanced` variant to `HaltReason` to distinguish runs that auto-advanced past blocks from normal completions.

**How it would work:**
- Add `TargetAutoAdvanced` to the `HaltReason` enum
- Return it instead of `TargetCompleted` when auto-advance was active and at least one target was skipped
- Use the variant for exit code decisions

**Pros:**
- Makes the halt reason semantically distinct from normal completion
- Exit code logic can pattern-match on halt reason alone

**Cons:**
- `HaltReason` is a simple enum with no payload; adding variants for modes creates combinatorial growth
- The existing `TargetCompleted` + `items_blocked` list already provides the same information
- Every match arm on `HaltReason` would need updating

**Why not chosen:** The combination of `TargetCompleted` + non-empty `items_blocked` is sufficient to distinguish auto-advanced runs. Adding a variant would be redundant and increase match arm maintenance.

### Alternative: Wrap scheduler loop body in a per-target retry loop

**Summary:** Instead of modifying the block detection branch, wrap the entire loop body in a `for target in targets` loop, catching blocks and continuing.

**How it would work:**
- Restructure the scheduler to iterate over targets explicitly
- Each target gets its own sub-loop for phases
- Blocks are caught at the outer loop level

**Pros:**
- Cleaner conceptual separation between target iteration and phase processing

**Cons:**
- Major refactor of the scheduler loop (hundreds of lines)
- Duplicates advancement logic that already works
- High risk of introducing regressions for a simple feature

**Why not chosen:** Disproportionate effort and risk for a feature that can be cleanly implemented with a ~15-line conditional at the existing decision point.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Circuit breaker reset ordering wrong | Independent targets trip the circuit breaker incorrectly | Low | Reset `consecutive_exhaustions` immediately in the auto-advance branch, before advancing. Dedicated test case with two consecutively blocked targets. |
| Exit code logic misses edge case | All-blocked run exits 0 (false success) | Low | Explicit check in `handle_run`: if `items_completed.is_empty() && !items_blocked.is_empty()`, exit 1. Test case for all-blocked scenario. |
| `batch_commit()` fails during auto-advance | Blocked target state not persisted before advancing | Very Low | Same risk exists in the current halt path. `batch_commit()` failure is logged but doesn't prevent advancement — the backlog is saved at shutdown anyway. |

---

## Integration Points

### Existing Code Touchpoints

- `src/main.rs` lines 56-66 — Add `--auto-advance` flag to `Run` variant
- `src/main.rs` lines 624-630 — Add `auto_advance` to `RunParams` construction
- `src/main.rs` lines 707-738 — Add exit code logic based on `RunSummary`
- `src/scheduler.rs` lines 49-59 — Add `auto_advance: bool` field to `RunParams`
- `src/scheduler.rs` lines 608-631 — Add conditional auto-advance branch at block detection
- `tests/scheduler_test.rs` — Add test cases for auto-advance behavior

### External Dependencies

None. No new crates, APIs, or external services.

---

## Open Questions

- [x] Duplicate target IDs — Already handled: `handle_run()` (lines 389-401) detects duplicates at parse time using a `HashSet` and rejects the run with an error listing the duplicate IDs
- [x] `items_blocked` deduplication — Resolved: deduplicate in `build_summary()` before constructing `RunSummary` (see Key Decisions above)

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain (or they're flagged for spec phase)

---

## Assumptions

Decisions made without human input:

- **Mode: medium** — Per workflow default. Two alternatives explored, key tradeoffs analyzed.
- **No `PRODUCT_VISION.md` found** — No product vision file exists at the project root; design decisions guided by PRD and tech research only.
- **Auto-advance branch calls drain_join_set()** — Even though join set should be empty with `max_wip=1`, keeping symmetry with the halt path for defensive correctness.
- **Exit code logic in handle_run, not scheduler** — The scheduler returns `RunSummary`; the caller decides the exit code. This keeps the scheduler pure of process-level concerns.
- **No distinct summary messaging for all-blocked** — The PRD's "Should Have" criterion for distinct messaging is addressed by the existing `items_completed`/`items_blocked` separation in the summary output. No new log formatting needed beyond the per-skip log messages.

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-19 | Initial design draft | Recommended approach: conditional branch at block detection site with circuit breaker reset. Two alternatives evaluated. |
| 2026-02-19 | Self-critique (7 agents) | 6 auto-fixes applied: added pseudocode for auto-advance branch, concrete exit code logic with decision matrix, `items_blocked` dedup decision, log format specification, clarified duplicate target handling, clarified `--only` interaction. No directional items remaining. |
