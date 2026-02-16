# Technical Research: WRK-001 — Wrap kill_process_group in spawn_blocking

**Date:** 2026-02-12
**Status:** Complete

## Problem

`kill_process_group` (agent.rs:288-313) uses `std::thread::sleep` in a polling loop that blocks a tokio worker thread for up to 5 seconds. It is called from two sites within the async function `run_subprocess_agent`:

1. **Line 237** — timeout path
2. **Line 254** — shutdown signal path

Both calls block the tokio worker thread, violating the async contract.

## Approach: `tokio::task::spawn_blocking`

### Why spawn_blocking

The PRD constrains the solution to `spawn_blocking` rather than rewriting with `tokio::time::sleep`. This is the right choice because:

1. **Minimal change** — The entire synchronous function body moves into the closure unchanged. No logic refactoring needed.
2. **Existing pattern** — coordinator.rs:556-572 already uses `spawn_blocking` for blocking git operations, establishing a codebase convention.
3. **Preserves exact semantics** — The SIGTERM → poll with `std::thread::sleep` → SIGKILL behavior is identical. No timing changes.
4. **Thread pool is appropriate** — tokio's blocking thread pool is designed for exactly this: short-lived blocking operations (up to 5s here) that shouldn't occupy async worker threads.

### Implementation Strategy

**Option A: Make `kill_process_group` async (recommended)**

Change the function signature to `async fn kill_process_group(pgid: i32)` and wrap the body in `spawn_blocking` inside the function. The two call sites add `.await`.

```rust
async fn kill_process_group(pgid: i32) {
    tokio::task::spawn_blocking(move || {
        // ... existing body unchanged ...
    })
    .await
    .unwrap_or_else(|e| {
        log_warn!("kill_process_group task panicked: {}", e);
    });
}
```

**Option B: Wrap at call sites**

Keep `kill_process_group` synchronous and wrap at each call site:

```rust
let pid = child_pid;
tokio::task::spawn_blocking(move || kill_process_group(pid))
    .await
    .unwrap_or_else(|e| {
        log_warn!("kill_process_group task panicked: {}", e);
    });
```

**Recommendation: Option A** — Encapsulates the async concern in the function itself, keeping call sites cleaner. Two call sites means the wrapping logic would be duplicated in Option B.

### Panic Handling

Following the coordinator.rs pattern, use `.unwrap_or_else` on the `JoinHandle`. A panic inside `kill_process_group` would mean something went very wrong with signal delivery; logging a warning and continuing is appropriate since the process is already in a cleanup/error path. The alternative (`.unwrap()`) would propagate the panic to the caller, which is undesirable in cleanup code.

### Move Semantics

`kill_process_group` takes `pgid: i32` — a `Copy` type. Moving it into the `spawn_blocking` closure requires no cloning. The `use nix::sys::signal::{killpg, Signal}` import inside the function body works fine inside the closure.

## Key Details

### Constants (unchanged)
- `SIGTERM_GRACE_PERIOD_SECONDS = 5` — max blocking duration
- `KILL_POLL_INTERVAL_MS = 100` — poll interval (up to 50 iterations)

### Dependencies (no changes needed)
- `tokio` is already in Cargo.toml with `features = ["full"]`, which includes `task::spawn_blocking`
- No new dependencies required

### What NOT to change
- `kill_all_children` (lines 71-117) — called from synchronous shutdown context, must remain synchronous
- The polling logic itself — no refactoring, no timing changes
- Constants — no changes to grace period or poll interval

## Test Impact

Existing tests that exercise process killing:

1. **subprocess_timeout_kills_process** (agent_test.rs:153-179) — Tests timeout → kill path. Will exercise the new async `kill_process_group`. Timing assertions (< 15s) remain valid since wall-clock behavior is unchanged.

2. **process_group_kill_cleans_up_subprocess** (agent_test.rs:345-376) — Tests process group cleanup. Same timing characteristics.

Both tests should pass without modification since the observable behavior (timing, kill semantics) is identical.

## Risks

- **Very low risk.** This is a mechanical refactor: move a synchronous function body into `spawn_blocking` and await. The kill logic is entirely unchanged.
- The only new failure mode is a panic inside the blocking task, which is handled by `.unwrap_or_else` with a log warning.

## Assumptions

- The tokio runtime is always available when `kill_process_group` is called (both call sites are inside `run_subprocess_agent`, an async function — this is guaranteed).
- `spawn_blocking` will not be starved. The default blocking thread pool (512 threads) is more than sufficient for this use case.
