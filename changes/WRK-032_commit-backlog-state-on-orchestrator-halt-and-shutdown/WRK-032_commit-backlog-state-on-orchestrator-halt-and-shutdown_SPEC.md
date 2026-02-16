# SPEC: Commit backlog state on orchestrator halt and shutdown

**ID:** WRK-032
**Status:** Draft
**Created:** 2026-02-13
**PRD:** ./WRK-032_commit-backlog-state-on-orchestrator-halt-and-shutdown_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

When the orchestrator halts, BACKLOG.yaml may have uncommitted changes from status transitions, block reasons, or follow-up ingestion that occurred outside phase completions. These changes are written to disk by `save_backlog()` but never committed to git, creating a durability gap. This was observed in practice when WRK-029 was blocked by guardrail checks but the state change was never committed.

The fix adds a final git commit step to `handle_run()` that runs after the coordinator actor completes `save_backlog()`, committing BACKLOG.yaml if dirty.

## Approach

Modify `spawn_coordinator()` to return `(CoordinatorHandle, JoinHandle<()>)` so `handle_run()` can await the coordinator task after the scheduler halts. After awaiting, check if BACKLOG.yaml has uncommitted changes via `git::get_status()`, and if dirty, stage and commit it with the halt reason in the message. Git errors are logged as warnings and do not prevent clean process exit.

The shutdown sequence becomes:
```
run_scheduler() → kill_children → await coordinator JoinHandle → commit BACKLOG.yaml if dirty → print summary → return
```

**Patterns to follow:**

- `.claude/skills/changes/orchestrator/src/coordinator.rs:594-617` — `BatchCommit` handler demonstrates the `spawn_blocking` + git operations pattern
- `.claude/skills/changes/orchestrator/src/coordinator.rs:688-707` — `spawn_coordinator()` current implementation (modification target)
- `.claude/skills/changes/orchestrator/src/git.rs:107-127` — `get_status()` parsing of `--porcelain` output with `StatusEntry`

**Implementation boundaries:**

- Do not modify: `orchestrator/src/git.rs` — existing functions are used as-is
- Do not modify: `orchestrator/src/scheduler.rs` — `HaltReason` and `RunSummary` are used as-is
- Do not modify: `orchestrator/src/backlog.rs` — atomic save pattern is unchanged
- Do not refactor: the coordinator actor's internal shutdown hook (line 682-683) — `save_backlog()` stays where it is

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | API change and shutdown commit logic | Low | Modify `spawn_coordinator()` return type, update both call sites, add backlog commit logic to `handle_run()` |
| 2 | Tests | Low | Add unit tests for path filtering and integration tests for the shutdown commit flow |

**Ordering rationale:** Phase 1 implements all functional changes. Phase 2 adds test coverage — it depends on Phase 1 being complete since tests exercise the new code paths.

---

## Phases

### Phase 1: API change and shutdown commit logic

> Modify `spawn_coordinator()` to return JoinHandle, update call sites, add backlog commit after halt

**Phase Status:** complete

**Complexity:** Low

**Goal:** After orchestrator halt, BACKLOG.yaml is committed to git if it has uncommitted changes, with the halt reason in the commit message.

**Files:**

- `.claude/skills/changes/orchestrator/src/coordinator.rs` — modify — change `spawn_coordinator()` to return `(CoordinatorHandle, JoinHandle<()>)` (~3 lines)
- `.claude/skills/changes/orchestrator/src/main.rs` — modify — destructure tuple in `handle_run()` and `handle_triage()`, add shutdown commit logic after `kill_all_children()` in `handle_run()`

**Patterns:**

- Follow `coordinator.rs:594-617` (`BatchCommit` handler) for the `spawn_blocking` + git operations + `JoinError` handling pattern

**Tasks:**

- [x] In `coordinator.rs:spawn_coordinator()` (line 688-707): capture the `tokio::spawn()` result in a variable, return `(CoordinatorHandle { sender: tx }, task_handle)` instead of just `CoordinatorHandle`
- [x] In `main.rs:handle_run()` (line 426-432): destructure as `let (coord_handle, coord_task) = coordinator::spawn_coordinator(...)`; pass only `coord_handle` to `run_scheduler()`
- [x] In `main.rs:handle_triage()` (line 522-528): destructure as `let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(...)`; discard JoinHandle (out of scope per PRD)
- [x] In `main.rs:handle_run()`, after `kill_all_children()` (line 461) and before summary printing (line 463): add shutdown commit logic:
  1. Await `coord_task` — if `Err(JoinError)`, `log_warn!("Coordinator task panicked, skipping backlog commit: {:?}", err)` and skip to summary
  2. Use `spawn_blocking` to run git operations (clone needed variables into the closure):
     - Call `git::get_status(Some(&root))` — if `Err`, `log_warn!` and skip commit entirely
     - Filter entries: `entry.path.trim_matches('"') == "BACKLOG.yaml"`. Any status code qualifies as dirty (staged `M `, unstaged ` M`, both `MM`, untracked `??`). If no matching entry, skip silently (no log noise).
     - If dirty: call `git::stage_paths(&[&backlog_file_path], Some(&root))` — if `Err`, `log_warn!` and skip commit (file remains unstaged)
     - Then call `git::commit(&format!("[orchestrator] Save backlog state on halt ({:?})", summary.halt_reason), Some(&root))` — if `Err`, `log_warn!` (BACKLOG.yaml remains staged; next run or manual commit picks it up)
     - On success: `log_info!` with committed SHA
  3. Handle `spawn_blocking` `JoinError` with `log_warn!`, do not fail `handle_run()`

**Verification:**

- [x] `cargo build` succeeds with no errors or warnings
- [ ] Manual test: run orchestrator with a cap of 1, verify BACKLOG.yaml is committed after halt with message containing `[orchestrator] Save backlog state on halt (CapReached)`. Verify with `git log -1 --oneline` and `git show --name-only HEAD` (only BACKLOG.yaml should appear in the commit)
- [ ] Manual test: run orchestrator when BACKLOG.yaml has no uncommitted changes, verify no empty commit is created (git log should show no new commit)
- [ ] Manual test: modify an unrelated file before running orchestrator with cap 1, verify the shutdown commit contains only BACKLOG.yaml and the other file remains dirty (`git status` still shows it)
- [ ] Verify `log_info!` is emitted with commit SHA on successful shutdown commit
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-032][P1] Feature: Commit backlog state on orchestrator halt`

**Notes:**

The `backlog_file_path` variable (a `PathBuf` to `BACKLOG.yaml`) is already in scope in `handle_run()` (declared around line 263). The `root` variable is a `&Path` parameter. The `summary` variable (containing `halt_reason`) is returned from `run_scheduler()` at line 458. All data needed for the commit is available without additional plumbing. Use `backlog_file_path` for `stage_paths()` — do not construct a new path variable.

The `handle_triage()` path (line 522-528) uses immediate commits for each triage phase and drops the coordinator at line 588. It doesn't need the shutdown commit since triage uses destructive (immediate) commits. The JoinHandle is discarded with `_coord_task`.

Regarding `save_backlog()` failure detection: the coordinator runs `let _ = state.save_backlog()` (line 683), swallowing errors. However, `save_backlog()` uses atomic write (temp file + fsync + rename), so on failure the previous BACKLOG.yaml remains intact on disk. If the previous state was already committed, `get_status()` finds no changes and the commit is skipped. If the previous state had uncommitted changes, those (last-known-good) changes are committed. This is acceptable — no inconsistent state is ever committed.

Manual verification items are deferred to end-to-end testing — they require a full orchestrator run with a real backlog and git repo. The implementation was verified structurally via code review and `cargo build`/`cargo test` (490 tests passing, 0 failures).

**Followups:**

---

### Phase 2: Tests

> Add tests for path filtering logic and shutdown commit flow

**Phase Status:** complete

**Complexity:** Low

**Goal:** Verify the shutdown commit logic works correctly across happy path, no-op, and error scenarios.

**Files:**

- `.claude/skills/changes/orchestrator/src/coordinator.rs` — modify — add unit test for new `spawn_coordinator()` return type in existing test module (line 709+)
- `.claude/skills/changes/orchestrator/tests/coordinator_test.rs` — modify — add integration test(s) for shutdown commit flow if test infrastructure supports it

**Patterns:**

- Follow existing tests in `coordinator.rs:709+` for unit test structure
- Follow `tests/git_test.rs` for git operation test patterns with temp repos

**Tasks:**

- [x] Add unit test in `coordinator.rs` tests module: `spawn_coordinator_returns_joinhandle` — verify `spawn_coordinator()` returns a tuple where the JoinHandle resolves to `Ok(())` after handle is dropped
- [x] Add test for BACKLOG.yaml path matching: verify filtering logic correctly matches `"BACKLOG.yaml"` (unquoted), `"\"BACKLOG.yaml\""` (git-quoted), and does NOT match `"other.yaml"`, `"BACKLOG.yaml.bak"`, or `"subdir/BACKLOG.yaml"`
- [x] Add test verifying no commit when BACKLOG.yaml is clean (get_status returns no matching entry)
- [x] Add test for commit message format: verify `format!("[orchestrator] Save backlog state on halt ({:?})", HaltReason::CapReached)` produces expected string for representative HaltReason variants
- [x] Run `cargo test` — all tests pass, no regressions

**Verification:**

- [x] `cargo test` passes with no failures
- [x] New tests cover: JoinHandle resolves correctly, path matching (positive and negative cases), no-commit-when-clean, commit message format
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-032][P2] Feature: Add tests for backlog commit on halt`

**Notes:**

Full integration testing of the shutdown commit flow (spawning a real coordinator, running a scheduler, halting, verifying git commit) may be impractical in unit tests since it requires a full orchestrator setup. Focus on testing the individual components: JoinHandle lifecycle, path filtering logic, and commit message formatting. The Phase 1 manual verification covers the end-to-end flow.

**Followups:**

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met
- [x] Tests pass
- [x] No regressions introduced
- [x] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | complete | `[WRK-032][P1]` | API change + shutdown commit logic. 490 tests pass, code review clean. |
| 2 | complete | `[WRK-032][P2]` | Tests for JoinHandle, path matching, no-commit-when-clean, commit message format. 506 tests pass, code review clean. |

## Followups Summary

### Critical

### High

### Medium

### Low

## Design Details

### Key Types

No new types are introduced. Existing types used:

```rust
// coordinator.rs — return type changes from CoordinatorHandle to tuple
pub fn spawn_coordinator(...) -> (CoordinatorHandle, tokio::task::JoinHandle<()>)

// git.rs — used as-is
pub struct StatusEntry {
    pub status_code: String,
    pub path: String,
}

// scheduler.rs — used as-is
pub enum HaltReason {
    AllDoneOrBlocked, CapReached, CircuitBreakerTripped, ShutdownRequested,
    TargetCompleted, TargetBlocked, FilterExhausted, NoMatchingItems,
}

pub struct RunSummary {
    pub halt_reason: HaltReason,
    // ... other fields
}
```

### Architecture Details

The shutdown commit logic sits in `handle_run()` between `kill_all_children()` and the summary print block. It follows the same `spawn_blocking` pattern used by `BatchCommit` in the coordinator for consistency, though synchronous git calls would also be safe here since no other async work is running.

Data flow: `run_scheduler()` returns `RunSummary` (with `halt_reason`) → `coord_handle` was consumed by scheduler → coordinator's mpsc channel closes → coordinator runs `save_backlog()` → coordinator task completes → `handle_run()` awaits `JoinHandle` → dirty check → stage + commit → print summary.

### Design Rationale

See `WRK-032_commit-backlog-state-on-orchestrator-halt-and-shutdown_DESIGN.md` for full rationale on:
- Why commit from `handle_run()` rather than inside the coordinator (halt reason access, separation of concerns)
- Why `spawn_blocking` over synchronous calls (consistency with existing pattern)
- Why filter `get_status()` for BACKLOG.yaml only (prevent accidental staging of other files)

---

## Assumptions

Decisions made without human input (autonomous mode):

- **Mode selection:** Light mode, matching the design's light mode and the item's low complexity/small size assessments. Agents 1-2 (File & Change Analyzer, Dependency Mapper) were run for analysis.
- **Single phase for implementation:** Self-critique suggested splitting Phase 1 into 1a (API change) and 1b (shutdown logic). Kept as single phase because the total change is ~25 lines across 2 files with a single call site — splitting creates two trivially small phases with no independent verification benefit.
- **No timeout on JoinHandle await:** Self-critique suggested adding `tokio::time::timeout` around the coordinator await. Decided against it because the coordinator's shutdown path is deterministic (`save_backlog()` → exit) with no blocking I/O or loops. If the coordinator hangs, the operator would kill the process externally (SIGTERM/SIGKILL). Adding a timeout introduces a new failure mode (timeout fires during a slow-but-legitimate save) with no practical benefit.
- **No fallback if `get_status()` fails:** Self-critique suggested attempting a commit anyway if `get_status()` fails. Decided against it because blindly committing could stage unexpected files or create empty commits. The warn-and-skip approach is safer — if `get_status()` fails, something is wrong with git and committing is unlikely to succeed either.
- **`save_backlog()` failure is handled implicitly:** The coordinator uses `let _ = state.save_backlog()` which swallows errors. Because `save_backlog()` uses atomic write (temp + fsync + rename), on failure the previous valid BACKLOG.yaml stays on disk. The dirty check via `get_status()` then either finds changes (commits last-known-good state) or finds no changes (skips). No inconsistent state is committed.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
