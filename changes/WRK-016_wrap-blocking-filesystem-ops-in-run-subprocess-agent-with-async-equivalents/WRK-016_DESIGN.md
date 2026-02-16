# Design: Wrap blocking filesystem ops in run_subprocess_agent with async equivalents

**ID:** WRK-016
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-016_PRD.md
**Tech Research:** ./WRK-016_TECH_RESEARCH.md
**Mode:** Light

## Overview

Replace 3 blocking `std::fs` calls in `agent.rs` with their `tokio::fs` async equivalents. The change is mechanical: swap `fs::remove_file` / `fs::read_to_string` with `tokio::fs::remove_file` / `tokio::fs::read_to_string`, add `.await`, convert `read_result_file` and `cleanup_result_file` to `async fn`, and update all callers including 4 test functions in `agent_test.rs`. This is the simplest correct approach because the 3 filesystem calls are independent and interspersed with async logic, making `tokio::fs` cleaner than manual `spawn_blocking`.

---

## System Design

### High-Level Architecture

No new components or architectural changes. This is a mechanical conversion of 3 function calls within a single module (`agent.rs`) from synchronous to asynchronous, plus updating callers in the test file (`agent_test.rs`).

The affected call chain:

```
run_subprocess_agent (async fn)
  ├── tokio::fs::remove_file(result_path).await    [line 179, stale cleanup]
  ├── ... subprocess spawn + wait ...
  ├── read_result_file(result_path).await           [line 260]
  │     └── tokio::fs::read_to_string(path).await   [line 324]
  └── cleanup_result_file(result_path).await        [lines 264, 271]
        └── tokio::fs::remove_file(path).await       [line 340]
```

### Component Breakdown

#### `read_result_file` (agent.rs)

**Purpose:** Read and parse a subprocess result JSON file into a `PhaseResult`.

**Changes:**
- Signature: `pub fn read_result_file(path: &Path) -> Result<PhaseResult, String>` becomes `pub async fn read_result_file(path: &Path) -> Result<PhaseResult, String>`
- Body: `fs::read_to_string(path)` becomes `tokio::fs::read_to_string(path).await`
- Error handling: Unchanged — `map_err` with `ErrorKind::NotFound` differentiation works identically because `tokio::fs` returns `std::io::Result<T>` with the same `io::Error` values (confirmed in tech research: tokio::fs delegates to std::fs internally)

#### `cleanup_result_file` (agent.rs)

**Purpose:** Delete the result file after successful read; log warning on failure.

**Changes:**
- Signature: `fn cleanup_result_file(path: &Path)` becomes `async fn cleanup_result_file(path: &Path)`
- Body: `fs::remove_file(path)` becomes `tokio::fs::remove_file(path).await`
- Error handling: Unchanged — `log_warn!` on error, no propagation

#### `run_subprocess_agent` (agent.rs)

**Purpose:** Orchestrate subprocess execution and result collection.

**Changes:**
- Line 179: `fs::remove_file(result_path)` becomes `tokio::fs::remove_file(result_path).await`
- Line 260: `read_result_file(result_path)` becomes `read_result_file(result_path).await`
- Lines 264, 271: `cleanup_result_file(result_path)` becomes `cleanup_result_file(result_path).await`
- `run_subprocess_agent` is already `async fn`, so no signature change needed

#### Import changes (agent.rs)

- Remove: `use std::fs;`
- The `std::fs` import has exactly 3 uses, all targets of this change, so it becomes unused

### Data Flow

No changes to data flow. The same data moves through the same path; only the execution model changes from synchronous (blocking the tokio worker thread) to asynchronous (yielding the thread during I/O).

### Key Flows

#### Flow: Stale Result File Cleanup (pre-spawn)

> Remove any leftover result file from a previous run before spawning a new subprocess.

1. **Attempt removal** — `tokio::fs::remove_file(result_path).await`
2. **Success** — File removed; `log_warn!` noting stale file was found, then continue to subprocess spawn
3. **Handle NotFound** — Silently ignored (expected case: no stale file exists)
4. **Handle other errors** — Return error with descriptive message

**Edge cases:**
- No stale file exists — `NotFound` is silently ignored (unchanged behavior)
- Permission error — Error propagated (unchanged behavior)

#### Flow: Result File Read + Parse

> Read the subprocess result JSON file and parse it into a `PhaseResult`.

1. **Read file** — `tokio::fs::read_to_string(path).await`
2. **Parse JSON** — `serde_json::from_str` (sync, no change needed)
3. **Return result** — `Ok(PhaseResult)` or descriptive `Err(String)`

**Edge cases:**
- File not found — Specific error message "Result file not found: {path}" (unchanged)
- Other I/O error — Generic error with path and error detail (unchanged)
- Invalid JSON — Parse error with path and serde error (unchanged)

#### Flow: Result File Cleanup (post-read)

> Delete the result file after successful processing.

1. **Attempt removal** — `tokio::fs::remove_file(path).await`
2. **Handle failure** — `log_warn!` but do not propagate (unchanged behavior)

---

## Technical Decisions

### Key Decisions

#### Decision: Use `tokio::fs` over manual `spawn_blocking`

**Context:** Two options exist for making filesystem calls async-safe: `tokio::fs` (per-call async wrapper) or manual `spawn_blocking` (batch blocking calls in a closure).

**Decision:** Use `tokio::fs` for all 3 calls.

**Rationale:**
- The 3 calls are independent and interspersed with async logic (subprocess spawn, timeout wait, status matching)
- `tokio::fs` is a drop-in replacement with identical error types
- Less boilerplate than `spawn_blocking`
- Consistent with tokio ecosystem best practices for isolated fs calls
- The codebase reserves `spawn_blocking` for batched operations (coordinator.rs git ops)

**Consequences:** Each call incurs a tiny `spawn_blocking` overhead internally (~microseconds). This is negligible for 3 isolated calls.

#### Decision: Convert test functions to `#[tokio::test]`

**Context:** `read_result_file` becoming `async fn` means its 4 sync test callers won't compile.

**Decision:** Convert the 4 test functions from `#[test] fn` to `#[tokio::test] async fn` and add `.await` to the `read_result_file` calls.

The 4 test functions requiring conversion:
- `read_result_file_valid_json` (line 51)
- `read_result_file_missing_file` (line 65)
- `read_result_file_invalid_json` (line 80)
- `read_result_file_missing_required_fields` (line 96)

**Rationale:** This is the standard pattern already used by 9+ other tests in the same file. No alternative exists — you must use an async runtime to call async functions in tests.

**Consequences:** None negative. The test file already has `tokio` as a dev-dependency and uses `#[tokio::test]` extensively.

#### Decision: Keep test fixture setup synchronous

**Context:** Test functions use `fs::write()` to create JSON fixture files before calling `read_result_file`.

**Decision:** Leave `fs::write()` calls in tests as-is (blocking).

**Rationale:** Test fixture setup runs before the async operation under test. Converting it to async would add noise without benefit. The fixture writes are to temp files on local disk and complete instantly.

**Consequences:** Tests will still use `std::fs::write` for setup. This is fine — it's not in the production async path.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Micro overhead | ~microsecond per-call `spawn_blocking` overhead in `tokio::fs` | Clean, minimal-boilerplate async code | 3 isolated calls; overhead is immeasurable in practice |
| Breaking `read_result_file` API | All callers must add `.await` | Async-correct public API | Only 5 call sites (verified); one-time migration cost |

---

## Alternatives Considered

### Alternative: Manual `spawn_blocking` wrappers

**Summary:** Wrap each `std::fs` call in `tokio::task::spawn_blocking(move || { ... }).await.unwrap()`.

**How it would work:**
- Each fs call gets a `spawn_blocking` closure
- Results are unwrapped from the `JoinHandle`

**Pros:**
- Explicit about what's happening at the execution level

**Cons:**
- More boilerplate (closure, move semantics, JoinHandle unwrap)
- `tokio::fs` does exactly this internally, so it's redundant wrapping
- Inconsistent with the pattern of using `spawn_blocking` for batches only

**Why not chosen:** `tokio::fs` provides the same behavior with less code. Manual `spawn_blocking` is better suited for batching multiple related operations (as done in coordinator.rs for git ops).

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Missed caller of `read_result_file` | Compile error | Very Low | Grep verified exactly 5 call sites; compiler will catch any misses (Rust's `#[must_use]` on `Future` warns if `.await` is forgotten) |
| Behavioral difference between `tokio::fs` and `std::fs` | Incorrect error handling | Very Low | `tokio::fs` delegates to `std::fs` internally; error types are identical |

---

## Integration Points

### Existing Code Touchpoints

- `src/agent.rs` — Convert `read_result_file` (pub) and `cleanup_result_file` (private) to async; replace 3 `std::fs` calls with `tokio::fs`; remove `use std::fs;`
- `tests/agent_test.rs` — Convert 4 test functions from `#[test]` to `#[tokio::test]`; add `.await` to `read_result_file` calls

### External Dependencies

- `tokio::fs` — Already available via `tokio = { features = ["full"] }` in Cargo.toml. No new dependencies.

---

## Open Questions

None. The change is fully understood with a clear implementation path.

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
| 2026-02-12 | Initial design draft | Mechanical tokio::fs replacement; light mode, no alternatives worth deep analysis |
| 2026-02-12 | Self-critique (7 agents) + auto-fixes | Added explicit test function names, success path in stale cleanup flow, error type parity evidence, compiler safety note; dismissed over-analysis of race conditions/shutdown/rollback as irrelevant to mechanical conversion |
