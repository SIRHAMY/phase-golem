# Change: Replace blocking std::fs in resolve_or_find_change_folder with tokio::fs

**Status:** Proposed
**Created:** 2026-02-13
**Author:** Orchestrator (autonomous)

## Problem Statement

The `resolve_or_find_change_folder` function in `executor.rs` uses blocking `std::fs` calls (`read_dir`, `create_dir_all`, `DirEntry::file_type`) inside an async context. It is called from `execute_phase`, which is an `async fn` running on the tokio runtime. Blocking filesystem I/O on the tokio executor thread can stall the runtime, preventing other tasks from making progress. While the practical impact is low for this particular function (called once per phase, directory reads are fast), it violates async best practices and creates a pattern that could cause real issues if copied to higher-frequency code paths.

## User Stories / Personas

- **Orchestrator maintainer** - Wants the codebase to consistently use async I/O in async contexts so that blocking calls don't silently accumulate and eventually cause runtime stalls.

## Desired Outcome

`resolve_or_find_change_folder` should use `tokio::fs` for all filesystem operations, making it an `async fn` that can be `.await`ed at its single call site in `execute_phase`. The behavior and error handling should remain identical.

## Success Criteria

### Must Have

- [ ] `resolve_or_find_change_folder` is an `async fn`
- [ ] `std::fs::read_dir` replaced with `tokio::fs::read_dir` using `next_entry().await` iteration
- [ ] `std::fs::create_dir_all` replaced with `tokio::fs::create_dir_all().await`
- [ ] `DirEntry::file_type()` uses tokio's async `file_type().await`
- [ ] Call site in `execute_phase` (line 311) updated to `.await` the result
- [ ] All existing tests pass (test call sites may need `.await` additions for the new async signature)
- [ ] `cargo clippy` passes

### Should Have

- [ ] No new dependencies required (tokio "full" features already include `tokio::fs`)

## Scope

### In Scope

- Converting `resolve_or_find_change_folder` from sync to async
- Updating the single call site in `execute_phase`
- Replacing `Path::exists()` check (line 476) with async `read_dir` error handling (`NotFound` instead of pre-checking, eliminates TOCTOU race)

### Out of Scope

- Other blocking `std::fs` calls in other files (e.g., `main.rs`, `config.rs`, `backlog.rs`, `worklog.rs` — these run in synchronous startup/shutdown contexts)
- WRK-019 (targets `run_subprocess_agent`, a different function)
- Adding new unit tests for `resolve_or_find_change_folder` (it's private and tested indirectly through `execute_phase` tests)
- Refactoring the function's logic or error handling beyond what's needed for the async conversion

## Non-Functional Requirements

- **Performance:** No measurable change expected. Async directory reads have slightly more overhead than blocking reads, but the function is called once per phase execution, not in a hot loop.

## Constraints

- Must not add new crate dependencies. `tokio::fs::read_dir` returns a `ReadDir` with `next_entry().await` — no `futures::StreamExt` needed.
- Function is private (`fn`, not `pub fn`), so API surface impact is internal only.

## Dependencies

- **Depends On:** Nothing — tokio "full" features already enabled in Cargo.toml.
- **Blocks:** Nothing.

## Risks

- [ ] Low: Iteration pattern change (`for` loop to `while let` with `next_entry().await`) changes iteration mechanics — mitigated by the fact that directory entry order is already non-deterministic and any matching directory is acceptable (not order-dependent), confirmed via existing test execution.

## Open Questions

None — scope is well-defined and implementation path is clear.

## Assumptions

- The `Path::exists()` check on line 476 can be folded into the `tokio::fs::read_dir` call by handling `ErrorKind::NotFound` — this avoids a separate blocking `exists()` check and removes a TOCTOU race. Decision: proceed with this approach since it's a strict improvement.
- No `futures` crate dependency is needed because `tokio::fs::ReadDir::next_entry()` is a native async method, not a `Stream` trait implementation.

## References

- Target function: `executor.rs:468-498`
- Call site: `executor.rs:311`
- Existing async fs patterns in codebase: `agent.rs:178`, `agent.rs:323`, `agent.rs:339`
- Related but separate item: WRK-019 (different function scope)
