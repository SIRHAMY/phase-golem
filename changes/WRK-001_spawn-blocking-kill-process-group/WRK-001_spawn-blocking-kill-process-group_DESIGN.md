# Design: Wrap kill_process_group in spawn_blocking

**ID:** WRK-001
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-001_spawn-blocking-kill-process-group_PRD.md
**Tech Research:** ./WRK-001_tech-research.md
**Mode:** Light

## Overview

Make `kill_process_group` an async function that wraps its entire synchronous body inside `tokio::task::spawn_blocking`. This moves the blocking poll-and-sleep loop (up to 5 seconds of `std::thread::sleep`) off tokio worker threads and onto the dedicated blocking thread pool, while preserving the exact SIGTERM → poll → SIGKILL semantics. The two call sites in `run_subprocess_agent` add `.await`. This follows the established `spawn_blocking` pattern from `coordinator.rs`.

---

## System Design

### High-Level Architecture

This is a single-function refactor. No new components, modules, or data structures are introduced. The change affects one function signature and its two call sites within the same file (`agent.rs`).

```
Before:
  run_subprocess_agent (async) ──sync call──> kill_process_group (sync, blocks worker thread)

After:
  run_subprocess_agent (async) ──.await──> kill_process_group (async, delegates to blocking pool)
                                                └──spawn_blocking──> closure (sync body, runs on blocking thread)
```

### Component Breakdown

#### `kill_process_group` (modified)

**Purpose:** Gracefully terminate a process group (SIGTERM → poll → SIGKILL)

**Changes:**
- Signature: `fn kill_process_group(pgid: i32)` → `async fn kill_process_group(pgid: i32)`
- Body: Entire existing body moves into a `tokio::task::spawn_blocking(move || { ... })` closure
- Panic handling: `.await.unwrap_or_else(|e| log_warn!(...))` on the `JoinHandle`

**Interfaces:**
- Input: `pgid: i32` (process group ID, `Copy` type — moves into closure trivially)
- Output: `()` (unchanged — function is fire-and-forget cleanup)

**Dependencies:** `tokio::task::spawn_blocking` (already available via `tokio = { features = ["full"] }`)

### Data Flow

1. `run_subprocess_agent` determines a process group needs killing (timeout or shutdown signal)
2. Calls `kill_process_group(child_pid).await`
3. `kill_process_group` spawns a blocking task with the existing synchronous body
4. Blocking task runs on tokio's blocking thread pool: sends SIGTERM, polls with `std::thread::sleep`, sends SIGKILL if needed
5. `JoinHandle` resolves; if the blocking task panicked, log a warning and continue
6. Caller proceeds with `child.wait().await` and cleanup

### Key Flows

#### Flow: Timeout Kill (Happy Path)

> Process exceeds its timeout; orchestrator kills the process group and collects exit status.

1. **Timeout fires** — `tokio::time::timeout` returns `Err`
2. **Call kill** — `kill_process_group(child_pid).await` dispatches to blocking pool
3. **SIGTERM sent** — Blocking task sends SIGTERM to process group
4. **Poll loop** — Polls every 100ms (up to 5s) for process group exit via `killpg(pgid, None)`
5. **Process exits** — `killpg` returns `ESRCH`, blocking task returns
6. **Await completes** — `run_subprocess_agent` resumes on async worker
7. **Cleanup** — `child.wait().await`, `unregister_child(pgid)`

**Edge cases:**
- Process doesn't exit within 5s — SIGKILL sent, blocking task returns immediately
- Process already gone when SIGTERM sent — `killpg` returns `ESRCH` on the initial SIGTERM call, function returns immediately (never enters poll loop)
- Process dies between SIGTERM and first poll iteration — First `killpg(pgid, None)` check returns `ESRCH`, poll loop exits immediately
- Blocking task panics — `.unwrap_or_else` logs warning, caller continues cleanup. Process remains in the process registry and will be cleaned up by `kill_all_children` at shutdown

#### Flow: Shutdown Kill

> Shutdown signal received; orchestrator kills the process group before returning.

1. **Shutdown detected** — `is_shutdown_requested()` returns true
2. **Call kill** — `kill_process_group(child_pid).await` (same as above)
3. **Cleanup** — `child.wait().await`, return error

**Edge cases:** Same as timeout flow.

---

## Technical Decisions

### Key Decisions

#### Decision: Make `kill_process_group` async (vs. wrapping at call sites)

**Context:** The blocking body could be wrapped either inside the function or at each call site.

**Decision:** Make `kill_process_group` itself `async fn` with `spawn_blocking` inside.

**Rationale:**
- Encapsulates the async concern in the function, not scattered across call sites
- Two call sites would duplicate the wrapping and panic-handling logic
- Follows the principle that the function "knows" it does blocking work

**Consequences:** The function can no longer be called from synchronous code. This is fine — both call sites are async, and `kill_all_children` (synchronous caller) has its own separate implementation that must remain synchronous because it runs during process shutdown when the tokio runtime may be shutting down or unavailable.

#### Decision: Use `.unwrap_or_else` with `log_warn!` for panic handling

**Context:** `spawn_blocking` returns `JoinHandle<T>`; `.await` returns `Result<T, JoinError>` where `Err` indicates the spawned task panicked.

**Decision:** Use `.unwrap_or_else(|e| log_warn!("kill_process_group task panicked: {}", e))`, adapting the coordinator.rs `spawn_blocking` pattern.

**Rationale:**
- A panic inside `kill_process_group` is highly unlikely (simple signal/sleep operations)
- If it occurs, we're already on a cleanup/error path — crashing the caller is worse than logging and continuing
- Adapts the coordinator.rs pattern: coordinator uses `.unwrap_or_else(|e| Err(...))` because those calls return `Result`; here the function returns `()`, so logging is the appropriate equivalent

**Consequences:** In the extremely unlikely panic case, the process group may not be fully cleaned up. The process remains registered in the process registry, so `kill_all_children` at shutdown will attempt to terminate it as a safety net.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Additional thread | Spawning a blocking task uses a thread from the blocking pool, plus negligible (<1ms) context-switch overhead for task dispatch and await wakeup | Async worker thread stays free for other tasks | Blocking pool has 512 thread default; one thread for ~5s max is negligible. Currently max_wip=1, so at most one concurrent kill. If future concurrency work increases this, blocking pool pressure should be re-evaluated |
| Swallowed panics | A panic inside the blocking task is logged but not propagated | Cleanup code doesn't crash on an unlikely failure | Both call sites are already on error/cleanup paths |

---

## Alternatives Considered

### Alternative: Rewrite with `tokio::time::sleep`

**Summary:** Replace `std::thread::sleep` with `tokio::time::sleep` and make the polling loop fully async without `spawn_blocking`.

**How it would work:**
- Change polling loop to `loop { tokio::time::sleep(poll_interval).await; ... }`
- Function becomes `async` naturally without needing `spawn_blocking`

**Pros:**
- No blocking thread used at all
- Fully idiomatic async

**Cons:**
- Refactors the polling logic, not just wrapping it
- Changes timing characteristics subtly (async sleep vs. thread sleep)
- Larger diff, more risk of behavioral change
- PRD explicitly constrains to `spawn_blocking` approach

**Why not chosen:** PRD and tech research both specify `spawn_blocking` to keep the change minimal and consistent with `coordinator.rs` patterns. The async rewrite would be a larger change with marginal benefit.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Blocking task panic swallowed | Process group not fully cleaned up | Very Low | `kill_all_children` at shutdown provides safety net; `log_warn!` makes it visible |

---

## Integration Points

### Existing Code Touchpoints

- `agent.rs:kill_process_group` — Signature changes to `async fn`, body wrapped in `spawn_blocking`
- `agent.rs:run_subprocess_agent` (timeout path, ~line 237) — Add `.await` to `kill_process_group` call
- `agent.rs:run_subprocess_agent` (shutdown path, ~line 254) — Add `.await` to `kill_process_group` call

### External Dependencies

- None. `tokio::task::spawn_blocking` is already available.

---

## Open Questions

None. This is a straightforward mechanical refactor with clear precedent in the codebase.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Initial design draft | Light-mode design for mechanical spawn_blocking refactor |
| 2026-02-12 | Self-critique (7 agents) | ~39 raw findings → 9 unique after dedup. 6 auto-fixed (edge cases, panic/registry interaction, coordinator pattern divergence rationale, concurrency constraint, runtime availability). 0 directional. Remaining quality items deferred — not actionable for this small/low-complexity change. |
