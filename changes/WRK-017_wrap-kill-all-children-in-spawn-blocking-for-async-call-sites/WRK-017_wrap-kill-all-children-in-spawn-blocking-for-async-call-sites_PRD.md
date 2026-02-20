# Change: Wrap kill_all_children in spawn_blocking for async call sites

**Status:** Proposed
**Created:** 2026-02-19
**Author:** phase-golem (autonomous)

## Problem Statement

`kill_all_children()` in `src/agent.rs` is a synchronous function that sends SIGTERM to all registered child process groups, polls for exit with `std::thread::sleep` (up to 5 seconds at 100ms intervals), then SIGKILLs survivors. It is called from two async functions — `handle_run` (main.rs:635) and `handle_triage` (main.rs:851) — where it blocks a tokio worker thread for the duration of the grace period.

Blocking a tokio worker thread with `std::thread::sleep` is a well-known anti-pattern: during the up-to-5-second grace period, any other async work scheduled on that worker thread is stalled. While the practical impact is low here (both call sites are near end-of-function cleanup and are mutually exclusive CLI subcommands), it violates the project's established pattern: the analogous `kill_process_group()` function (agent.rs:304) already uses `tokio::task::spawn_blocking` for the same SIGTERM-poll-SIGKILL logic.

The function itself must remain synchronous because it may be called during shutdown when the tokio runtime is unavailable (this constraint is documented in the WRK-001 SPEC). The fix is at the call sites, not the function definition.

## User Stories / Personas

- **Maintainer** — Wants consistent async hygiene across the codebase. The existing `spawn_blocking` pattern in `kill_process_group` sets the expectation; `kill_all_children` call sites should follow the same pattern.

## Desired Outcome

Both async call sites of `kill_all_children()` wrap the call in `tokio::task::spawn_blocking()` and `.await` the returned `JoinHandle`, so the blocking poll-and-sleep loop runs on the tokio blocking thread pool instead of a worker thread. The function signature and implementation remain unchanged.

## Success Criteria

### Must Have

- [ ] `handle_run` calls `kill_all_children()` via `spawn_blocking`, `.await`s the result, and handles `JoinHandle` errors
- [ ] `handle_triage` calls `kill_all_children()` via `spawn_blocking`, `.await`s the result, and handles `JoinHandle` errors
- [ ] Error handling uses `.await.unwrap_or_else(|e| ...)` with a warning log, matching the pattern in `kill_process_group()` (agent.rs:332)
- [ ] Errors from the `JoinHandle` are logged but not propagated, maintaining the same silent-failure semantics as the current direct call
- [ ] `kill_all_children()` function signature and body are not modified
- [ ] Existing tests pass without modification

## Scope

### In Scope

- Wrapping the two async call sites of `kill_all_children()` in `spawn_blocking`
- `.await`ing the `JoinHandle` and handling panics via `unwrap_or_else`

### Out of Scope

- Modifying the `kill_all_children()` function itself
- Making `kill_all_children()` async (it must remain sync for shutdown-path safety)
- Adding new tests (the change is mechanical and covered by existing integration tests)
- Refactoring `kill_all_children` to share code with `kill_process_group`
- Wrapping other blocking calls in `spawn_blocking` (tracked separately in WRK-019)

## Non-Functional Requirements

- **Performance:** No behavioral change. The blocking work moves from a worker thread to the blocking thread pool, freeing worker threads for other async tasks during cleanup.

## Constraints

- `kill_all_children()` must remain a synchronous `pub fn` — it is called from shutdown paths where the tokio runtime may be unavailable (documented in WRK-001 SPEC).
- The `spawn_blocking` wrapper is only safe at call sites where the tokio runtime is known to be running. Both `handle_run` and `handle_triage` are async functions executing within the `#[tokio::main]` runtime, so this is satisfied.
- Both call sites are mutually exclusive CLI subcommands, so concurrent invocation of `kill_all_children()` is not a concern.

## Dependencies

- **Depends On:** None
- **Blocks:** None

## Risks

- [ ] Minimal risk — the pattern is already established in the codebase (`kill_process_group`, git operations in `coordinator.rs`). The change is mechanical.

## Open Questions

None — the approach is well-defined and has existing precedent in the codebase.

## Assumptions

- WRK-020 ("Consider wrapping kill_all_children in spawn_blocking for async call sites") is a duplicate of this item. Both originated from WRK-001 (WRK-017 from build, WRK-020 from review). WRK-020 should be closed as duplicate when WRK-017 is completed.
- "Light" mode PRD was used since the change is small, well-scoped, and has clear precedent.

## References

- `src/agent.rs:71-113` — `kill_all_children()` definition
- `src/agent.rs:300-332` — `kill_process_group()` as existing `spawn_blocking` pattern
- `src/main.rs:635` — async call site in `handle_run`
- `src/main.rs:851` — async call site in `handle_triage`
- `src/coordinator.rs:573-648` — additional `spawn_blocking` usage patterns
