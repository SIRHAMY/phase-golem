# Tech Research: Wrap kill_all_children in spawn_blocking

**ID:** WRK-017
**Status:** Complete
**Created:** 2026-02-19
**PRD:** ./WRK-017_wrap-kill-all-children-in-spawn-blocking-for-async-call-sites_PRD.md
**Mode:** Light

## Overview

Researching the correct pattern for wrapping two async call sites of `kill_all_children()` in `tokio::task::spawn_blocking`, including error handling, closure capture semantics, and shutdown-path constraints. The codebase already has an established pattern via `kill_process_group()`; this research verifies that pattern is correct and identifies any gotchas.

## Research Questions

- [x] What is the exact `spawn_blocking` + error handling pattern used in this codebase?
- [x] Are there any gotchas with `spawn_blocking` during shutdown or signal handling?
- [x] Does the closure need `move` when `kill_all_children()` takes no parameters?

---

## External Research

### Landscape Overview

`tokio::task::spawn_blocking` is the standard Tokio mechanism for running synchronous, blocking code (CPU-bound work, OS blocking calls, `std::thread::sleep` loops) without starving the async worker thread pool. The worker pool is sized to match CPU cores and is intended exclusively for non-blocking futures; blocking work on a worker thread delays every other future scheduled on that thread. `spawn_blocking` offloads closures to a separate, dynamically-sized blocking thread pool (default max 512 threads).

The pattern used in this codebase — `spawn_blocking(move || { ... }).await.unwrap_or_else(|e| ...)` — is idiomatic and well-established in the Tokio ecosystem.

### Common Patterns & Approaches

#### Pattern: Fire-and-forget with panic log (codebase's existing pattern)

**How it works:** Wrap the blocking call in `spawn_blocking`, `.await` the `JoinHandle`, and absorb a panic via `unwrap_or_else` with a warn log. The return value of the closure is `()`.

**When to use:** Cleanup/teardown side-effecting calls that must not propagate errors — exactly the semantics of `kill_all_children()`.

**Tradeoffs:**
- Pro: Simple, idiomatic, matches codebase conventions
- Con: Panics are silently absorbed (intentional for cleanup)

**References:**
- [tokio::task::spawn_blocking docs](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) — canonical API reference
- [Tokio spawning tutorial](https://tokio.rs/tokio/tutorial/spawning) — official introduction

#### Pattern: Propagate JoinError as domain error (coordinator pattern)

**How it works:** Closure returns `Result<T, E>`. `.unwrap_or_else` collapses `JoinError` into `Err`, producing a single `Result<T, E>`.

**When to use:** When the blocking work has a meaningful return value or error path (git operations, file I/O).

**Tradeoffs:**
- Pro: Errors surface to callers
- Con: More complex, not needed for void cleanup

#### Pattern: block_in_place (alternative, not recommended here)

**How it works:** Runs blocking code on the current worker thread, temporarily migrating other tasks away.

**When to use:** Only when avoiding a new thread is critical.

**Tradeoffs:**
- Pro: No thread spawn overhead
- Con: Hurts cache locality for other tasks, cannot be used in `current_thread` runtimes

### Standards & Best Practices

- Tokio tutorial states blocking work should always use `spawn_blocking` or `block_in_place`, never run directly on async tasks
- `std::thread::sleep` is the canonical example of work requiring `spawn_blocking`
- Prefer `spawn_blocking` over `block_in_place` for general use
- Functions called when the runtime may be unavailable must remain synchronous

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| `spawn_blocking` tasks cannot be cancelled once started | `.abort()` has no effect on started blocking tasks; runtime waits for them on shutdown | Accept bounded blocking duration (5s max here is fine) |
| `spawn_blocking` during runtime shutdown may silently drop closure | If called after shutdown begins, closure may never execute (tokio#7499) | Only use at call sites confirmed within a live runtime — keep the function itself sync for shutdown paths |
| Using `tokio::time::sleep` inside `spawn_blocking` | Returns a Future, cannot be used in sync context | Use `std::thread::sleep` (already correct in this codebase) |

### Key Learnings

- The existing `kill_process_group` pattern is correct and idiomatic
- The constraint that `kill_all_children()` must remain sync is well-supported by tokio#7499 (shutdown-path risk)
- The 5-second blocking duration is a textbook `spawn_blocking` use case
- Non-cancellability of blocking tasks is acceptable for bounded cleanup

---

## Internal Research

### Existing Codebase State

**Relevant files/modules:**

- `src/agent.rs:71-113` — `kill_all_children()` definition: synchronous `pub fn` that sends SIGTERM to all registered PGIDs, polls with 100ms intervals for up to 5 seconds, then sends SIGKILL to survivors
- `src/agent.rs:304-333` — `kill_process_group()` definition: **the gold standard pattern** for `spawn_blocking` with panic-absorbing error handling
- `src/main.rs:635` — async call site in `handle_run` (after scheduler completes, before coordinator shutdown)
- `src/main.rs:851` — async call site in `handle_triage` (after dropping coordinator_handle, before final log)
- `src/coordinator.rs:620-652` — additional `spawn_blocking` patterns with `Result` propagation

**Existing patterns in use:**

- `kill_process_group()` (agent.rs:304-333): `spawn_blocking(move || { ... }).await.unwrap_or_else(|e| log_warn!("... task panicked: {}", e))` — fire-and-forget with panic log
- `coordinator.rs` (lines 620-652): `spawn_blocking(move || { ... }).await.unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {}", e)))` — propagate as `Result`

### Reusable Components

- `log_warn!` macro — already imported in agent.rs, available in main.rs
- `kill_all_children()` function — remains unchanged, just gets wrapped at call sites
- Constants (`SIGTERM_GRACE_PERIOD_SECONDS`, `KILL_POLL_INTERVAL_MS`) — already defined, no changes needed

### Constraints from Existing Code

- `kill_all_children()` must remain sync (`pub fn`) — called from shutdown paths where tokio runtime may be unavailable (WRK-001 constraint)
- Both call sites are async functions inside `#[tokio::main]` — `spawn_blocking` is safe here
- Both call sites are mutually exclusive CLI subcommands — no concurrency concerns
- Silent failure semantics — current direct call doesn't propagate errors; new pattern must match

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| No concerns identified | Research fully validates the PRD approach | Proceed as specified |

The PRD is well-aligned with research findings. No conflicts, complications, or underestimated areas.

---

## Critical Areas

None identified. The change is mechanical, the pattern is established, and the constraints are well-documented.

---

## Deep Dives

None needed for light mode. The pattern is straightforward and well-understood.

---

## Synthesis

### Open Questions

None — all research questions resolved.

### Recommended Approaches

#### Wrapping Pattern

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `spawn_blocking` + `unwrap_or_else` with `log_warn!` | Matches `kill_process_group` pattern exactly; idiomatic; simple | Panics absorbed silently | Cleanup calls returning `()` (this case) |
| `spawn_blocking` + `unwrap_or_else` with `Err(...)` | Propagates errors as `Result` | Overkill for void cleanup; changes call site error semantics | Calls returning `Result<T, E>` |
| `block_in_place` | No thread spawn | Hurts other task locality; not used for this pattern in codebase | Never for this use case |

**Initial recommendation:** Use the first approach — it exactly matches the existing `kill_process_group` pattern at agent.rs:304-333.

#### Exact Implementation

Replace at both call sites:
```rust
kill_all_children();
```
With:
```rust
tokio::task::spawn_blocking(|| {
    kill_all_children();
})
.await
.unwrap_or_else(|e| log_warn!("kill_all_children task panicked: {}", e));
```

Note: `move` is optional since `kill_all_children()` takes no parameters and the closure captures nothing. Both `|| { ... }` and `move || { ... }` compile identically. The existing pattern uses `move` — either is acceptable.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [tokio::task::spawn_blocking docs](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) | Official docs | Canonical API reference including shutdown behavior |
| [tokio::task::JoinError docs](https://docs.rs/tokio/latest/tokio/task/struct.JoinError.html) | Official docs | `is_panic()`, `is_cancelled()` methods |
| [Tokio spawning tutorial](https://tokio.rs/tokio/tutorial/spawning) | Tutorial | Official introduction to task spawning patterns |
| [Tokio graceful shutdown guide](https://tokio.rs/tokio/topics/shutdown) | Guide | Shutdown sequencing and signal handling |
| [tokio issue #7499](https://github.com/tokio-rs/tokio/issues/7499) | Issue | Critical gotcha: `spawn_blocking` during shutdown may not run closure |
| [spawn_blocking vs block_in_place](https://users.rust-lang.org/t/which-is-better-for-disk-bound-block-in-place-vs-spawn-blocking/73576) | Forum | Community consensus on when to use each |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-19 | Light external research on spawn_blocking patterns | Confirmed PRD approach is idiomatic; identified shutdown gotcha (tokio#7499) validating sync constraint |
| 2026-02-19 | Light internal research on codebase patterns | Found exact pattern in kill_process_group (agent.rs:304-333); mapped both call sites and constraints |

## Assumptions

- Light mode research was appropriate given the small scope and clear codebase precedent.
- No deep dives needed — the pattern is well-established and research raised no concerns.
- WRK-020 remains a duplicate as noted in the PRD.
