# Design: Add test coverage for shutdown signal kill_process_group path

**ID:** WRK-018
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-018_add-test-coverage-for-shutdown-signal-kill-process-group-path_PRD.md
**Tech Research:** ./WRK-018_add-test-coverage-for-shutdown-signal-kill-process-group-path_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add a unit test inside `src/agent.rs` (within a `#[cfg(test)] mod tests {}` block) that exercises the shutdown signal code path at lines 265-270. A `#[cfg(test)]` setter function provides test-only control over the global shutdown flag. The test sets the flag before spawning a subprocess, runs `run_subprocess_agent` with a mock script that exits successfully, and asserts the function returns `Err("Shutdown requested")`. This approach follows existing codebase patterns (unit test modules in `coordinator.rs`, `scheduler.rs`, `log.rs`, `lock.rs`) and keeps test-only code out of the production binary.

---

## System Design

### High-Level Architecture

No new components or architectural changes. This design adds:

1. A `#[cfg(test)]` setter function in `src/agent.rs` to control the shutdown flag
2. A `#[cfg(test)] mod tests {}` block in `src/agent.rs` with one async test

Both are compiled only during `cargo test` for the `phase_golem` crate.

### Component Breakdown

#### `set_shutdown_flag_for_testing(value: bool)`

**Purpose:** Allow tests to set/reset the global `shutdown_flag()` without sending real OS signals.

**Placement:** Module-level function in `src/agent.rs`, gated with `#[cfg(test)]`, placed immediately before the `#[cfg(test)] mod tests {}` block. Not `pub` — only needs to be visible within the crate's test compilation.

**Signature:** `#[cfg(test)] fn set_shutdown_flag_for_testing(value: bool)`

**Responsibilities:**
- Store `value` to the `AtomicBool` inside `shutdown_flag()` using `Ordering::Relaxed`

**Interfaces:**
- Input: `bool` value
- Output: none (void)

**Dependencies:** `shutdown_flag()` (private function in same module)

#### Test: `shutdown_signal_kills_process_group_and_returns_error`

**Purpose:** Verify that when `is_shutdown_requested()` returns true after a subprocess exits normally, `run_subprocess_agent` returns `Err` containing `"Shutdown requested"`.

**Responsibilities:**
- Set shutdown flag to `true` via `set_shutdown_flag_for_testing`
- Build a `tokio::process::Command` for `mock_agent_success.sh`
- Call `run_subprocess_agent` with the command
- Assert the result is `Err` containing `"Shutdown requested"`
- Reset shutdown flag to `false` (cleanup)

**Interfaces:**
- Input: none (test function)
- Output: test pass/fail

**Dependencies:** `run_subprocess_agent`, `set_shutdown_flag_for_testing`, `mock_agent_success.sh` fixture

### Data Flow

1. Test calls `set_shutdown_flag_for_testing(true)` — stores `true` to global `AtomicBool`
2. Test builds a `Command` pointing to `tests/fixtures/mock_agent_success.sh`
3. Test calls `run_subprocess_agent(cmd, result_path, timeout)`
4. `run_subprocess_agent` spawns subprocess, waits for it to exit (exit 0)
5. After normal exit, function checks `is_shutdown_requested()` — returns `true`
6. Function calls `kill_process_group(child_pid)` (ESRCH expected — process already exited)
7. Function returns `Err("Shutdown requested".to_string())`
8. Test asserts error contains `"Shutdown requested"`
9. Test calls `set_shutdown_flag_for_testing(false)` — cleanup

### Key Flows

#### Flow: Shutdown signal path test

> Verify that `run_subprocess_agent` returns the correct error when shutdown is requested.

1. **Setup** — Create `TempDir` for result file, set shutdown flag to `true`
2. **Build command** — Build `tokio::process::Command` for `mock_agent_success.sh` using `env!("CARGO_MANIFEST_DIR")` to locate the fixture: `Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mock_agent_success.sh")`. Pass the result file path as the first argument (`.arg(&result_path)`).
3. **Execute** — Call `run_subprocess_agent(cmd, result_path, 30s timeout)`. The test is annotated with `#[tokio::test]` for async runtime support.
4. **Assert** — Result is `Err(e)` where `e.contains("Shutdown requested")`
5. **Cleanup** — Reset shutdown flag to `false`

**Edge cases:**
- Subprocess writes valid result JSON, but it's never read because the shutdown check at line 266 precedes the result file read at line 273. The result file is intentionally left unread; `TempDir` handles cleanup on drop.
- `kill_process_group` receives ESRCH because process already exited — handled gracefully by existing implementation (line 311-312). This is expected and benign.
- `Ordering::Relaxed` is correct for the flag store/load because the `await` on subprocess completion establishes a happens-before relationship, ensuring the flag value is visible when checked at line 266.

---

## Technical Decisions

### Key Decisions

#### Decision: Unit test in `src/agent.rs` (not integration test in `tests/agent_test.rs`)

**Context:** The PRD proposed placing the test in `tests/agent_test.rs`, but tech research found that `#[cfg(test)]` functions are invisible to integration tests. This is a **scope divergence from the PRD** — the PRD's stated approach (line 56: "Adding test(s) to `tests/agent_test.rs`") would cause a compile error because Rust compiles the library without `cfg(test)` when building for integration test crates.

**Decision:** Place the test in a `#[cfg(test)] mod tests {}` block inside `src/agent.rs`.

**Rationale:**
- The PRD's original approach is not feasible due to Rust's `cfg(test)` visibility constraints (see Tech Research: Critical Areas)
- Follows existing patterns: `coordinator.rs`, `scheduler.rs`, `log.rs`, `lock.rs` all have `#[cfg(test)] mod tests {}` blocks
- The test exercises internal state management (shutdown flag), which is a natural fit for a unit test

**Consequences:** Agent tests will be split across two locations (`src/agent.rs` for flag-checking tests, `tests/agent_test.rs` for subprocess lifecycle tests). This is acceptable because they test different concerns (internal flag state vs subprocess lifecycle).

#### Decision: Explicit flag reset (not `#[serial]`)

**Context:** The global shutdown flag could leak between tests if not reset. Tech research identified this as a common pitfall: "Panic leaves global state dirty — if test panics before reset, flag leaks to subsequent tests."

**Decision:** Each test explicitly resets the flag to `false` after assertions. Escalate to `#[serial]` only if parallel interference is observed.

**Rationale:**
- With a single shutdown test, interference is unlikely
- Avoids adding a dev-dependency for a single test
- PRD identifies `serial_test` as an escalation path

**Limitation:** Explicit reset is not panic-safe. If the test panics before the reset line executes, the flag leaks to subsequent tests. This is an accepted risk given: (a) the test is straightforward with low panic probability, and (b) `#[serial]` can be added as an immediate fix if flaky test behavior is observed.

**Consequences:** If additional shutdown flag tests are added in the future and interference occurs, `serial_test` should be added as a dev-dependency at that point.

#### Decision: Set flag before spawning subprocess

**Context:** The shutdown flag could be set before or after the subprocess runs.

**Decision:** Set the flag before spawning. The flag is already `true` when the shutdown check at line 266 executes.

**Rationale:**
- Deterministic — no timing dependency between flag setting and subprocess completion
- Tests the defensive behavior of the shutdown path (flag is checked after every subprocess exit)
- Simpler than trying to coordinate flag-setting during subprocess execution

**Consequences:** Does not test the race condition of a signal arriving mid-execution. This is acceptable — the test verifies the flag-checking logic, not signal delivery timing.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Split test locations | Agent tests live in two files | Zero production code footprint from test setter | Internal state tests belong in unit test module; subprocess lifecycle tests belong in integration tests |
| Pre-set flag | Doesn't test real-time signal arrival | Deterministic, fast test execution | Signal delivery is a separate concern; this test verifies the code path logic |

---

## Alternatives Considered

### Alternative: Unconditional `pub` setter + integration test in `tests/agent_test.rs`

**Summary:** Make `set_shutdown_flag_for_testing` unconditionally `pub` (not `#[cfg(test)]`), keep the test in `tests/agent_test.rs` alongside other agent tests.

**How it would work:**
- Add `pub fn set_shutdown_flag_for_testing(value: bool)` without `#[cfg(test)]` gating
- Test lives in `tests/agent_test.rs` with all other agent tests

**Pros:**
- All agent tests co-located in one file
- No conditional compilation complexity

**Cons:**
- Ships a test-only function in the production binary (trivial cost, but unnecessary)
- Relies on naming convention (`_for_testing`) to prevent misuse

**Why not chosen:** The codebase already has an established pattern of `#[cfg(test)] mod tests {}` blocks in `src/` files. Following this pattern keeps test-only code out of production and is idiomatic Rust.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Global shutdown flag leaks between tests | Other tests may see stale `true` flag and fail unexpectedly | Low (single test, explicit reset) | Explicit `set_shutdown_flag_for_testing(false)` after assertions; escalate to `#[serial]` if observed |
| `mock_agent_success.sh` path differs for unit vs integration tests | Test fails to find fixture script | Low | Use `env!("CARGO_MANIFEST_DIR")` to build absolute path to `tests/fixtures/mock_agent_success.sh` (standard Rust pattern for locating project-relative paths from unit tests) |

---

## Integration Points

### Existing Code Touchpoints

- `src/agent.rs:20-23` — `shutdown_flag()` function: accessed by the new setter (no modification needed, just called)
- `src/agent.rs:183-298` — `run_subprocess_agent()`: called by the test (no modification needed)
- `tests/fixtures/mock_agent_success.sh` — reused as the mock subprocess

### External Dependencies

- `tempfile::TempDir` — used for temporary result file storage. Already a dev-dependency in `Cargo.toml` and used by existing integration tests in `tests/agent_test.rs`.
- `tests/fixtures/mock_agent_success.sh` — existing fixture script that exits 0 and writes valid result JSON. Located via `env!("CARGO_MANIFEST_DIR")` at compile time.

---

## Open Questions

None — all questions resolved during tech research.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain

---

## Assumptions

Decisions made without human input during autonomous design:

1. **Explicit reset over `#[serial]`** — 6 of 7 critique agents flagged panic-safety of the explicit flag reset approach. Two valid options: (a) add `#[serial]` from the start, or (b) accept the explicit reset with documented limitation. Chose (b) because: the PRD itself identifies `#[serial]` as an escalation path, there's only one shutdown test, and adding a dev-dependency for a single test is disproportionate. Documented the panic-safety limitation explicitly.
2. **Design mode: light** — Small, well-understood task (adding one test + one setter function). No architectural complexity warranting medium/heavy design.

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Initial design draft | Unit test in `src/agent.rs` with `#[cfg(test)]` setter; follows existing codebase patterns |
| 2026-02-20 | Self-critique (7 agents) | Auto-fixed: PRD divergence callout, fixture path resolution details, `#[tokio::test]` annotation, Ordering::Relaxed rationale, panic-safety limitation, setter placement details, external dependencies |
