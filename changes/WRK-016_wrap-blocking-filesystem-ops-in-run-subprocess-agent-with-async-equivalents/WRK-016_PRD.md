# Change: Wrap blocking filesystem ops in run_subprocess_agent with async equivalents

**Status:** Proposed
**Created:** 2026-02-12
**Author:** Autonomous Agent (originated from WRK-001/build)

## Problem Statement

The `run_subprocess_agent` function in `agent.rs` is an `async fn` running on the tokio async runtime, but it makes 3 blocking `std::fs` calls that execute directly on async worker threads. These calls — `fs::remove_file` (stale result cleanup), `fs::read_to_string` (result parsing), and `fs::remove_file` (post-read cleanup) — can stall the tokio thread pool if the filesystem is slow (e.g., NFS/Network File System, overloaded disk, or large result files). While this is unlikely to cause measurable performance issues under typical local-disk conditions, it violates async best practices: blocking calls on async worker threads can starve other tasks of execution time and reduce concurrency.

This blocking I/O problem was identified during the WRK-001 build phase, which already fixed the analogous issue in `kill_process_group` by wrapping its blocking loop in `spawn_blocking`. The 3 filesystem calls in `run_subprocess_agent` and its helpers (`read_result_file`, `cleanup_result_file`) are the remaining blocking operations in the async agent execution path (code that runs on tokio worker threads during runtime operations, as opposed to startup-only code).

## User Stories / Personas

- **Orchestrator operator** — Runs the orchestrator to manage multi-phase AI agent workflows. Expects the async runtime to remain responsive even when filesystem I/O is slow, so concurrent agents aren't starved of worker threads.

## Desired Outcome

After this change, all filesystem operations in `run_subprocess_agent` and its helper functions (`read_result_file`, `cleanup_result_file`) use async-aware I/O instead of blocking `std::fs` calls. No filesystem operations in this code path execute directly on tokio worker threads. All observable behavior is preserved: result file cleanup, JSON parsing, error messages (including NotFound suppression for stale files), and warning logs remain functionally identical.

## Success Criteria

### Must Have

- [ ] `fs::remove_file` at line 179 (stale result deletion) is replaced with `tokio::fs::remove_file(...).await`
- [ ] `fs::read_to_string` in `read_result_file` is replaced with `tokio::fs::read_to_string(...).await`
- [ ] `fs::remove_file` in `cleanup_result_file` is replaced with `tokio::fs::remove_file(...).await`
- [ ] `read_result_file` and `cleanup_result_file` become `async fn`
- [ ] All existing error handling behavior is preserved: NotFound errors on stale result cleanup are silently ignored, read errors distinguish NotFound from other I/O errors, cleanup failure logs a warning but does not propagate
- [ ] All callers of `read_result_file` are updated to `.await` — this includes `run_subprocess_agent` in `agent.rs` and 4 test functions in `tests/agent_test.rs` (`read_result_file_valid_json`, `read_result_file_missing_file`, `read_result_file_invalid_json`, `read_result_file_missing_required_fields`)
- [ ] Test functions calling `read_result_file` are converted from `#[test]` to `#[tokio::test]` async functions
- [ ] Existing tests continue to pass

### Should Have

- [ ] No new dependencies added (tokio "full" already provides `tokio::fs`)

### Nice to Have

- [ ] Remove `use std::fs;` import if no longer needed in the module after the change

## Scope

### In Scope

- The 3 blocking `std::fs` calls within `run_subprocess_agent`, `read_result_file`, and `cleanup_result_file` in `agent.rs`
- Updating function signatures to `async fn` where needed
- Updating call sites within `agent.rs` to `.await` the new async functions
- Updating test functions in `tests/agent_test.rs` that call `read_result_file` to be async

### Out of Scope

- Blocking filesystem ops in other modules (`executor.rs`, `backlog.rs`, `config.rs`, `main.rs`, `worklog.rs`, `lock.rs`, `git.rs`, `migration.rs`, `preflight.rs`) — these are tracked separately (e.g., WRK-006 for executor.rs)
- `CliAgentRunner::verify_cli_available()` — this uses blocking `std::process::Command` but runs at startup, not in the async agent execution path
- Other blocking calls in `agent.rs` outside the 3 target functions (e.g., `kill_all_children` which uses blocking `std::thread::sleep` but is only called during shutdown)
- Performance benchmarking — the improvement is async correctness (preventing worker thread blocking), not measurable perf gain under typical local-disk conditions
- Retry logic for transient filesystem errors (current fail-fast behavior is preserved)
- Changing the approach used by `kill_process_group` (already correctly using `spawn_blocking`)

## Non-Functional Requirements

- **Performance:** No regression. `tokio::fs` operations internally use `spawn_blocking`, so latency characteristics are equivalent or better than raw `std::fs` on the async runtime.

## Constraints

- Must use `tokio::fs` (preferred) or `tokio::task::spawn_blocking` — both are available via the existing `tokio` dependency with "full" features
- `read_result_file` is `pub` — its signature change from sync to async is a breaking internal API change; all callers within the codebase must be updated (verified: `agent.rs` and `tests/agent_test.rs` only)
- `cleanup_result_file` is private (`fn`, not `pub fn`) — only called from within `run_subprocess_agent`, so the signature change has no external impact
- `MockAgentRunner` does not call these filesystem helpers, so mock tests are unaffected
- Test fixture setup code (e.g., `fs::write` for creating test JSON files) may remain blocking — only the calls to `read_result_file` need async conversion

## Dependencies

- **Depends On:** None — `tokio::fs` is already available
- **Blocks:** Nothing directly, but contributes to the broader goal of eliminating blocking I/O from async code paths (WRK-016 scope is limited strictly to the 3 filesystem calls listed in Success Criteria)

## Risks

- [ ] Minimal risk — `read_result_file` callers have been verified via grep: called from `run_subprocess_agent` in `agent.rs` and 4 test functions in `tests/agent_test.rs`. Not called from `executor.rs` or any other module.

## Open Questions

None — this change is well-understood with a clear implementation path.

## Assumptions

- **`tokio::fs` over `spawn_blocking`:** Using `tokio::fs` directly is preferred over manual `spawn_blocking` wrappers because it produces less boilerplate and `tokio::fs` internally uses `spawn_blocking` anyway. The codebase uses `spawn_blocking` in `coordinator.rs` for git operations, but the git operations in `coordinator.rs` wrap multiple calls in a single blocking closure to reduce thread pool overhead. Here, each fs call is independent and interspersed with async logic, making `tokio::fs` the cleaner choice.
- **`read_result_file` callers (verified):** Grep confirms `read_result_file` is called from `agent.rs:260` (within `run_subprocess_agent`) and from 4 test functions in `tests/agent_test.rs`. It is NOT called from `executor.rs` or any other module.
- **Caller responsibility for path safety:** The caller is responsible for ensuring `result_path` is a safe, validated path. This is unchanged from current behavior.

## References

- WRK-001: Original work item that fixed `kill_process_group` blocking and identified these remaining fs ops
- WRK-006: Related item for blocking fs ops in `executor.rs`
- `agent.rs` lines 173-347: The affected code
- `coordinator.rs` lines 547+: Established `spawn_blocking` pattern for reference
