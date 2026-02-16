# Tech Research: Wrap blocking filesystem ops in run_subprocess_agent with async equivalents

**ID:** WRK-016
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-016_PRD.md
**Mode:** Light

## Overview

Researching the correct approach for converting 3 blocking `std::fs` calls in `agent.rs` to async equivalents. The key questions are: should we use `tokio::fs` or manual `spawn_blocking`, are the APIs compatible (same error types, same behavior), and are there any gotchas to watch for?

## Research Questions

- [x] Should we use `tokio::fs` or `spawn_blocking` for these 3 independent fs calls?
- [x] Are `tokio::fs::remove_file` and `tokio::fs::read_to_string` API-compatible with their `std::fs` equivalents?
- [x] Do error types and `ErrorKind::NotFound` matching work identically?
- [x] What does the existing codebase already do for async/blocking patterns?
- [x] Are there any gotchas with `tokio::fs` for this use case?

---

## External Research

### Landscape Overview

The `tokio::fs` module provides async equivalents of all major `std::fs` functions. Since most operating systems lack truly asynchronous filesystem APIs, `tokio::fs` is implemented as a thin wrapper that delegates each operation to `std::fs` via an internal `asyncify` helper (which uses `spawn_blocking`). This is the standard and recommended approach in the Rust async ecosystem.

For isolated, one-off filesystem operations interspersed with async logic (like WRK-016's 3 calls), `tokio::fs` is preferred over manual `spawn_blocking`. Manual `spawn_blocking` is better when batching multiple related blocking operations into a single closure.

### Common Patterns & Approaches

#### Pattern: Direct tokio::fs Drop-in Replacement

**How it works:** Replace `std::fs::remove_file(path)` with `tokio::fs::remove_file(path).await`, and similarly for `read_to_string`. The containing function must be `async fn`.

**When to use:** Individual blocking fs calls interspersed with async logic, where each call is independent.

**Tradeoffs:**
- Pro: Minimal boilerplate, drop-in replacement, clear intent
- Pro: Internally uses `spawn_blocking`, so async worker threads stay free
- Con: Slight overhead per call (microseconds) from spawn_blocking — negligible for one-off operations

**References:**
- [tokio::fs module docs](https://docs.rs/tokio/latest/tokio/fs/index.html) — official docs covering functions, caveats, and performance
- [tokio::fs::remove_file source](https://github.com/tokio-rs/tokio/blob/master/tokio/src/fs/remove_file.rs) — confirms `asyncify(move || std::fs::remove_file(path)).await`
- [tokio::fs::read_to_string source](https://github.com/tokio-rs/tokio/blob/master/tokio/src/fs/read_to_string.rs) — confirms `asyncify(move || std::fs::read_to_string(path)).await`

#### Pattern: Manual spawn_blocking Batching

**How it works:** Group multiple blocking fs calls into a single `tokio::task::spawn_blocking(move || { ... })` closure.

**When to use:** Multiple related blocking operations with no async work between them, or performance-critical code with many fs calls.

**Tradeoffs:**
- Pro: Reduces thread pool overhead for batches
- Con: More boilerplate, requires moving data into closure

**References:**
- [spawn_blocking docs](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
- Already used in codebase: `coordinator.rs` lines 547+ for git operations

### Technologies & Tools

| Technology | Purpose | Pros | Cons | Used With Patterns |
|------------|---------|------|------|-------------------|
| [tokio::fs](https://docs.rs/tokio/latest/tokio/fs/index.html) | Async filesystem ops | Drop-in replacement, minimal code change | Micro overhead per call | Direct replacement |
| [spawn_blocking](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) | Run blocking code off async threads | Flexible, good for batches | More boilerplate | Manual batching |

### Standards & Best Practices

1. **Error type compatibility is guaranteed.** Both `tokio::fs::remove_file` and `tokio::fs::read_to_string` return `std::io::Result<T>`. Error kinds (including `ErrorKind::NotFound`) are identical because `tokio::fs` calls `std::fs` internally and propagates errors unchanged.

2. **Function signatures are near-identical.** Only differences: `tokio::fs` functions are `async` (return a Future) and require `.await` at call sites. Parameter types (`impl AsRef<Path>`) and return types (`io::Result<T>`) are the same.

3. **Prefer `tokio::fs` over `spawn_blocking` for isolated operations.** Tokio docs and maintainers recommend this approach.

4. **Test conversion:** When a function becomes `async fn`, test functions calling it must use `#[tokio::test]` and `async fn` instead of `#[test]` and `fn`.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid | Relevant? |
|---------|-------------------|--------------|-----------|
| High-frequency call overhead | Each `tokio::fs` call incurs spawn_blocking overhead; 9-64x slower for thousands of calls in tight loops ([#3664](https://github.com/tokio-rs/tokio/issues/3664)) | Use spawn_blocking batching for hot paths | No — only 3 isolated calls |
| File write flush semantics | `tokio::fs::File::write()` returns before kernel flush | Call `flush()` explicitly | No — no streaming writes |
| Special files (pipes, FIFOs) | Can cause hangs during shutdown on Linux | Use `tokio::net::unix::pipe` instead | No — regular JSON files only |
| Forgetting `#[tokio::test]` | Tests calling async functions won't compile with `#[test]` | Convert test attribute and signature | Yes — 4 tests need conversion |

### Key Learnings

- The conversion is entirely mechanical: replace `fs::` with `tokio::fs::`, add `.await`, make functions `async fn`
- No error handling changes needed — `tokio::fs` returns identical `std::io::Error` values
- No performance concerns for 3 isolated calls on local files
- None of the documented `tokio::fs` pitfalls apply to this use case

---

## Internal Research

### Existing Codebase State

The orchestrator is an async Rust application using `tokio` with `"full"` features. The codebase demonstrates mature async patterns with clear separation between blocking and async operations. `tokio::fs` is not currently used anywhere in the codebase — this will be the first usage.

**Relevant files/modules:**
- `src/agent.rs` — Contains `run_subprocess_agent` (async), `read_result_file` (sync, pub), `cleanup_result_file` (sync, private). Has 3 blocking `std::fs` calls at lines 179, 324, 340
- `tests/agent_test.rs` — Contains 4 sync test functions calling `read_result_file` at lines 57, 70, 86, 106
- `src/coordinator.rs` — Reference: uses `spawn_blocking` for git operations (lines 547+)
- `Cargo.toml` — `tokio` with `features = ["full"]` already present (line 20)

**Existing patterns in use:**
- `spawn_blocking` for batched blocking ops (coordinator.rs git operations)
- `spawn_blocking` for blocking poll loops (agent.rs `kill_process_group`)
- `Result<T, String>` for error propagation throughout the codebase
- `log_warn!` / `log_debug!` macros for logging
- `#[tokio::test]` already used in agent_test.rs for other async tests (lines 112, 136, 155, 183+)
- `tempfile::TempDir` for test fixtures

### Reusable Components

- `tokio::fs` is already available via `tokio = { features = ["full"] }` — no new dependencies needed
- `#[tokio::test]` pattern is already established in the test file for other tests
- Error handling patterns (`map_err`, `ErrorKind::NotFound` matching) will work identically

### Constraints from Existing Code

- `read_result_file` is `pub` — changing to `async fn` is a breaking API change; all callers verified: `agent.rs:260` and 4 tests in `agent_test.rs`
- `cleanup_result_file` is private — only called from `run_subprocess_agent`, no external impact
- `MockAgentRunner` does not call these helpers — mock tests unaffected
- Test fixture setup code (`fs::write` for creating test JSON files) can remain blocking
- The `use std::fs;` import at line 2 has exactly 3 uses — all targets of this change — so it can be removed after conversion

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| `tokio::fs` is a drop-in replacement | Confirmed: identical signatures, identical error types, identical behavior | No surprises — conversion is mechanical |
| No new dependencies needed | Confirmed: `tokio` with `"full"` features provides `tokio::fs` | None — clean |
| `read_result_file` callers are agent.rs + 4 tests | Verified via grep: exactly 5 call sites | PRD is accurate |
| Test fixture `fs::write` can stay blocking | Correct: only the calls to `read_result_file` need async conversion | Reduces scope |

No concerns found. The PRD is accurate and well-aligned with research findings.

---

## Critical Areas

### Test Conversion from #[test] to #[tokio::test]

**Why it's critical:** The 4 test functions must be converted or they won't compile.

**Why it's easy to miss:** It's tempting to only change `agent.rs` and forget the test file.

**What to watch for:** Each test function needs both `#[tokio::test]` attribute AND `async fn` signature. The test body needs `.await` on the `read_result_file` call. Test fixture setup (`fs::write`) does NOT need conversion.

---

## Deep Dives

No deep dives needed — the problem space is well-understood and the conversion is mechanical.

---

## Synthesis

### Open Questions

None. All research questions have been answered and the implementation path is clear.

### Recommended Approaches

#### Async Conversion Strategy

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `tokio::fs` direct replacement | Minimal boilerplate, drop-in API, clear intent | Micro overhead per call | Isolated, independent fs calls (this case) |
| Manual `spawn_blocking` | Efficient for batches, flexible | More boilerplate, closure ownership | Multiple related blocking calls with no async between them |

**Initial recommendation:** Use `tokio::fs` direct replacement. The 3 fs calls are independent and interspersed with async logic, making `tokio::fs` the cleaner choice. This is also consistent with the PRD's recommendation and the codebase's existing pattern of using `spawn_blocking` only for batched operations.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [tokio::fs module docs](https://docs.rs/tokio/latest/tokio/fs/index.html) | Official Docs | API reference, caveats, performance guidance |
| [tokio::fs::remove_file docs](https://docs.rs/tokio/latest/tokio/fs/fn.remove_file.html) | Official Docs | Function signature and behavior |
| [tokio::fs::read_to_string docs](https://docs.rs/tokio/latest/tokio/fs/fn.read_to_string.html) | Official Docs | Function signature and behavior |
| [Issue #2926: Why tokio uses blocking file I/O](https://github.com/tokio-rs/tokio/issues/2926) | Discussion | Maintainer explanation of tokio::fs internals |
| [Issue #3664: tokio::fs performance](https://github.com/tokio-rs/tokio/issues/3664) | Discussion | Performance characteristics (confirms not a concern for one-off calls) |
| [spawn_blocking docs](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) | Official Docs | Understanding the mechanism tokio::fs uses internally |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Initial external + internal research (light mode) | Confirmed tokio::fs is drop-in compatible; verified all callers; no concerns found |
| 2026-02-12 | PRD analysis | PRD is accurate; no conflicts between research and PRD |
