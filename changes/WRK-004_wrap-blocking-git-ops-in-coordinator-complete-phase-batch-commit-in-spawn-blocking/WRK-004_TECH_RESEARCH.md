# Tech Research: Wrap Blocking Git Ops in spawn_blocking

**ID:** WRK-004
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-004_PRD.md
**Mode:** Light

## Overview

Research how to wrap the synchronous git calls in `handle_complete_phase` and `handle_batch_commit` with `tokio::task::spawn_blocking`, following the existing pattern already used for `GetHeadSha` and `IsAncestor` in the same coordinator actor loop. The main questions are: (1) confirm the right approach, (2) understand data ownership constraints for the closures, and (3) identify where state mutations must remain on the async executor thread.

## Research Questions

- [x] What is the correct pattern for wrapping blocking git ops in spawn_blocking? — **Answered:** Follow the existing `GetHeadSha`/`IsAncestor` pattern already in the codebase.
- [x] What data needs to be cloned/moved into the blocking closure? — **Answered:** `project_root`, `backlog_path`, `output_paths`, `item_id`, `result.phase`, `is_destructive`, and `pending_batch_phases` (for `batch_commit`).
- [x] Where must state mutations remain outside the closure? — **Answered:** `pending_batch_phases.push` and `pending_batch_phases.clear` must stay on the async executor thread.

---

## External Research

### Landscape Overview

Tokio implements a cooperative event loop where tasks should voluntarily yield control. When synchronous blocking operations (like `std::process::Command` for git calls) run directly on the async executor thread, they block the entire runtime. Tokio provides two primary mechanisms:

1. **`spawn_blocking`** — Spawns on a dedicated thread pool (recommended for subprocess I/O)
2. **`block_in_place`** — Runs on current thread while executor migrates other tasks

The ecosystem strongly recommends `spawn_blocking` for subprocess operations.

### Common Patterns & Approaches

#### Pattern: `tokio::task::spawn_blocking`

**How it works:** Spawns a blocking closure on a dedicated thread pool separate from the main async runtime. Returns a `JoinHandle<T>` that can be `.await`ed.

**When to use:** Non-async operations that eventually finish — CPU-intensive work, synchronous third-party libraries, `std::process::Command` subprocess calls.

**Tradeoffs:**
- Pro: Prevents blocking the event loop, scales automatically, safe for long-running operations
- Con: Context-switch overhead (avoid for operations <1ms), cannot be aborted once started

**References:**
- [spawn_blocking official docs](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
- [Bridging with sync code - Tokio](https://tokio.rs/tokio/topics/bridging)

#### Pattern: `tokio::process::Command` (Alternative, out of scope)

**How it works:** Fully async subprocess execution, no thread pool needed.

**When to use:** Would be a deeper refactor of the git module itself.

**Tradeoffs:**
- Pro: No thread pool overhead, can be cancelled
- Con: Requires refactoring all of `git.rs`, more invasive change

**References:**
- [tokio::process docs](https://docs.rs/tokio/latest/tokio/process/index.html)

### Standards & Best Practices

- All message handlers in an actor loop should follow the same pattern for consistency
- Always `.await` the `JoinHandle` and handle `JoinError` — never drop it silently
- Return `Result<T, E>` from blocking closures rather than relying on panic propagation

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Forgetting to `.await` the `JoinHandle` | Panics go undetected, silent failures | Always `.await` and handle `Result` |
| Not handling panics in spawned tasks | `JoinError` silently swallowed | Use `.unwrap_or_else` pattern |
| Moving mutable state references into closure | Violates Rust's ownership rules; closures on separate threads can't borrow `&mut` | Clone immutable data, keep mutations outside |

### Key Learnings

- `spawn_blocking` is the correct and standard approach for this use case
- The existing codebase already implements the exact right pattern
- No external libraries or tools needed beyond what's already in use

---

## Internal Research

### Existing Codebase State

The coordinator (`coordinator.rs`) runs an async actor loop at line 511 (`run_coordinator`). It processes `CoordinatorCommand` variants via match arms.

**Already using `spawn_blocking` correctly:**
- `GetHeadSha` (lines 556-563) — clones `project_root`, spawns blocking, awaits, handles panic
- `IsAncestor` (lines 565-572) — same pattern

**Not yet using `spawn_blocking` (the fix targets):**
- `CompletePhase` (lines 536-551) — calls `handle_complete_phase` synchronously
- `BatchCommit` (lines 552-555) — calls `handle_batch_commit` synchronously

**Relevant files/modules:**
- `orchestrator/src/coordinator.rs` — Main file to modify. Contains actor loop, handler functions, state management.
- `orchestrator/src/git.rs` — Synchronous git wrappers using `std::process::Command`. Functions: `get_status`, `stage_paths`, `commit`, `get_head_sha`, `is_ancestor`. All return `Result<T, String>`.
- `orchestrator/src/agent.rs` (lines 291-300) — Secondary example of `spawn_blocking` for `kill_process_group`.

### Existing Patterns

**Exact pattern to follow (GetHeadSha, lines 556-563):**

```rust
CoordinatorCommand::GetHeadSha { reply } => {
    let project_root = state.project_root.clone();
    let result = tokio::task::spawn_blocking(move || {
        crate::git::get_head_sha(&project_root)
    })
    .await
    .unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {}", e)));
    let _ = reply.send(result);
}
```

Key characteristics:
1. Clone immutable state before the closure
2. `move` closure captures owned data
3. `.await` the `JoinHandle`
4. `.unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {}", e)))` for panic handling
5. Send result through reply channel

### Reusable Components

- The error handling pattern `unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {}", e)))` is used consistently and should be reused as-is.
- Helper functions `collect_orchestrator_paths`, `build_phase_commit_message`, `has_staged_changes`, `build_batch_commit_message` are pure/non-blocking — safe to call inside `spawn_blocking` closures.

### Constraints from Existing Code

1. **State mutations stay on async thread:** `handle_complete_phase` conditionally pushes to `state.pending_batch_phases` (non-destructive path, line 411-413). `handle_batch_commit` clears `state.pending_batch_phases` (lines 426, 432). These must happen after `.await` returns.

2. **Ownership for closures:** Blocking closures must own their data. Need to clone: `project_root`, `backlog_path`, `output_paths`, `item_id`, `result.phase`, `is_destructive`. The `pending_batch_phases` data for `build_batch_commit_message` needs cloning too.

3. **Function signatures:** `get_status`, `stage_paths`, `commit` take `Option<&Path>` — references derived from owned `PathBuf` inside the closure work fine.

4. **Early returns in `handle_batch_commit`:** The function has two early return paths (empty pending list, no staged changes) that either skip git ops or clear state. The restructured code must handle these branches correctly — some branches need no `spawn_blocking` at all.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| "Group git calls into a single `spawn_blocking` per handler" | Confirmed as correct approach. Splitting across multiple closures would add complexity with no benefit since the coordinator processes commands sequentially. | No concern — PRD decision is sound. |
| "No test changes should be needed" | Tests interact via the async API (`CoordinatorHandle`), and handler signatures are internal. Tests should indeed pass unchanged. | Low risk, but verify by running tests after implementation. |
| "Nice to have: consolidated helper for spawn_blocking + error mapping" | The boilerplate is only 2 lines (`.await.unwrap_or_else(...)`) and a helper would add abstraction for minimal gain. | Recommend skipping the helper — not worth the abstraction overhead for 2 call sites. |

---

## Critical Areas

### State Mutation Ordering in `handle_complete_phase`

**Why it's critical:** The function has a branch: if `is_destructive`, it commits immediately; otherwise, it pushes to `pending_batch_phases`. The git operations (staging, committing) must be in `spawn_blocking`, but the `pending_batch_phases.push` must be outside.

**Why it's easy to miss:** The current function mixes git I/O and state mutation in the same scope. When refactoring to `spawn_blocking`, it's tempting to move the entire function body into the closure.

**What to watch for:** The closure should return enough information for the async code to decide whether to push to `pending_batch_phases`. Since `is_destructive` is known before spawning, the branch can be decided at the match arm level: if destructive, spawn a closure that stages + commits; if not, spawn a closure that stages only, then push to `pending_batch_phases` after `.await`.

### Early Returns in `handle_batch_commit`

**Why it's critical:** The function has two early return points that skip git operations: (1) empty pending list, (2) no staged changes. The "no staged changes" check requires calling `get_status` (a blocking call).

**Why it's easy to miss:** The early return on "no staged changes" includes both a git call AND a state mutation (`pending_batch_phases.clear()`). These need to be separated.

**What to watch for:** The blocking closure should return a result indicating whether to commit or just clear. The async code then handles the state mutation based on the result.

---

## Deep Dives

_(No deep dives needed — light mode research. The problem space is well-understood and the pattern is established in the codebase.)_

---

## Synthesis

### Open Questions

_(None — all research questions resolved.)_

### Recommended Approaches

#### Approach: Inline `spawn_blocking` in Match Arms

Move the git I/O into `spawn_blocking` closures directly in the `CompletePhase` and `BatchCommit` match arms, keeping state mutations on the async thread after `.await`.

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Inline in match arms (like GetHeadSha) | Consistent with existing pattern, clear data flow, state mutations visibly on async thread | Handler functions may be split or restructured | You want consistency and clarity (recommended) |
| Keep handler fns, make them async | Preserves function encapsulation | Hides `spawn_blocking` inside helpers, inconsistent with GetHeadSha/IsAncestor pattern | You want to minimize match arm size |

**Initial recommendation:** Inline in match arms, following the existing `GetHeadSha`/`IsAncestor` pattern. This is the most consistent approach and makes it visually obvious that all git operations in the actor loop use `spawn_blocking`.

#### Approach: Consolidated Helper (Nice to Have from PRD)

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| No helper | Simple, 2 call sites don't justify abstraction | Minor duplication of `.await.unwrap_or_else(...)` | Fewer than ~5 call sites (recommended) |
| Helper function | Reduces boilerplate | Adds indirection, generic types needed | Many call sites with identical error handling |

**Initial recommendation:** Skip the helper. Two call sites don't justify the abstraction.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [spawn_blocking docs](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) | Official docs | API reference for the primary mechanism |
| [Bridging with sync code](https://tokio.rs/tokio/topics/bridging) | Official guide | Patterns for async/sync integration |
| [Actors with Tokio](https://ryhl.io/blog/actors-with-tokio/) | Blog post | Actor pattern that the coordinator follows |
| `coordinator.rs` lines 556-572 | Internal code | Exact pattern to replicate |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Light external research on spawn_blocking patterns | Confirmed spawn_blocking is the correct approach; identified block_in_place as alternative (not applicable here) |
| 2026-02-12 | Light internal research on coordinator.rs and git.rs | Mapped existing pattern (GetHeadSha/IsAncestor), identified handler code to change, documented state mutation constraints |
| 2026-02-12 | PRD concern analysis | No conflicts found; PRD assumptions align with research findings |
