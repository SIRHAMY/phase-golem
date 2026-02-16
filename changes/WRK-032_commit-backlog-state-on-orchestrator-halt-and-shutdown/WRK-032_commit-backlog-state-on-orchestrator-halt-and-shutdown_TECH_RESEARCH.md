# Tech Research: Commit backlog state on orchestrator halt and shutdown

**ID:** WRK-032
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-032_commit-backlog-state-on-orchestrator-halt-and-shutdown_PRD.md
**Mode:** Light

## Overview

Researching how to commit BACKLOG.yaml to git when the orchestrator halts. The core question is where in the shutdown sequence to insert the git commit, given the coordinator's actor pattern (tokio mpsc), the existing shutdown hook in `run_coordinator()`, and the fact that `spawn_coordinator()` does not return a JoinHandle.

## Research Questions

- [x] Where should the BACKLOG.yaml commit happen — in the coordinator actor, in `handle_run()`, or in the scheduler?
- [x] Does `spawn_coordinator()` return the JoinHandle for the actor task?
- [x] What existing git helper functions can we reuse?
- [x] Are there pitfalls with `spawn_blocking` during tokio shutdown?

---

## External Research

### Landscape Overview

Graceful shutdown in tokio applications follows a well-established three-phase pattern: detect shutdown signal, notify components, wait for completion. For our case — running a synchronous git commit after an actor completes — the key concern is ensuring the commit runs after the actor's cleanup but before the process exits.

### Common Patterns & Approaches

#### Pattern: JoinHandle Await + Synchronous Cleanup

**How it works:** Store the JoinHandle from `tokio::spawn`, await it before running final cleanup. Cleanup code runs synchronously after the actor task resolves.

**When to use:** When final cleanup (like git commit) must happen after actor shutdown completes.

**Tradeoffs:**
- Pro: Explicit ordering guarantee — actor finishes, then cleanup runs
- Pro: Simple, easy to reason about
- Con: Requires the JoinHandle to be returned from the spawn function

**References:**
- [Graceful Shutdown | Tokio](https://tokio.rs/tokio/topics/shutdown) — official shutdown guide
- [Actors with Tokio – Alice Ryhl](https://ryhl.io/blog/actors-with-tokio/) — actor pattern with shutdown

#### Pattern: Actor-Internal Cleanup Hook

**How it works:** The actor performs cleanup (including side effects like git commits) in its own shutdown path — after the message loop exits but before the spawned task's future resolves.

**When to use:** When the actor already has a shutdown hook and the cleanup naturally belongs inside the actor (it has all necessary state).

**Tradeoffs:**
- Pro: Self-contained — no changes needed in callers
- Pro: Runs on all shutdown paths automatically
- Con: Mixes actor concerns (state management) with infrastructure concerns (git commits)
- Con: Errors in cleanup can't be observed by the caller (JoinHandle is discarded)

**References:**
- [Actors with Tokio – Alice Ryhl](https://ryhl.io/blog/actors-with-tokio/) — shutdown patterns

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| `spawn_blocking` during runtime shutdown may not execute | Known tokio issue ([#7499](https://github.com/tokio-rs/tokio/issues/7499)) — closures may be dropped without running | Perform git operations synchronously when possible in the shutdown path, or ensure runtime is still alive |
| Dropping JoinHandle detaches the task | You lose the ability to await completion | Store and await JoinHandle before cleanup |
| `process::exit()` skips destructors | Cleanup code never runs | Use normal function returns; let `main` exit naturally |
| Blocking git operations in async context | `std::process::Command` blocks the tokio runtime thread | Use `spawn_blocking` (already done in `batch_commit`) |

### Key Learnings

- The existing codebase already uses `spawn_blocking` for git operations in `batch_commit()`, so the pattern is established.
- The `spawn_blocking` shutdown issue is worth knowing about but unlikely to affect us since the coordinator actor loop has already exited cleanly before cleanup runs.
- No need for external crates (git2, gitoxide) — the existing `std::process::Command` wrappers in `git.rs` are sufficient.

---

## Internal Research

### Existing Codebase State

The orchestrator uses an actor-based coordinator pattern:

1. `handle_run()` spawns the coordinator via `spawn_coordinator()` → returns `CoordinatorHandle` (sender only)
2. `run_scheduler()` uses the handle to send commands, calls `batch_commit()` before each halt
3. When `run_scheduler()` returns, the `CoordinatorHandle` goes out of scope, the mpsc sender drops
4. The coordinator actor exits its message loop, runs `save_backlog()` (line 683), then the task future resolves
5. `handle_run()` prints summary and returns

**Critical finding:** `spawn_coordinator()` discards the JoinHandle (coordinator.rs:697-704). The handle is not stored or returned, so `handle_run()` cannot currently await the actor task.

**Relevant files/modules:**

- `orchestrator/src/main.rs:426-432` — spawns coordinator, gets `CoordinatorHandle`
- `orchestrator/src/main.rs:458` — calls `run_scheduler()`, which consumes the handle
- `orchestrator/src/main.rs:460-490` — post-scheduler: kills children, prints summary, returns
- `orchestrator/src/coordinator.rs:688-707` — `spawn_coordinator()` discards JoinHandle
- `orchestrator/src/coordinator.rs:682-683` — shutdown hook: `save_backlog()` after message loop exits
- `orchestrator/src/coordinator.rs:594-617` — `batch_commit()` handler with `spawn_blocking` pattern
- `orchestrator/src/git.rs:77-93` — `stage_paths()`: stages explicit file paths
- `orchestrator/src/git.rs:96-104` — `commit()`: creates commit with message, returns SHA
- `orchestrator/src/git.rs:107-127` — `get_status()`: parses `git status --porcelain`
- `orchestrator/src/scheduler.rs:37-46` — `HaltReason` enum (8 variants)
- `orchestrator/src/backlog.rs:66-94` — atomic save: temp file + fsync + rename

**Existing patterns in use:**

- `batch_commit()` uses `spawn_blocking` to run git operations from async context
- Git operations always use explicit paths (never `git add -A`)
- `get_status()` + check for staged changes before committing (avoids empty commits)
- Commit messages use `[prefix]` format for phase commits

### Reusable Components

- `crate::git::stage_paths(&[&Path], Option<&Path>)` — stage BACKLOG.yaml
- `crate::git::commit(&str, Option<&Path>)` — create the commit
- `crate::git::get_status(Option<&Path>)` — check if BACKLOG.yaml is dirty
- `CoordinatorState.project_root` — available in the shutdown hook
- `CoordinatorState.backlog_path` — path to BACKLOG.yaml

### Constraints from Existing Code

- `spawn_coordinator()` discards JoinHandle — to await from `handle_run()`, the function signature must change
- The coordinator shutdown hook (line 682-683) runs synchronously in the actor task — git calls here would need `spawn_blocking` but the actor is already inside a spawned task on the tokio runtime
- `run_scheduler()` **moves** the `CoordinatorHandle` (it takes ownership) — the handle is dropped when the scheduler returns, triggering coordinator shutdown

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| "Adding a final BACKLOG.yaml commit step in `handle_run()` shutdown path" | `spawn_coordinator()` discards JoinHandle; `handle_run()` has no way to await coordinator completion | Either return JoinHandle from `spawn_coordinator()`, or commit inside the coordinator's own shutdown hook |
| "Ensuring the coordinator actor task is awaited (not just handle-dropped)" | JoinHandle is discarded at spawn time | Must modify `spawn_coordinator()` to return `(CoordinatorHandle, JoinHandle)` if committing from `handle_run()` |
| Commit happens "after coordinator's `save_backlog()` completes" | The coordinator's shutdown hook runs `save_backlog()` at line 683; after this line, we could also commit | If committing inside the coordinator hook, ordering is trivially correct |
| Shutdown commit uses `spawn_blocking` | `spawn_blocking` during runtime shutdown may not execute ([tokio#7499](https://github.com/tokio-rs/tokio/issues/7499)) | If committing inside the coordinator (which is itself a spawned task), the runtime is still alive and `spawn_blocking` is safe; but simpler to call git synchronously since we're already in a blocking-compatible context |

---

## Critical Areas

### Ordering: `save_backlog()` must complete before git commit

**Why it's critical:** If the git commit runs before `save_backlog()`, it captures stale BACKLOG.yaml state.

**Why it's easy to miss:** If committing from `handle_run()`, there's a race: the scheduler returns (dropping the handle), but the coordinator task may still be executing `save_backlog()`.

**What to watch for:** Either (a) commit inside the coordinator after `save_backlog()` returns, or (b) await the coordinator's JoinHandle from `handle_run()` before committing. Both guarantee ordering.

### Git operations in the coordinator shutdown hook are blocking

**Why it's critical:** The coordinator runs inside `tokio::spawn`. Blocking calls (git via `std::process::Command`) will block a runtime worker thread.

**Why it's easy to miss:** `save_backlog()` is already fast (write + fsync + rename), but git operations may be slower.

**What to watch for:** Use `spawn_blocking` for git operations even in the shutdown hook, or accept the brief block since this is shutdown and no other work is running. The `batch_commit` handler already uses `spawn_blocking` as the pattern.

---

## Deep Dives

*None needed for light research.*

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| Should the commit happen inside the coordinator or in `handle_run()`? | Determines whether `spawn_coordinator()` needs to return a JoinHandle | See recommended approaches below |

### Recommended Approaches

#### Where to commit BACKLOG.yaml on shutdown

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| **A: Commit inside coordinator shutdown hook** | No API changes; runs on all shutdown paths; ordering trivially correct (after `save_backlog()`) | Mixes git concerns into coordinator; errors not observable by caller; uses `spawn_blocking` in shutdown (but runtime is alive) | Minimal change, self-contained |
| **B: Return JoinHandle, commit from `handle_run()`** | Separation of concerns; caller controls commit message (can include halt reason); errors are observable | Requires changing `spawn_coordinator()` return type; must thread JoinHandle through or around scheduler; more files touched | Clean architecture, PRD's original intent |

**Initial recommendation:** **Approach B** (return JoinHandle, commit from `handle_run()`).

Rationale:
- The PRD explicitly wants the halt reason in the commit message — this is only available in `handle_run()` from the scheduler's `RunSummary`
- Git commit is an infrastructure concern that belongs at the orchestrator level (`handle_run()`), not inside the coordinator actor
- The change to `spawn_coordinator()` is small (return tuple instead of just handle) and makes the shutdown sequence explicit and testable
- `run_scheduler()` takes ownership of the `CoordinatorHandle`, but the JoinHandle can be kept separately in `handle_run()`
- After `run_scheduler()` returns, the `CoordinatorHandle` is dropped → coordinator receives shutdown → `save_backlog()` runs → actor task completes → `handle_run()` awaits JoinHandle → then commits. The ordering is naturally correct.

The implementation is approximately:
1. `spawn_coordinator()` returns `(CoordinatorHandle, JoinHandle<()>)` — ~3 lines changed
2. `handle_run()` keeps the JoinHandle, passes only the `CoordinatorHandle` to `run_scheduler()`
3. After `run_scheduler()` returns, await the JoinHandle (coordinator finishes `save_backlog()`)
4. Check if BACKLOG.yaml is dirty via `get_status()`, filter for BACKLOG.yaml path
5. If dirty: `stage_paths()` + `commit()` with message including halt reason
6. Wrap in warn-and-continue error handling

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Graceful Shutdown - Tokio](https://tokio.rs/tokio/topics/shutdown) | Docs | Official patterns for coordinated shutdown |
| [Actors with Tokio](https://ryhl.io/blog/actors-with-tokio/) | Article | Actor shutdown patterns, JoinHandle handling |
| [tokio#7499 - spawn_blocking during shutdown](https://github.com/tokio-rs/tokio/issues/7499) | Issue | Known pitfall to be aware of |
| [JoinHandle docs](https://docs.rs/tokio/latest/tokio/task/struct.JoinHandle.html) | Docs | Behavior when dropped vs awaited |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Light external research: tokio shutdown patterns, git commit patterns | Identified JoinHandle await + synchronous cleanup as best fit |
| 2026-02-13 | Medium internal research: coordinator, main, git, scheduler, backlog | Found `spawn_coordinator()` discards JoinHandle; mapped all integration points |
| 2026-02-13 | PRD analysis | Identified JoinHandle gap; recommended Approach B (return JoinHandle) |
