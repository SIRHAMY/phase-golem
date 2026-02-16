# WRK-013: Extract shared test helpers to tests/common/mod.rs

## Problem Statement

The orchestrator's integration test suite (`orchestrator/tests/`) contains 14 test files with significant duplication of helper functions. The same factory/builder functions for creating test data (backlog items, configs, git environments) are copy-pasted across 6-10 files, making maintenance harder and increasing the risk of test helpers drifting out of sync.

## Proposed Approach

1. Create `orchestrator/tests/common/mod.rs` following the standard Rust pattern for shared test utilities.
2. Extract duplicated helpers into logical groupings:
   - **Backlog builders:** `make_item`, `make_in_progress_item`, `make_blocked_item`, `empty_backlog`, `make_backlog` (duplicated in 6+ files)
   - **Git environment setup:** `setup_test_env`, `setup_temp_repo` (duplicated in 4+ files)
   - **Config builders:** `make_phase_config`, `default_config`, `default_guardrails`, `default_feature_pipeline` (duplicated in 3+ files)
   - **Path helpers:** `fixtures_dir`, `fixture_path`, `backlog_path` (duplicated in 3+ files)
3. Replace duplicated definitions in each test file with `mod common;` import and qualified usage.
4. Verify all tests still pass after extraction.

## Files Affected

- **New:** `orchestrator/tests/common/mod.rs`
- **Modified (remove duplication):**
  - `tests/backlog_test.rs`
  - `tests/coordinator_test.rs`
  - `tests/executor_test.rs`
  - `tests/scheduler_test.rs`
  - `tests/preflight_test.rs`
  - `tests/prompt_test.rs`
  - `tests/git_test.rs`
  - `tests/agent_test.rs`
  - `tests/migration_test.rs`
  - `tests/worklog_test.rs`

## Assessment

| Dimension  | Rating | Rationale |
|------------|--------|-----------|
| Size       | Medium | 1 new file + ~10 modified test files |
| Complexity | Low    | Straightforward extract-and-deduplicate; well-known Rust pattern |
| Risk       | Low    | Only test code affected; no production interfaces changed |
| Impact     | Medium | Reduces maintenance burden; prevents helper drift across test files |

## Assumptions

- Helpers that are truly unique to a single test file should remain in that file; only extract functions duplicated in 2+ files.
- A flat `common/mod.rs` is sufficient (no need for sub-modules within common) given the current scope.
- Some helpers may have slight signature variations across files — unify to the most general version.
