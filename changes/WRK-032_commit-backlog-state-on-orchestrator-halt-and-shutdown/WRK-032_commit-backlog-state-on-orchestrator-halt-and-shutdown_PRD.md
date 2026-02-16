# Change: Commit backlog state on orchestrator halt and shutdown

**Status:** Proposed
**Created:** 2026-02-13
**Author:** Orchestrator (autonomous)

## Problem Statement

The orchestrator writes BACKLOG.yaml to disk whenever state changes occur (phase completions, status transitions, block reasons, follow-up ingestion), but BACKLOG.yaml disk writes are only committed to git as part of phase completion commits — immediately for destructive phases (phases that modify code, like build) and batched for non-destructive phases (phases that produce documents, like prd, design, review).

When the orchestrator halts (execution cap reached, circuit breaker tripped after consecutive failures, shutdown signal, all items blocked/done, target completed, filter exhausted), `batch_commit()` flushes any pending non-destructive phase work. However, `batch_commit()` only creates a commit if `pending_batch_phases` is non-empty. It does not check whether BACKLOG.yaml itself has uncommitted changes that occurred outside phase completions — such as status transitions, block reasons from guardrail checks (automated risk/size/complexity thresholds), or follow-up item ingestion.

This creates a durability gap: these state changes are written to BACKLOG.yaml on disk but never committed to git. A subsequent `git checkout`, `git stash`, another orchestrator run, or any file-level reset silently loses the updated state.

**Observed in practice:** WRK-029 (a backlog item) was blocked by guardrail checks. The block reason and status transition were written to BACKLOG.yaml on disk, but the orchestrator halted without committing the change. The updated state was invisible to git history and vulnerable to being lost.

## User Stories / Personas

- **Orchestrator operator** — Runs the orchestrator to process backlog items. Expects that when the orchestrator reports an item is blocked or a cap is reached, that state is durable across runs. Currently must remember to manually `git add BACKLOG.yaml && git commit` after every halt, or risk losing orchestrator decisions.

## Desired Outcome

When the orchestrator halts for any reason, BACKLOG.yaml is committed to git if it has uncommitted changes (staged or unstaged). The commit happens after the coordinator's `save_backlog()` completes and after any pending batch commits are flushed. The operator can trust that the git history reflects the orchestrator's final state decisions without manual intervention.

## Success Criteria

### Must Have

- [ ] BACKLOG.yaml is committed to git on orchestrator halt if it has staged or unstaged changes, across all 8 halt reasons (AllDoneOrBlocked, CapReached, CircuitBreakerTripped, ShutdownRequested, TargetCompleted, TargetBlocked, FilterExhausted, NoMatchingItems)
- [ ] The commit only occurs if BACKLOG.yaml is actually dirty — no empty commits are created
- [ ] The commit happens after the coordinator actor task completes (which runs `save_backlog()` during shutdown), ensuring the latest state is captured
- [ ] Git failures during the shutdown commit are logged as warnings (via `log_warn!`) but do not prevent clean process exit
- [ ] The commit message uses the format `[orchestrator] Save backlog state on halt` to distinguish shutdown commits from phase commits
- [ ] The shutdown commit stages only BACKLOG.yaml, even if other files are dirty in the working tree
- [ ] If the coordinator's `save_backlog()` fails during shutdown, the git commit is skipped (no stale state committed)

### Should Have

- [ ] The halt reason is included in the commit message for diagnostic value (e.g., `[orchestrator] Save backlog state on halt (CapReached)`)
- [ ] The shutdown commit is logged via `log_info!` so the operator can see it happened

### Nice to Have

- [ ] None currently identified

## Scope

### In Scope

- Adding a final BACKLOG.yaml commit step in the `handle_run()` shutdown path in main.rs
- Checking git status for BACKLOG.yaml specifically before committing
- Ensuring the coordinator actor task is awaited (not just handle-dropped) so `save_backlog()` completes before the git commit runs
- Non-fatal error handling for git operations during shutdown

### Out of Scope

- Committing BACKLOG.yaml in other command paths (`status`, `add`, `advance`, `unblock`) — these are synchronous commands that already save to disk and can be addressed separately
- Committing BACKLOG.yaml in the `handle_triage()` path — triage uses immediate commits (destructive flag) for its phases and can be addressed as a follow-up
- Committing `_worklog/` entries in the shutdown commit — worklog writes are committed via phase completion commits; any missed entries can be committed on the next run
- Panic/SIGKILL recovery — these prevent cleanup code from running entirely
- Transactional guarantees between disk write and git commit — the atomic save pattern (temp file + fsync + rename) already ensures disk consistency; the git commit is an additional durability layer
- Mid-run BACKLOG.yaml commits — too noisy; `batch_commit()` already handles phase output commits
- Changes to the coordinator's internal commit logic — git commits remain a scheduler/main-level concern
- Committing other dirty files (changes/ directories, etc.) — only BACKLOG.yaml is committed

## Constraints

- The commit must happen after the coordinator's `save_backlog()` on shutdown. The coordinator runs as a tokio actor; `save_backlog()` executes when the actor's mpsc channel closes (all senders dropped). The implementation must await the coordinator's spawned task to completion before running the git commit, not just drop the handle.
- Git operations are synchronous (`std::process::Command`). In the shutdown path, git calls should use `tokio::task::spawn_blocking` if still in an async context, or run synchronously if the async runtime is no longer needed.
- The existing `batch_commit()` is a coordinator command sent over the mpsc channel. The shutdown commit must call git functions directly from main.rs since the coordinator has already shut down.

## Dependencies

- **Depends On:** None — uses existing `git::stage_paths()`, `git::commit()`, and `git::get_status()` functions
- **Blocks:** None
- **Implementation dependency:** `spawn_coordinator()` may need to return the `JoinHandle` for the spawned actor task so `handle_run()` can await it before committing

## Risks

- [ ] Race condition between coordinator shutdown save and git commit: if the git commit runs before `save_backlog()` completes, the commit captures stale state. Mitigation: await the coordinator actor task to completion before running the git commit.
- [ ] Signal handler interruption during shutdown commit: a second SIGTERM/SIGKILL during the commit could leave `.git/index.lock` (a file git uses to prevent concurrent operations). Mitigation: accept this as a known edge case; manual removal of `.git/index.lock` is standard recovery.
- [ ] Git precondition violation: if the repository enters an unexpected state (mid-rebase, detached HEAD) during the run, the shutdown commit may fail. Mitigation: the orchestrator validates git preconditions at startup; failures during shutdown are logged and tolerated.

## Assumptions

- The `handle_run()` path is the primary (and currently only) path that needs this fix, since it's the only long-running orchestrator mode where state accumulates across multiple phase executions
- Including only BACKLOG.yaml (not all dirty files) is correct because the orchestrator should only commit state it owns; other dirty files may be intentional user work or will be committed by their respective phase commits
- A warn-and-continue error handling strategy is preferred over fail-fast, since the backlog is already saved to disk and the git commit is a durability bonus, not a correctness requirement
- No configuration flag is needed — always committing BACKLOG.yaml on halt is the expected default behavior
- The orchestrator process lock guarantees no other orchestrator instance is running, so concurrent writes to BACKLOG.yaml are not possible
- BACKLOG.yaml is tracked by git and not in `.gitignore`
- Dropping the coordinator handle and awaiting the actor task to completion is sufficient to guarantee `save_backlog()` has run — no explicit `shutdown()` method is needed since the actor's cleanup code runs synchronously before the task future resolves
- The expected shutdown sequence is: scheduler halts → `batch_commit()` flushes pending phases → coordinator handle dropped → coordinator actor runs `save_backlog()` and exits → actor task awaited → git dirty check on BACKLOG.yaml → git commit if dirty → print summary → process exits

## Open Questions

- [ ] Does `spawn_coordinator()` currently return the `JoinHandle` for the actor task, or does this need to be added? (Implementation detail to verify during SPEC/build)

## References

- `orchestrator/src/main.rs` — `handle_run()` shutdown path
- `orchestrator/src/coordinator.rs` — coordinator shutdown save, `batch_commit()`
- `orchestrator/src/git.rs` — git helper functions (`stage_paths`, `commit`, `get_status`)
- `orchestrator/src/scheduler.rs` — `HaltReason` enum and scheduler exit points
- `orchestrator/src/backlog.rs` — atomic save pattern (temp file + fsync + rename)
