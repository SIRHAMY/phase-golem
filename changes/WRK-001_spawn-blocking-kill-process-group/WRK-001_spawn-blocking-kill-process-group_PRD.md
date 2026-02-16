# Change: Wrap kill_process_group sleep in spawn_blocking

**Status:** Proposed
**Created:** 2026-02-12
**Author:** Claude (autonomous)

## Problem Statement

The `kill_process_group` function in `agent.rs` uses `std::thread::sleep()` in a polling loop that can block a tokio worker thread for up to 5 seconds. This function is called from the async `run_subprocess_agent` function on two paths: timeout handling and shutdown signal handling.

The polling loop sends SIGTERM to a process group, then polls every 100ms (up to 50 iterations over 5 seconds) waiting for the process group to exit before sending SIGKILL. Each `std::thread::sleep(100ms)` call blocks the tokio worker thread, preventing it from servicing other async tasks.

While the orchestrator currently runs one agent at a time (max_wip=1), this is a correctness issue — blocking tokio worker threads violates the async contract and can interfere with signal checking, timeout tracking, and future concurrent execution.

The same pattern exists in `kill_all_children`, which is called from synchronous shutdown context and does not need to be changed.

## User Stories / Personas

- **Orchestrator Developer** — Wants the async runtime to function correctly without worker thread starvation, especially as concurrency features are added in future work.

## Desired Outcome

When this change is complete, `kill_process_group` runs its blocking poll-and-sleep loop on tokio's blocking thread pool via `spawn_blocking`, freeing the async worker thread to process other tasks during the up-to-5-second grace period.

The observable behavior of process killing (SIGTERM → poll → SIGKILL) remains identical.

## Success Criteria

### Must Have

- [ ] `kill_process_group` body runs inside `tokio::task::spawn_blocking`
- [ ] The calling async code awaits the `spawn_blocking` result
- [ ] No `std::thread::sleep` calls execute on tokio worker threads
- [ ] SIGTERM → poll → SIGKILL behavior is preserved exactly
- [ ] Existing tests pass without modification
- [ ] `cargo clippy` produces no new warnings

### Should Have

- [ ] Follow the existing `spawn_blocking` pattern from `coordinator.rs` (`.unwrap_or_else` for panic handling)

### Nice to Have

- [ ] (None — this is a minimal change)

## Scope

### In Scope

- Wrapping the `kill_process_group` function body in `spawn_blocking`
- Making the function `async` or wrapping calls at the call sites
- Adjusting the two call sites in `run_subprocess_agent`

### Out of Scope

- Changing `kill_all_children` (called from synchronous context during final shutdown)
- Refactoring the polling logic itself (e.g., replacing with tokio::time::sleep)
- Adding new tests beyond verifying existing tests still pass
- Changing the grace period duration or poll interval

## Non-Functional Requirements

- **Performance:** No measurable performance change. The blocking work moves to a dedicated thread pool instead of a worker thread, but the total wall-clock time for process killing remains the same.

## Constraints

- Must use `tokio::task::spawn_blocking` (not `tokio::time::sleep` or other async alternatives) to keep the change minimal and consistent with existing patterns in the codebase.
- The `kill_all_children` function must remain synchronous — it is called during final process cleanup where the async runtime may not be fully available.

## Dependencies

- **Depends On:** Nothing — standalone fix
- **Blocks:** Nothing directly, but improves runtime correctness for WRK-004 and future concurrency work

## Risks

- [ ] **Minimal risk:** The change is mechanical — move existing blocking code into `spawn_blocking` and await it. The kill logic itself is unchanged.

## Assumptions

- The `kill_process_group` function does not need to remain synchronous. Both call sites are already in an async context (`run_subprocess_agent` is `async fn`).
- Using `spawn_blocking` rather than rewriting with `tokio::time::sleep` is the right approach because it keeps the change minimal, matches existing codebase patterns (coordinator.rs), and preserves the exact polling semantics.
- `kill_all_children` is intentionally left unchanged since it runs during synchronous shutdown cleanup.

## References

- `agent.rs:288-313` — current `kill_process_group` implementation
- `agent.rs:237,254` — call sites in `run_subprocess_agent`
- `coordinator.rs:558-571` — existing `spawn_blocking` pattern to follow
