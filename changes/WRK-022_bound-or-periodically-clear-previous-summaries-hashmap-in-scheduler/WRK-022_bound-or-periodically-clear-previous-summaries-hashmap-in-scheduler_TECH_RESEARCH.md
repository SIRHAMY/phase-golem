# Tech Research: Bound previous_summaries HashMap in Scheduler

**ID:** WRK-022
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-022_bound-or-periodically-clear-previous-summaries-hashmap-in-scheduler_PRD.md
**Mode:** Light

## Overview

Researching the best approach for cleaning up `previous_summaries` HashMap entries in the scheduler when items reach terminal states (Done/Blocked). The core question is simple: what's the idiomatic Rust pattern for lifecycle-based HashMap cleanup, and does the existing codebase have constraints or patterns we should follow?

## Research Questions

- [x] What is the idiomatic Rust pattern for removing HashMap entries tied to lifecycle state transitions?
- [x] Does the existing codebase have cleanup patterns we should follow?
- [x] Which handler functions need signature changes to support cleanup?
- [x] Are there any concurrency or timing gotchas with removing entries in an async scheduler?

---

## External Research

### Landscape Overview

The problem of HashMap entries accumulating for completed items in a long-running scheduler is a well-recognized pattern. It is a "logical memory leak" — unbounded growth of a collection that retains entries for items that will never be accessed again. The standard Rust approach is straightforward: call `HashMap::remove()` at the point where the item reaches a terminal state. No exotic caching libraries, TTL-based eviction, or bounded data structures are needed when clear lifecycle transitions serve as natural cleanup points.

### Common Patterns & Approaches

#### Pattern: Point-of-Transition Removal (Recommended)

**How it works:** When an item transitions to a terminal state (Done, Blocked), the handler that processes that transition calls `HashMap::remove(&item_id)` to immediately evict the entry.

**When to use:** When you have clear, deterministic lifecycle transitions and you know exactly when an entry is no longer needed.

**Tradeoffs:**
- Pro: O(1) per removal, no iteration overhead, no additional dependencies
- Pro: Cleanup is deterministic and immediate
- Pro: Easy to reason about and test
- Con: Requires discipline to add `remove()` calls at every terminal transition point; if a new terminal state is added and `remove()` is forgotten, entries will leak

**References:**
- [HashMap::remove() documentation](https://doc.rust-lang.org/std/collections/struct.HashMap.html) — Official Rust docs
- [HashMap Entry API discussion](https://users.rust-lang.org/t/how-to-remove-with-hashmaps-entry-api/49961) — Community patterns

#### Pattern: Periodic Defensive Sweep with `retain()`

**How it works:** Periodically call `HashMap::retain(|key, _| active_items.contains(key))` to sweep away entries whose item IDs are no longer in the active backlog.

**When to use:** As defense-in-depth for long-running systems. Listed as "Nice to Have" in the PRD.

**Tradeoffs:**
- Pro: Catches entries missed by point-of-transition removal
- Con: O(n) in HashMap entries per sweep
- Con: Requires access to the current set of active item IDs

**References:**
- [HashMap::retain() documentation](https://doc.rust-lang.org/std/collections/struct.HashMap.html#method.retain)

### Technologies & Tools

No external libraries needed. The standard library `HashMap::remove()` is the right tool. Libraries like [moka](https://github.com/moka-rs/moka) (bounded cache with TTL) or [transient-hashmap](https://docs.rs/transient-hashmap) (time-based eviction) are overkill for simple state-based cleanup.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| HashMap does not auto-shrink after removal | Internal bucket allocation stays high after removals | Not a concern here — map holds at most `max_wip` entries at steady state. `shrink_to_fit()` not needed. |
| Premature removal during retries | Removing summary on transient failure breaks context chain for retry | Only remove on terminal states (Done, Blocked). Retries use separate `failure_context` mechanism. |
| Race between cleanup and read in async | Removing entry while another task reads it | Not a risk — scheduler is single-threaded for HashMap access; summaries are `.cloned()` before task spawn. |
| Forgetting to add `remove()` for future terminal states | New terminal states could leak entries | Defensive `retain()` sweep (Nice to Have) provides safety net. |

### Key Learnings

- `HashMap::remove()` is the standard, idiomatic approach. No libraries needed.
- HashMap does not auto-shrink, but at `max_wip` scale this is irrelevant.
- Per-entry overhead is ~73% on top of data ([measured here](https://ntietz.com/blog/rust-hashmap-overhead/)), reinforcing value of cleanup even for small entries.

---

## Internal Research

### Existing Codebase State

The `previous_summaries` HashMap is a local variable in `run_scheduler()` at line 481 of `scheduler.rs`:

```rust
let mut previous_summaries: HashMap<String, String> = HashMap::new();
```

**Current lifecycle:**
- **Insert:** Lines 904 and 951 — inserted at end of `handle_phase_success()` and `handle_subphase_complete()`
- **Read:** Line 620 — cloned before being passed to spawned async tasks as `Option<&str>`
- **Never removed:** No cleanup occurs; entries accumulate for the scheduler session lifetime

**Relevant files/modules:**
- `scheduler.rs` — HashMap definition, handler functions, all insert/read sites
- `executor.rs` — Receives summary as `Option<&str>` at line 658
- `prompt.rs` — Renders summary as `## Previous Phase Summary` (lines 188-189, 340-344)

**Existing patterns in use:**
- `RunningTasks` cleanup via `.remove()` (lines 90-92, 682, 1294) — precedent for cleanup on state transitions
- `coordinator.archive_item()` called for Done transitions (line 888) — natural cleanup point
- `log_debug!()` macro used throughout for observability

### Handler Functions and Signatures

| Handler | Receives `previous_summaries`? | Terminal Transitions |
|---------|-------------------------------|---------------------|
| `handle_phase_success()` (line 817) | Yes — `&mut HashMap<String, String>` | Done (line 885-891), Blocked (line 893-896) |
| `handle_subphase_complete()` (line 908) | Yes — `&mut HashMap<String, String>` | None — subphase completions are non-terminal |
| `handle_phase_failed()` (line 956) | **No** | Blocked (line 974) |
| `handle_phase_blocked()` (line 983) | **No** | Blocked (line 1001) |

**Call site:** All handlers are called from `handle_task_completion()` (lines 777-815). The failed/blocked call sites at lines 797-800 will need updating.

### Reusable Components

- `ItemStatus` enum: `Done` and `Blocked` are the terminal states
- `ItemUpdate` enum: `TransitionStatus(ItemStatus::Done)` and `SetBlocked(reason)` matching patterns already exist
- `log_debug!()` macro for observability logging

### Constraints from Existing Code

- HashMap is a local variable in `run_scheduler()` — cleanup must happen within handler functions that receive `&mut` reference
- Single-threaded HashMap access (async but not multi-threaded) — no race conditions
- Summaries are `.cloned()` before task spawn (line 620) — cleanup cannot affect in-flight executions
- Cleanup must occur after all coordinator updates per PRD requirement

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Adding `previous_summaries` param to `handle_phase_failed()` and `handle_phase_blocked()` | Confirmed — these handlers do not currently receive the parameter; call sites at lines 797-800 need updating | Straightforward signature change; no design concerns |
| `handle_phase_success()` is the cleanup point for Done transitions | Confirmed — it calls `coordinator.archive_item()` at line 888 | Add `.remove()` after the archive call |
| Only terminal states are safe cleanup points | Confirmed — retries use separate `failure_context`, not `previous_summaries` | PRD is correct; no risk of premature removal |

No conflicts between PRD assumptions and research findings. The PRD's analysis is accurate and complete.

---

## Critical Areas

### Ensuring all terminal transition paths have cleanup

**Why it's critical:** There are multiple code paths that transition items to Done or Blocked. Missing any one path would leave entries leaking.

**Why it's easy to miss:** Terminal transitions in `handle_phase_success()` include both Done (line 885) and Blocked (line 893) — the Blocked path in this handler could be overlooked since it's the "success" handler.

**What to watch for:** All three handlers with terminal transitions need cleanup:
1. `handle_phase_success()` — Done path AND Blocked path
2. `handle_phase_failed()` — Blocked path (when retries exhausted)
3. `handle_phase_blocked()` — Blocked path

---

## Deep Dives

No deep dives needed — the approach is clear and well-validated by both external patterns and internal codebase analysis.

---

## Synthesis

### Open Questions

None. The approach is straightforward and fully validated.

### Recommended Approaches

#### Cleanup Mechanism

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Point-of-transition `remove()` | O(1), deterministic, idiomatic, no dependencies | Must add `remove()` at every terminal path | Clear lifecycle transitions exist (our case) |
| Replace with LRU cache | Auto-bounded | Extra dependency, complexity for no benefit | No clear lifecycle transitions |
| Periodic `retain()` sweep | Catches missed cleanup points | O(n) per sweep, needs active item set | Defense-in-depth (Nice to Have) |

**Initial recommendation:** Point-of-transition `remove()` as the primary mechanism. This is the PRD's proposed approach and research fully validates it.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [HashMap::remove() docs](https://doc.rust-lang.org/std/collections/struct.HashMap.html) | Official docs | API reference for the primary operation |
| [HashMap memory overhead](https://ntietz.com/blog/rust-hashmap-overhead/) | Blog post | Quantifies per-entry overhead (~73%) |
| [Pretty State Machine Patterns](https://hoverbear.org/blog/rust-state-machine-pattern/) | Blog post | Idiomatic Rust state transition patterns |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Light external research: HashMap cleanup patterns in Rust | Confirmed `remove()` is the standard approach; no libraries needed |
| 2026-02-12 | Light internal research: codebase analysis of scheduler handlers | Mapped all terminal transition paths, identified handler signature changes needed |
| 2026-02-12 | PRD analysis | No conflicts found; PRD assumptions fully validated by research |
