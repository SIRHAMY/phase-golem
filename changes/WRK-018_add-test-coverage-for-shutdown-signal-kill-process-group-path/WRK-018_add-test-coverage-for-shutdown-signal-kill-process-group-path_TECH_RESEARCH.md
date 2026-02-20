# Tech Research: Add test coverage for shutdown signal kill_process_group path

**ID:** WRK-018
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-018_add-test-coverage-for-shutdown-signal-kill-process-group-path_PRD.md
**Mode:** Light

## Overview

Research how to add test coverage for the shutdown signal `kill_process_group` code path in `run_subprocess_agent` (lines 265-270 of `src/agent.rs`). The main technical question is how to control the global shutdown flag (`OnceLock<Arc<AtomicBool>>`) from test code, given the distinction between unit tests and integration tests in Rust's compilation model.

## Research Questions

- [x] How should the test-only setter be exposed — `#[cfg(test)]` vs unconditional `pub`?
- [x] Where should the test live — integration test (`tests/agent_test.rs`) or unit test (`src/agent.rs`)?
- [x] How to handle test isolation for global static state?
- [x] Are the PRD's line number references and assumptions still accurate?

---

## External Research

### Landscape Overview

Testing code that depends on global static state is a common challenge in Rust. The ecosystem has converged on a small set of pragmatic patterns: `#[cfg(test)]` setters for unit tests, `pub`/`pub(crate)` setters for integration tests, explicit cleanup for state reset, and `serial_test` for serializing tests that share mutable globals.

### Common Patterns & Approaches

#### Pattern: `#[cfg(test)]` setter function

**How it works:** A `pub fn` gated with `#[cfg(test)]` that directly stores to the underlying `AtomicBool`. Compiled only during `cargo test` for the same crate.

**When to use:** Unit tests within the same crate (`#[cfg(test)] mod tests {}` blocks in `src/`).

**Tradeoffs:**
- Pro: Zero production code footprint — compiled out entirely
- Pro: Idiomatic Rust pattern, well-documented in The Rust Book
- Con: **Not visible to integration tests** in `tests/` — this is the critical constraint (see Critical Areas below)

**References:**
- [The Rust Book — Test Organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html) — canonical reference
- [GitHub rust-lang/cargo#8379](https://github.com/rust-lang/cargo/issues/8379) — open request to relax this

#### Pattern: Unconditional `pub` setter with test-only naming

**How it works:** A `pub fn set_shutdown_flag_for_testing(value: bool)` that ships in the binary but is clearly named as test-only.

**When to use:** When integration tests need to control global state.

**Tradeoffs:**
- Pro: Accessible from integration tests in `tests/`
- Pro: No conditional compilation complexity
- Con: Ships in production binary (trivial cost — single `AtomicBool::store`)
- Con: Relies on naming convention to prevent misuse

#### Pattern: Explicit flag reset (teardown cleanup)

**How it works:** Every test that mutates global state resets it to a known baseline after assertions.

**When to use:** Always, when any test mutates shared global state.

**Tradeoffs:**
- Pro: Simple, zero-dependency
- Con: Not panic-safe — if test panics before reset, flag leaks to subsequent tests

#### Pattern: `#[serial]` from `serial_test` crate

**How it works:** Procedural macro that serializes test execution via mutex locking. Compatible with `#[tokio::test]`.

**When to use:** When parallel test interference occurs despite explicit flag reset.

**Tradeoffs:**
- Pro: 75M+ downloads, actively maintained, de facto standard for this problem
- Pro: Targeted serialization (doesn't slow entire suite)
- Con: Adds a dev-dependency

**References:**
- [serial_test on crates.io](https://crates.io/crates/serial_test)
- [serial_test docs.rs](https://docs.rs/serial_test/latest/serial_test/)

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| `#[cfg(test)]` invisible to integration tests | Library is compiled without `cfg(test)` when linked by `tests/` crate; setter won't exist | Use unit test in `src/` or unconditional `pub` setter |
| Panic leaves global state dirty | Test panics after setting flag, subsequent tests see stale state | Reset flag before assertions, or use `scopeguard`/Drop guard |
| `Ordering::Relaxed` insufficient | Could be a concern if flag and data don't have happens-before relationship | Safe here: `await` on subprocess creates happens-before; `Relaxed` is correct |

### Key Learnings

- The `#[cfg(test)]` + integration test visibility gap is the single most important finding for this task
- The PRD's proposed approach (`#[cfg(test)]` setter + test in `tests/agent_test.rs`) will not compile
- The simplest fix is to place the test in a unit test module inside `src/agent.rs`
- `Ordering::Relaxed` is correct for this use case (the `await` establishes happens-before)

---

## Internal Research

### Existing Codebase State

**Relevant files/modules:**
- `src/agent.rs:20-28` — `shutdown_flag()` (private), `is_shutdown_requested()` (pub), `OnceLock<Arc<AtomicBool>>` pattern
- `src/agent.rs:265-270` — untested shutdown signal path: check flag → `kill_process_group` → `Err("Shutdown requested")`
- `src/agent.rs:300-333` — `kill_process_group` implementation (handles ESRCH gracefully)
- `tests/agent_test.rs` — 11 existing integration tests using `TempDir`, `tokio::test`, mock shell scripts
- `tests/common/mod.rs` — test utilities including `fixtures_dir()`
- `tests/fixtures/mock_agent_success.sh` — writes valid JSON, exits 0 (suitable for shutdown test)

**Existing patterns in use:**
- `#[cfg(test)] mod tests {}` — used in `coordinator.rs`, `scheduler.rs`, `log.rs`, `lock.rs` for unit tests
- Integration tests in `tests/` — used in `agent_test.rs` for subprocess lifecycle tests
- No `#[cfg(test)]` blocks currently exist in `agent.rs`
- No `serial_test` dependency in `Cargo.toml`

### Reusable Components

- `mock_agent_success.sh` — exits 0, writes valid result JSON; perfect for reaching the shutdown check (line 266)
- `common::fixtures_dir()` — locates fixture scripts
- `TempDir` — automatic cleanup for result files
- Existing assertion patterns: `assert!(err.contains("..."), "Expected '...' in: {}", err)`

### Constraints from Existing Code

- **OnceLock initialization:** `Arc<AtomicBool>` initialized once per process; the atomic value can be freely stored/loaded
- **Parallel test execution:** `cargo test` runs tests in parallel within a single process; global flag mutations can leak
- **Process registry:** `unregister_child(pgid)` at line 263 happens before shutdown check — process already removed when `kill_process_group` is called on it (ESRCH, handled gracefully)

### PRD Accuracy

All line number references verified as accurate:
- Lines 20-28: `shutdown_flag()`, `is_shutdown_requested()` — correct
- Lines 265-270: shutdown check → `kill_process_group` → `Err("Shutdown requested")` — correct
- Lines 300-333: `kill_process_group` implementation — correct
- `mock_agent_success.sh` exists and is suitable
- No `set_shutdown_flag_for_testing` exists yet
- No `serial_test` dependency exists yet

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Setter gated with `#[cfg(test)]` and test in `tests/agent_test.rs` | `#[cfg(test)]` functions are invisible to integration tests — library is compiled without `cfg(test)` for `tests/` crate | Must either (A) place test in unit test module inside `src/agent.rs`, or (B) make setter unconditionally `pub` |
| Setter signature: `pub fn set_shutdown_flag_for_testing(value: bool)` | Signature is correct; `Ordering::Relaxed` is appropriate | No issue with the implementation itself — only with visibility |

---

## Critical Areas

### `#[cfg(test)]` Visibility to Integration Tests

**Why it's critical:** The PRD's proposed approach will cause a compile error. This is the single blocker for the change.

**Why it's easy to miss:** The `#[cfg(test)]` pattern is well-documented for unit tests, and the distinction is subtle — many Rust developers don't encounter it until they hit the compile error.

**What to watch for:** The design must choose one of the two approaches (unit test in `src/` or unconditional setter). Both are valid; the choice affects where the test code lives.

---

## Deep Dives

(None needed — light mode research.)

---

## Synthesis

### Open Questions

(None — all research questions resolved.)

### Recommended Approaches

#### Setter Visibility and Test Location

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| A: Unit test in `src/agent.rs` with `#[cfg(test)]` setter | Zero production footprint; `#[cfg(test)]` is idiomatic; setter accesses private `shutdown_flag()` directly | Test lives in `src/` instead of `tests/`; breaks pattern of agent tests being in `tests/agent_test.rs` | You want the cleanest production binary and don't mind splitting test locations |
| B: Unconditional `pub` setter + integration test in `tests/agent_test.rs` | All agent tests in one place; no conditional compilation | Trivial production overhead (one function that stores a bool); relies on naming convention | You want all agent tests co-located and don't mind the minimal production footprint |

**Initial recommendation:** Approach A (unit test in `src/agent.rs`). Reasons:
1. Follows the existing codebase pattern — `coordinator.rs`, `scheduler.rs`, `log.rs`, `lock.rs` all have `#[cfg(test)] mod tests {}` blocks
2. Zero production code added (the setter compiles out)
3. The test is specifically about internal state management (shutdown flag), which is a natural fit for a unit test
4. The integration tests in `tests/agent_test.rs` test the subprocess lifecycle from the outside; this test exercises an internal flag-checking path

#### Test Isolation

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Explicit flag reset | Simple, no new dependency | Not panic-safe | Default starting point; sufficient for a single test |
| `#[serial]` from `serial_test` | Guarantees serialization | Adds dev-dependency | Parallel test interference observed |

**Initial recommendation:** Start with explicit flag reset. The PRD identifies `serial_test` as an escalation path if needed — this is correct. With a single shutdown test, interference is unlikely.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Rust Book — Test Organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html) | Official docs | Explains `#[cfg(test)]` visibility rules |
| [cargo#8379](https://github.com/rust-lang/cargo/issues/8379) | GitHub issue | Open request to make `cfg(test)` visible to integration tests |
| [serial_test](https://crates.io/crates/serial_test) | Crate | De facto standard for serializing tests with shared state |
| [rust-lang/rust#84629](https://github.com/rust-lang/rust/issues/84629) | GitHub issue | Documents `cfg(test)` cross-crate limitation |

---

## Assumptions

Decisions made without human input during autonomous research:

1. **Research mode: light** — This is a small, well-understood task (adding a test for a flag check). Medium/heavy research would be excessive.
2. **Recommended Approach A over B** — Unit test in `src/agent.rs` follows existing codebase patterns and avoids shipping test-only code. Both approaches are valid.

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Parallel external + internal research (light mode) | Identified `#[cfg(test)]` visibility issue as critical divergence from PRD; verified all PRD assumptions except setter visibility; documented two recommended approaches |
