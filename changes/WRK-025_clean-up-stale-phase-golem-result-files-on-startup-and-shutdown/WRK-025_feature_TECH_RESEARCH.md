# Tech Research: Clean Up Stale .phase-golem Result Files on Startup and Shutdown

**ID:** WRK-025
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-025_feature_PRD.md
**Mode:** Light

## Overview

Researching how to implement startup/shutdown cleanup of stale `phase_result_*.json` files in `.phase-golem/`. The core questions are: what Rust patterns to use for directory scanning and file deletion, where exactly in the startup/shutdown flow to insert cleanup, and what error handling approach to follow.

## Research Questions

- [x] What Rust pattern should we use for directory scanning + pattern-matched deletion?
- [x] Where exactly in `handle_run()` should startup cleanup be inserted?
- [x] Where exactly in `handle_run()` should shutdown cleanup be inserted?
- [x] What error handling pattern should cleanup follow (consistent with existing code)?
- [x] Should cleanup use async (`tokio::fs`) or sync (`std::fs`) I/O?

---

## External Research

### Landscape Overview

This is a well-established Rust pattern: read a directory, filter entries by filename pattern, delete matching files with graceful error handling. Rust's standard library (`std::fs` / `tokio::fs`) provides all necessary tools. The only decision point is whether to use a `glob` crate vs hand-rolling with `read_dir` + string matching.

### Common Patterns & Approaches

#### Pattern: Hand-Rolled `read_dir` + String Matching

**How it works:** Use `tokio::fs::read_dir()` to iterate directory entries, check each filename with `starts_with("phase_result_")` and `ends_with(".json")`, and call `remove_file()` on matches.

**When to use:** Simple, fixed patterns like `phase_result_*.json` where the pattern is unlikely to change.

**Tradeoffs:**
- Pro: No external dependency, zero overhead, simple to understand
- Pro: ~15 lines of code, fully auditable
- Con: Manual string matching; less flexible if patterns become complex

#### Pattern: `glob` Crate

**How it works:** Use `glob::glob("path/phase_result_*.json")` to get matching paths, then delete each.

**When to use:** Complex or dynamic glob patterns, multiple patterns in different places.

**Tradeoffs:**
- Pro: Powerful pattern matching, standard glob syntax
- Con: Adds external dependency, overkill for a single fixed pattern

### Standards & Best Practices

1. **Best-effort cleanup:** Continue deleting remaining files even if one deletion fails. Standard pattern in cleanup code (systemd, Docker, package managers).
2. **Log failures but don't abort:** For non-critical operations, log at warning level and continue.
3. **Single summary log:** Emit one info-level line with count of removed files, not per-file logs.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Aborting startup on cleanup failure | Cleanup is non-critical; blocking startup is worse than stale files | Swallow all errors, log warnings |
| Not handling `read_dir` failure gracefully | Directory might not exist or have permission issues | Match on error, log warning, return early |
| Per-file info logging | Noisy output for large cleanups | Log one summary line with total count |

### Key Learnings

- Hand-rolled `read_dir` + string matching is the right approach for this use case — no external dependency needed
- The `tokio::fs::read_dir` async iterator pattern is already established in the codebase (`executor.rs:522-543`)

---

## Internal Research

### Existing Codebase State

The codebase already has per-execution cleanup of result files in `agent.rs`. This change adds a complementary startup/shutdown cleanup as defense-in-depth.

**Relevant files/modules:**

- `src/executor.rs:505-507` — `result_file_path()` function constructs paths as `.phase-golem/phase_result_{item_id}_{phase}.json`
- `src/agent.rs:188-202` — Pre-spawn cleanup: deletes stale result file before spawning agent. Uses `tokio::fs::remove_file()`, handles `NotFound` as expected, returns `Err` on other failures
- `src/agent.rs:351-360` — Post-read cleanup: `cleanup_result_file()` deletes result file after successful read. Logs warning on any error, does not propagate (swallows errors)
- `src/main.rs:328-761` — `handle_run()` async function. Lock acquired at line 346, scheduler runs around line 400+, shutdown flow at lines 650-721, function returns at line 760
- `src/lock.rs:44-94` — `try_acquire()` creates `.phase-golem/` directory with `create_dir_all()`, then acquires file lock. Directory is guaranteed to exist after this returns
- `src/executor.rs:522-543` — `resolve_or_find_change_folder()` shows the async `read_dir` iteration pattern used in this codebase
- `src/log.rs` — Custom logging macros: `log_info!`, `log_warn!`, `log_debug!`, `log_error!`

**Existing patterns in use:**

- `tokio::fs` for async file operations throughout agent/executor code
- `match` on `io::ErrorKind::NotFound` to treat missing files as expected (not an error)
- `log_warn!("Warning: ...")` for non-critical I/O failures
- `log_info!("[tag] message")` with tags like `[pre]`, `[post]` for lifecycle events

### Reusable Components

- **`tokio::fs::read_dir` + async iterator pattern** — Already used in `executor.rs:522-543` for iterating directory entries with `next_entry().await`
- **`runtime_dir` variable** — Already available in `handle_run()` at line 345 (`root.join(".phase-golem")`)
- **Logging macros** — `log_info!` and `log_warn!` already imported in `main.rs`
- **Error handling pattern** — `match` on `tokio::fs::remove_file()` result with `ErrorKind::NotFound` handling, established in `agent.rs`

### Constraints from Existing Code

- **Must use `tokio::fs`** for consistency — `handle_run()` is async, and all file I/O in agent/executor code uses tokio
- **Lock ordering** — Cleanup must run after `lock::try_acquire()` (line 346) and before lock release (function return)
- **No new dependencies needed** — `tokio` with `fs` feature is already in `Cargo.toml`

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| None identified | PRD is accurate about all code locations, naming patterns, and error handling | No design adjustments needed |

The PRD is well-researched and accurately reflects the codebase. All referenced line numbers, file paths, naming conventions, and architectural constraints are confirmed.

---

## Critical Areas

### Startup Insertion Point Ordering

**Why it's critical:** Cleanup must run after lock acquisition but before any agents are spawned. Getting this wrong could delete results from the current run.

**Why it's easy to miss:** The startup flow has several steps between lock acquisition (line 346) and scheduler creation (~line 400+). The cleanup could be inserted at several valid points.

**What to watch for:** Insert immediately after lock acquisition (line 346), before git preconditions check (line 347). This is the earliest safe point and maximizes the guarantee that no agents can have written results yet.

---

## Deep Dives

_No deep dives needed — the problem space is well-understood._

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| None | — | — |

No open questions. The implementation path is clear.

### Recommended Approaches

#### Directory Scanning + Pattern Matching

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Hand-rolled `tokio::fs::read_dir` + string matching | No dependency, ~15 lines, consistent with existing code | Less flexible for complex patterns | Simple fixed pattern (our case) |
| `glob` crate | Powerful pattern matching | Unnecessary dependency | Complex/dynamic patterns |

**Initial recommendation:** Hand-rolled `tokio::fs::read_dir` + `starts_with("phase_result_")` && `ends_with(".json")`. Matches the existing pattern in `executor.rs:522-543` and adds no dependencies.

#### Implementation Location

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| New function in `main.rs` | Collocated with call sites, simple | Grows `main.rs` slightly | Small, focused function (our case) |
| New `cleanup.rs` module | Clean separation | Overkill for ~20 lines | Multiple cleanup concerns |

**Initial recommendation:** Add an `async fn cleanup_stale_result_files(runtime_dir: &Path)` directly in `main.rs`. It's ~20 lines and only called from `handle_run()`, so a separate module isn't warranted.

#### Error Handling

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Swallow all errors, log warnings | Never blocks startup/shutdown | Could hide real problems | Non-critical cleanup (our case) |
| Return `Result`, let caller decide | Flexible | Caller must remember to swallow | Critical operations |

**Initial recommendation:** Swallow all errors internally. Log `log_warn!` for individual file deletion failures and `read_dir` failures. Log `log_info!` summary when stale files are found. Consistent with `cleanup_result_file()` in `agent.rs:351-360`.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| `src/executor.rs:522-543` | Code | Shows async `read_dir` iteration pattern |
| `src/agent.rs:351-360` | Code | Shows error-swallowing cleanup pattern |
| `src/agent.rs:188-202` | Code | Shows `NotFound` error handling pattern |
| `src/main.rs:345-346` | Code | Startup insertion point (after lock acquisition) |
| `src/main.rs:721` | Code | Shutdown insertion point (after backlog commit) |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Light internal + external research | Confirmed PRD accuracy, identified insertion points, recommended hand-rolled read_dir approach |

## Assumptions

- Light research mode is appropriate given the simplicity of this change (glob + delete pattern with well-understood Rust stdlib APIs)
- No user Q&A conducted (autonomous mode) — decisions documented inline
