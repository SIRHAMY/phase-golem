# Design: Wrap kill_all_children in spawn_blocking for async call sites

**ID:** WRK-017
**Status:** Complete
**Created:** 2026-02-19
**PRD:** ./WRK-017_wrap-kill-all-children-in-spawn-blocking-for-async-call-sites_PRD.md
**Tech Research:** ./WRK-017_wrap-kill-all-children-in-spawn-blocking-for-async-call-sites_TECH_RESEARCH.md
**Mode:** Light

## Overview

Wrap both async call sites of `kill_all_children()` in `tokio::task::spawn_blocking` to move the blocking SIGTERM-poll-SIGKILL loop off the tokio worker thread pool. This mirrors the error handling pattern used by `kill_process_group()` (agent.rs:304-333) — `spawn_blocking` + `.await.unwrap_or_else` with a warning log — but wraps at the call site rather than inside the function definition, since `kill_all_children()` must remain synchronous for shutdown-path safety. No changes to the function itself are required.

---

## System Design

### High-Level Architecture

No new components or architectural changes. The existing `kill_all_children()` function remains a synchronous `pub fn`. The only change is how it is invoked from async contexts:

```
Before:  async fn handle_run/handle_triage → kill_all_children() [blocks worker thread]
After:   async fn handle_run/handle_triage → spawn_blocking(kill_all_children) → .await [blocks pool thread]
```

### Component Breakdown

No new components. Two existing call sites are modified.

#### Call Site: `handle_run` (main.rs:635)

**Purpose:** Cleanup after scheduler completes — kill remaining child processes before coordinator shutdown.

**Current code:**
```rust
kill_all_children();
```

**New code:**
```rust
tokio::task::spawn_blocking(move || {
    kill_all_children();
})
.await
.unwrap_or_else(|e| log_warn!("kill_all_children task panicked: {}", e));
```

#### Call Site: `handle_triage` (main.rs:851)

**Purpose:** Cleanup after triage loop completes — kill remaining child processes before final log.

**Current code:**
```rust
kill_all_children();
```

**New code:**
```rust
tokio::task::spawn_blocking(move || {
    kill_all_children();
})
.await
.unwrap_or_else(|e| log_warn!("kill_all_children task panicked: {}", e));
```

### Data Flow

No data flow changes. `kill_all_children()` takes no parameters and returns `()`. The `spawn_blocking` wrapper adds a `JoinHandle<()>` intermediary that is `.await`ed and unwrapped.

### Key Flows

#### Flow: Async cleanup via spawn_blocking

> Move the blocking kill-all-children loop to the tokio blocking thread pool during async cleanup.

1. **Async function reaches cleanup** — `handle_run` or `handle_triage` finishes its main work
2. **spawn_blocking dispatches closure** — Tokio schedules the closure on the blocking thread pool
3. **Closure executes kill_all_children()** — SIGTERM → poll (up to 5s) → SIGKILL runs on a blocking pool thread, not a worker thread
4. **.await completes** — The async function resumes when the blocking work finishes
5. **unwrap_or_else handles panics** — If `kill_all_children()` panics, a warning is logged and execution continues

**Edge cases:**
- `kill_all_children()` panics — Tokio captures the panic into a `JoinError`. The `.await` returns `Err(JoinError)`, and `unwrap_or_else` logs a warning via `log_warn!`. Execution continues normally (next statement runs). Note: this is a behavioral improvement over the current direct call, where a panic would crash the process.
- No child processes registered — `kill_all_children()` returns immediately, blocking time is negligible

---

## Technical Decisions

### Key Decisions

#### Decision: Use `spawn_blocking` + `unwrap_or_else` with `log_warn!`

**Context:** Need to run blocking cleanup in async context without stalling worker threads.

**Decision:** Use the same error handling pattern as `kill_process_group()` (agent.rs:304-333).

**Rationale:** This pattern is already established, reviewed, and working in the codebase. Consistency reduces cognitive load for maintainers.

**Consequences:** Panics are caught by the `JoinHandle` and logged via `log_warn!` instead of crashing the process. This is a behavioral improvement over the current direct call (where a panic would be unhandled and crash the process), but worth noting as a semantic change: cleanup now always continues rather than aborting on panic. This is appropriate for cleanup paths where recovery from a panic is not meaningful.

#### Decision: Use `move` keyword on closure

**Context:** `kill_all_children()` takes no parameters. The closure captures nothing, so `move` is technically unnecessary — both `|| { ... }` and `move || { ... }` compile identically.

**Decision:** Use `move || { ... }` to match the established `kill_process_group()` pattern (agent.rs:305).

**Rationale:** Consistency with the existing codebase pattern is more valuable than omitting a no-op keyword. Using `move` is idiomatic for `spawn_blocking` closures (signals the closure moves to another thread) and reduces cognitive friction for maintainers reading both patterns.

**Consequences:** None — the generated code is identical either way.

#### Decision: Keep `kill_all_children()` synchronous

**Context:** The function is called from shutdown paths where the tokio runtime may be unavailable (WRK-001 constraint).

**Decision:** Do not modify the function signature or body. Only change the call sites.

**Rationale:** The PRD and tech research both confirm this constraint. Making the function async would break shutdown-path callers.

**Consequences:** The function can be called directly in sync contexts and via `spawn_blocking` in async contexts.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Thread pool usage | Each cleanup call uses a blocking pool thread for up to 5 seconds | Worker threads stay free for async work during cleanup | Blocking pool is designed for exactly this; 5s is bounded and acceptable |
| Panic absorption | Panics in kill_all_children are caught and logged instead of crashing the process | Cleanup always continues; the process doesn't abort during teardown | Current direct call would crash on panic (unhandled). The new behavior is safer for cleanup paths where recovery is not meaningful. `kill_all_children()` uses defensive patterns (early returns on mutex lock failure, `let _ =` on signal errors) making panic unlikely. |

---

## Alternatives Considered

### Alternative: `block_in_place`

**Summary:** Use `tokio::task::block_in_place` instead of `spawn_blocking` to run the blocking code on the current worker thread.

**How it would work:**
- Call `block_in_place(|| kill_all_children())` at each call site
- No new thread spawned; current worker thread is temporarily repurposed

**Pros:**
- No thread spawn overhead

**Cons:**
- Not used for this pattern anywhere in the codebase
- Hurts cache locality for other tasks migrated away from the worker
- Incompatible with `current_thread` runtime (less portable)

**Why not chosen:** `spawn_blocking` is the established codebase pattern. `block_in_place` offers no meaningful benefit for a cleanup call and breaks consistency.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| `spawn_blocking` closure dropped during shutdown | `kill_all_children()` might not run | Very Low | Both call sites are inside async functions called from `match cli.command { ... }` within the `#[tokio::main]` async main. `handle_run` calls cleanup at line 635, before `coord_task.await` — the runtime is fully active. `handle_triage` calls cleanup at line 851, after `drop(coordinator_handle)` but still within the `#[tokio::main]` async main — the runtime is active. |

---

## Integration Points

### Existing Code Touchpoints

- `src/main.rs:635` — `handle_run`: wrap `kill_all_children()` call in `spawn_blocking`
- `src/main.rs:851` — `handle_triage`: wrap `kill_all_children()` call in `spawn_blocking`
- `src/agent.rs:304-333` — `kill_process_group()`: reference pattern (not modified)

### External Dependencies

None. Uses only `tokio::task::spawn_blocking` which is already a dependency.

---

## Open Questions

None.

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
| 2026-02-19 | Initial design draft | Straightforward wrapping pattern matching kill_process_group; light mode |
| 2026-02-19 | Self-critique (7 agents) and auto-fixes | Fixed: panic semantics characterization (behavioral change, not matching current), `move` keyword for consistency, "exactly mirrors" wording, runtime safety verification with code-level evidence. No directional issues found. |
