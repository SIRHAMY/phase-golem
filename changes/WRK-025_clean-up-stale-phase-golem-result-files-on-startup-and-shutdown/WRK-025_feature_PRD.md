# Change: Clean Up Stale .phase-golem Result Files on Startup and Shutdown

**Status:** Proposed
**Created:** 2026-02-20
**Author:** Autonomous Agent

## Problem Statement

Phase execution (a unit of orchestrator work, e.g., `prd`, `build`, `triage`) writes ephemeral result JSON files to `.phase-golem/` (e.g., `phase_result_WRK-001_build.json`). These files exist only to pass structured results from a spawned subprocess agent back to the parent orchestrator process within a single run. Currently, result files are cleaned up per-execution: once before spawning an agent (`agent.rs:188-202`) and once after reading the result (`agent.rs:352-360`).

However, if the orchestrator crashes, is killed (SIGKILL), or exits abnormally between an agent writing its result and the executor reading it, the result file persists on disk indefinitely. Over many runs, these stale result files — files created in previous orchestrator sessions that remain after abnormal termination — accumulate.

While disk space waste is minor, the primary risk is that a stale result file could be mistakenly read as a current result if the same work item ID (e.g., WRK-001) and phase are re-executed. The per-execution cleanup in `agent.rs` mitigates this for normal runs, but startup cleanup provides an additional safety layer.

## User Stories / Personas

- **Orchestrator operator** — Runs phase-golem repeatedly across development sessions. Expects the `.phase-golem/` directory to stay clean without manual intervention. Wants confidence that stale results from crashed runs cannot interfere with new executions.

## Desired Outcome

When the orchestrator starts, it deletes all `phase_result_*.json` files from the `.phase-golem/` directory before any agents are spawned. This guarantees a clean slate at the beginning of each run. On graceful shutdown (normal exit via SIGTERM, SIGINT, or completion), it performs the same cleanup to leave the directory tidy.

After this change, the `.phase-golem/` directory should only contain result files that belong to the currently-running orchestrator session (plus the lock and PID files which are already managed separately). The existing per-execution cleanup in `agent.rs` remains as a defense-in-depth fallback for individual files.

## Success Criteria

### Must Have

- [ ] On startup, all `phase_result_*.json` files in `.phase-golem/` are deleted before any agents are spawned
- [ ] Cleanup runs after the lock is acquired (ensuring exclusive access to `.phase-golem/`) but before the coordinator/scheduler starts processing work items
- [ ] Cleanup attempts to delete all matching files regardless of individual failures — each failure is logged as a warning, and startup continues even if all deletions fail
- [ ] If the `.phase-golem/` directory does not exist or cannot be read (e.g., permission error), cleanup logs a warning and is a no-op (does not abort startup)

### Should Have

- [ ] On graceful shutdown, all `phase_result_*.json` files are deleted while the lock is still held, after all agents have completed and the coordinator has shut down, but before the lock is released
- [ ] Cleanup logs at info level when stale files are found: "Cleaned up N stale result files from .phase-golem/". No log emitted when zero files are found.

### Nice to Have

- [ ] None identified — this is intentionally minimal

## Scope

### In Scope

- Startup cleanup of `phase_result_*.json` files in the `.phase-golem/` directory
- Graceful shutdown cleanup of the same files
- Warning-level logging for cleanup failures on individual files
- Informational logging when stale files are found and removed

### Out of Scope

- Cleanup of lock files (`phase-golem.lock`, `phase-golem.pid`) — these are already managed by `lock.rs`
- Cleanup of any files outside `.phase-golem/`
- Changing the existing per-execution cleanup in `agent.rs`
- Recursive directory cleanup or removing subdirectories
- Cleanup on ungraceful shutdown (SIGKILL) — this is exactly the scenario startup cleanup handles on the next run
- Special handling for symlinks, device files, or non-regular files — standard `remove_file` behavior applies

## Non-Functional Requirements

- **Performance:** Cleanup should complete in under 100ms for typical file counts (< 100 files). It runs once at startup and once at shutdown, so performance is not critical.

## Constraints

- Must run after lock acquisition to avoid racing with another instance that may be actively using result files
- Must run before any agents are spawned to avoid deleting results from the current run
- Must not fail startup — all errors during cleanup (both directory-level and file-level) should be logged and swallowed

## Dependencies

- **Depends On:** Nothing — this is a standalone improvement
- **Blocks:** Nothing

## Risks

- [ ] Deleting a result file that belongs to the current run: Mitigated by running cleanup before any agents are spawned (startup) and after all agents have completed (shutdown)
- [ ] Race condition with concurrent orchestrator: Mitigated by running cleanup after lock acquisition — only one orchestrator can hold the lock at a time

## Open Questions

None — this change is straightforward with well-understood behavior.

## Assumptions

- The `phase_result_*.json` glob pattern is sufficient to match all result files without matching other files in `.phase-golem/`. Confirmed by examining `executor.rs:505-507` which uses exactly this naming convention.
- Running cleanup after lock acquisition is safe because no other orchestrator can be running while we hold the lock.
- It is acceptable to delete result files from a crashed run without inspecting their contents. These files are ephemeral by design and have no archival value.
- Item IDs and phase names come from trusted internal configuration (backlog YAML), so the filename pattern does not need to defend against injection or adversarial input.
- Phase result files are the only ephemeral files that accumulate across runs. Lock and PID files are managed separately by `lock.rs`.

## References

- Result file path construction: `src/executor.rs:505-507`
- Per-execution cleanup: `src/agent.rs:188-202` (pre-spawn) and `src/agent.rs:352-360` (post-read)
- Lock acquisition: `src/main.rs:345-346`
- Startup flow: `src/main.rs:328-761`
- Shutdown flow: `src/main.rs:650-721`
