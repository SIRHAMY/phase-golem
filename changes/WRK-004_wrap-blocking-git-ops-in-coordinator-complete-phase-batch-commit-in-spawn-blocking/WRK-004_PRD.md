# Change: Wrap Blocking Git Ops in spawn_blocking

**Status:** Draft
**Created:** 2026-02-12
**Author:** Orchestrator (autonomous)

## Problem Statement

The orchestrator's coordinator runs an async actor loop (`run_coordinator`) that processes commands on a tokio runtime. Two handler functions — `handle_complete_phase` and `handle_batch_commit` — make synchronous git calls (`get_status`, `stage_paths`, `commit`) directly on the async executor thread. These `std::process::Command`-based calls block the thread while waiting for the git subprocess to complete, preventing the tokio runtime from driving other async tasks during that time.

Other git operations in the same actor loop (`GetHeadSha`, `IsAncestor`) are already correctly wrapped in `tokio::task::spawn_blocking`, creating an inconsistency in the codebase. The fix should bring `complete_phase` and `batch_commit` into alignment with the established pattern.

## User Stories / Personas

- **Orchestrator developer** — Wants consistent async patterns across the coordinator so that all synchronous git operations use `spawn_blocking` uniformly, making it immediately clear which operations block and which don't.
- **Orchestrator user** — Wants the orchestrator to remain responsive during git operations, especially when `complete_phase` runs potentially slow operations like `git add` and `git commit` on large changesets.

## Desired Outcome

All synchronous git operations within the coordinator's async actor loop are wrapped in `tokio::task::spawn_blocking`, preventing them from blocking the tokio runtime thread. The coordinator remains responsive while git subprocesses execute. The pattern is consistent with the existing `GetHeadSha` and `IsAncestor` handlers.

## Success Criteria

### Must Have

- [ ] `handle_complete_phase` git calls (`get_status`, `stage_paths`, `commit`) run inside `spawn_blocking`
- [ ] `handle_batch_commit` git calls (`get_status`, `commit`) run inside `spawn_blocking`
- [ ] All existing tests pass (tests interact with the coordinator via its async API, so no test changes should be needed unless handler signatures change)
- [ ] Error handling uses the same `unwrap_or_else(|e| Err(format!("spawn_blocking panicked: {}", e)))` pattern as `GetHeadSha`/`IsAncestor` handlers

### Should Have

- [ ] No new `unwrap()` calls — `spawn_blocking` join errors are handled with descriptive error messages (matching existing pattern: `"spawn_blocking panicked: {}"`)

### Nice to Have

- [ ] Consolidated helper for the `spawn_blocking` + error mapping pattern to reduce boilerplate

## Scope

### In Scope

- Wrapping blocking git calls in `handle_complete_phase` in `spawn_blocking`
- Wrapping blocking git calls in `handle_batch_commit` in `spawn_blocking`
- Restructuring `handle_complete_phase` and `handle_batch_commit` so their git calls run inside `spawn_blocking` closures, while state mutations (`pending_batch_phases.push`, `pending_batch_phases.clear`) remain on the async executor thread. Git I/O must be moved into `spawn_blocking` at the call site in `run_coordinator`, matching the existing `GetHeadSha`/`IsAncestor` pattern. State mutations occur only after the `.await` on `spawn_blocking` returns `Ok`.
- Updating `run_coordinator` match arms to `.await` the `spawn_blocking` results before performing state mutations

### Out of Scope

- Changing the git module itself to use async process spawning (e.g., `tokio::process::Command`)
- Wrapping non-git blocking operations (e.g., file I/O for backlog read/write)
- Refactoring the coordinator actor pattern
- Performance benchmarking

## Non-Functional Requirements

- **Performance:** No measurable regression — `spawn_blocking` moves work to a dedicated thread pool, adding only thread scheduling overhead which is dwarfed by the subprocess I/O time of git commands.

## Constraints

- Must use the existing `tokio::task::spawn_blocking` mechanism (already available and used in the codebase)
- `handle_complete_phase` mutates `state.pending_batch_phases` — state mutations must remain on the async executor thread, outside `spawn_blocking` closures. This is because `spawn_blocking` closures capture by move and run on a separate OS thread, so they cannot hold mutable references to coordinator state. Only the git operations (which work on cloned/owned data) should move into the closure.
- The `spawn_blocking` closures need owned data (cloned `PathBuf`, `String`, etc.) since they move to another thread
- The coordinator processes commands sequentially (single message at a time in the actor loop), so there are no race conditions between `spawn_blocking` completion and state mutation — the `.await` on `spawn_blocking` suspends the actor loop until the blocking task completes

## Dependencies

- **Depends On:** Nothing — this is a self-contained refactor within `coordinator.rs`
- **Blocks:** Nothing directly, though it improves runtime correctness for all downstream orchestrator usage

## Risks

- [ ] Low: `spawn_blocking` closures require owned data, so cloning paths and strings adds minor allocations. Mitigated by the fact that these are short-lived, small allocations before subprocess I/O.
- [ ] Low: Changing synchronous functions to async could require test adjustments. Mitigated by existing test patterns for async coordinator operations.

## Decisions

- **Group git calls into a single `spawn_blocking` closure per handler.** The git calls within `handle_complete_phase` (`get_status` → `stage_paths` → `get_status` → `commit`) and `handle_batch_commit` (`get_status` → `commit`) are each grouped into a single `spawn_blocking` closure rather than split across multiple closures. This keeps the change straightforward, preserves the existing call sequence, and avoids interleaving async state checks between git operations.

## Open Questions

_(None remaining — all questions resolved during autonomous PRD creation.)_

## Assumptions

- **No human available:** Running autonomously as part of the orchestrated changes workflow.
- **State mutation strategy:** Mutable state updates (`pending_batch_phases.push`, `pending_batch_phases.clear`) will remain outside `spawn_blocking` closures, with only the git operations moved into the blocking context.
- **Existing failure/recovery behavior unchanged:** This change does not alter what happens when git operations fail (e.g., staging failures, commit failures). The existing error propagation via `Result<(), String>` is preserved. If git operations fail inside the `spawn_blocking` closure, the error is returned and state mutations (e.g., `pending_batch_phases.push/clear`) are skipped. Concerns like rollback on partial staging, git subprocess timeouts, and commit failure recovery are pre-existing behaviors that are out of scope for this change.
- **Thread pool capacity sufficient:** The coordinator processes commands sequentially, so at most one `spawn_blocking` git operation runs at a time. Tokio's default blocking thread pool (512 threads) is more than sufficient.

## References

- `coordinator.rs` — `handle_complete_phase` and `handle_batch_commit` functions (the functions to change)
- `coordinator.rs` — `GetHeadSha` and `IsAncestor` match arms in `run_coordinator` (the pattern to follow)
- `git.rs` — `get_status`, `stage_paths`, `commit`, `get_head_sha`, `is_ancestor` — all use `std::process::Command` (synchronous)
