# SPEC: Extract Shared Test Helpers to tests/common/mod.rs

**ID:** WRK-013
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-013_extract-shared-test-helpers_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

The orchestrate project's 14 integration test files contain ~350+ lines of duplicated helper functions across 8 files. The same `make_item`, `make_in_progress_item`, `setup_test_env`, `make_backlog`, `empty_backlog`, and fixture path helpers are copied between test files with variations in signatures, default values, and naming. This makes BacklogItem struct changes painful (update 7+ files) and test data inconsistent. Rust's standard `tests/common/mod.rs` pattern provides the idiomatic solution.

## Approach

Create a single `tests/common/mod.rs` file containing canonical versions of shared helpers, then migrate test files one-by-one: add `mod common;`, update call sites to use `common::` prefix, remove local definitions. Files with incompatible signatures (scheduler_test, prompt_test) keep their local item builders and only migrate compatible helpers (backlog constructors, environment setup, fixture paths).

All canonical helpers use minimal defaults (`None` for optional fields, empty vecs for collections). Files needing richer defaults (e.g., `pipeline_type: Some("feature")`) use inline mutation or local wrapper functions.

**Patterns to follow (locate by function name, line numbers are approximate):**

- `tests/backlog_test.rs` `fn make_item(id, status)` — canonical item builder (copy verbatim, set `pipeline_type: None`)
- `tests/executor_test.rs` `fn setup_test_env()` — canonical env setup (copy verbatim)
- `tests/executor_test.rs` `fn make_backlog(items)` — canonical backlog constructor (copy verbatim)
- `tests/backlog_test.rs` `fn fixture_path(name)` — canonical fixture path (copy verbatim)
- `tests/agent_test.rs` `fn fixtures_dir()` — canonical fixtures directory (copy verbatim)
- `tests/executor_test.rs` `fn default_config()` — canonical config builder (copy verbatim)

**Implementation boundaries:**

- Do not modify: any file in `src/` (production code)
- Do not modify: `tests/fixtures/` (test fixture data files)
- Do not modify: `Cargo.toml` (no new dependencies needed)
- Do not refactor: test logic or assertions (only update helper call sites)
- Do not modify: `tests/git_test.rs`, `tests/config_test.rs`, `tests/lock_test.rs`, `tests/types_test.rs`, `tests/worklog_test.rs`, `tests/agent_integration_test.rs`

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Create common module + migrate simple files | Low | Create `tests/common/mod.rs` with all 8 documented helpers; migrate backlog_test, agent_test, migration_test |
| 2 | Migrate complex files | Med | Migrate coordinator_test, executor_test, preflight_test, scheduler_test, prompt_test |
| 3 | Final verification and cleanup | Low | Full test suite verification, duplicate audit, baseline comparison |

**Ordering rationale:** Phase 1 creates the common module and proves it works with the simplest migrations (fewest local wrappers needed). Phase 2 handles files requiring local wrappers or partial migration. Phase 3 verifies the complete migration.

---

## Phases

---

### Phase 1: Create common module + migrate simple files

> Create tests/common/mod.rs with all 8 canonical helpers and migrate the 3 simplest test files

**Phase Status:** complete

**Complexity:** Low

**Goal:** Establish the shared common module with all helpers and prove it works by migrating backlog_test.rs, agent_test.rs, and migration_test.rs — the three files with straightforward, direct replacements.

**Files:**

- `tests/common/mod.rs` — create — shared test helper module (~120 lines)
- `tests/backlog_test.rs` — modify — remove `fixture_path`, `make_item`, `make_in_progress_item`, `empty_backlog`; add `mod common;`; update call sites
- `tests/agent_test.rs` — modify — remove `fixtures_dir`; add `mod common;`; update call sites
- `tests/migration_test.rs` — modify — remove `fixtures_dir`; add `mod common;`; update call sites

**Patterns:**

- Follow `tests/backlog_test.rs` `fn make_item` for canonical item builder (exact copy with `pipeline_type: None`)
- Follow `tests/executor_test.rs` `fn setup_test_env` for canonical env setup (exact copy)
- Migration pattern per file: (1) add `mod common;` before the first `use` statement (after any `#![...]` attributes), (2) update all call sites to `common::` prefix, (3) remove local helper definitions
- Rollback: any phase can be reverted via `git revert <commit>` — no data migration or external coordination needed

**Tasks:**

- [x] Run `cargo test` to establish baseline — record total passing test count. All tests must pass before any migration begins. If any tests fail, stop and report.
- [x] Create `tests/common/` directory and `tests/common/mod.rs`
- [x] Add `#![allow(dead_code)]` at top of common/mod.rs (not all helpers used by all files)
- [x] Implement `make_item(id: &str, status: ItemStatus) -> BacklogItem` with `///` doc comment — copy body from `backlog_test.rs` `fn make_item` verbatim (all 22 fields, `pipeline_type: None`, `phase_pool: None`, timestamp `"2026-02-10T00:00:00+00:00"`, title `format!("Test item {}", id)`). Doc comment: purpose, parameters, and default values.
- [x] Implement `make_in_progress_item(id: &str, phase: &str) -> BacklogItem` with `///` doc comment — calls `make_item(id, InProgress)`, sets `item.phase = Some(phase.to_string())`, leaves `phase_pool: None` (minimal default; callers set if needed)
- [x] Implement `make_backlog(items: Vec<BacklogItem>) -> BacklogFile` with `///` doc comment — copy body from `executor_test.rs` `fn make_backlog` (`schema_version: 2`, `next_item_id: 0`)
- [x] Implement `empty_backlog() -> BacklogFile` with `///` doc comment — calls `make_backlog(vec![])`
- [x] Implement `fixtures_dir() -> PathBuf` with `///` doc comment — copy body from `agent_test.rs` `fn fixtures_dir` (`PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")`)
- [x] Implement `fixture_path(name: &str) -> PathBuf` with `///` doc comment — `fixtures_dir().join(name)`
- [x] Implement `setup_test_env() -> TempDir` with `///` doc comment — copy body from `executor_test.rs` `fn setup_test_env` verbatim (git init, config, README commit, create `_ideas`, `_worklog`, `changes`, `.orchestrator` dirs)
- [x] Implement `default_config() -> OrchestrateConfig` with `///` doc comment — copy body from `executor_test.rs` `fn default_config` (`OrchestrateConfig::default()` + insert `"feature"` pipeline via `default_feature_pipeline()`)
- [x] Add required imports to common/mod.rs: `use orchestrate::config::{default_feature_pipeline, OrchestrateConfig};`, `use orchestrate::types::{BacklogFile, BacklogItem, ItemStatus};`, `use std::path::PathBuf;`, `use std::fs;`, `use std::process::Command;`, `use tempfile::TempDir;`
- [x] Verify `cargo test --no-run` compiles (common module exists but no test files use it yet — existing tests still have local helpers)
- [x] Migrate `backlog_test.rs`: add `mod common;` before first `use` statement, replace `fixture_path(` → `common::fixture_path(`, `make_item(` → `common::make_item(`, `make_in_progress_item(` → `common::make_in_progress_item(`, `empty_backlog()` → `common::empty_backlog()`, then remove local helper definitions (locate by function name: `fn fixture_path`, `fn make_item`, `fn make_in_progress_item`, `fn empty_backlog`)
- [x] Run `cargo test --test backlog_test` — all tests must pass
- [x] Migrate `agent_test.rs`: add `mod common;` before first `use` statement, replace `fixtures_dir()` → `common::fixtures_dir()`, remove local `fn fixtures_dir` definition
- [x] Run `cargo test --test agent_test` — all tests must pass
- [x] Migrate `migration_test.rs`: add `mod common;` before first `use` statement, replace `fixtures_dir()` → `common::fixtures_dir()`, remove local `fn fixtures_dir` definition
- [x] Run `cargo test --test migration_test` — all tests must pass

**Verification:**

- [x] `cargo test --test backlog_test` passes with zero failures
- [x] `cargo test --test agent_test` passes with zero failures
- [x] `cargo test --test migration_test` passes with zero failures
- [x] `tests/common/mod.rs` exists with 8 `pub fn` helpers, `#![allow(dead_code)]`, and `///` doc comments on each
- [x] No local definitions of extracted helpers remain in migrated files. Verify: `grep -n "^fn make_item\|^fn fixture_path\|^fn make_in_progress_item\|^fn empty_backlog\|^fn fixtures_dir" tests/backlog_test.rs tests/agent_test.rs tests/migration_test.rs` returns no results
- [x] All three migrated files have `mod common;` before their first `use` statement

**Commit:** `[WRK-013][P1] Clean: Extract shared test helpers to tests/common/mod.rs and migrate simple files`

**Notes:**

- Baseline: 390 tests passed, 1 ignored. Post-migration: identical — 390 passed, 1 ignored.
- The `make_in_progress_item` canonical version uses `phase_pool: None` (minimal default), diverging from the majority pattern where 3 of 4 files set `PhasePool::Main`. This is intentional — callers that need `PhasePool::Main` add it explicitly, making test intent visible.
- backlog_test.rs is the best first migration because its helpers exactly match the canonical signatures (no `pipeline_type`, no `phase_pool` in `make_in_progress_item`).
- All 8 helpers are created in Phase 1 (even those only consumed in Phase 2) because they are copied verbatim from existing passing tests — the risk of copy errors is low, and `cargo test --no-run` confirms compilation. This avoids modifying common/mod.rs in Phase 2 (which would trigger recompilation of Phase 1 migrated files).
- Also cleaned up now-unused imports (`PathBuf`, `BacklogFile`, `BacklogItem`) from migrated files. No clippy warnings introduced.

**Followups:**

---

### Phase 2: Migrate complex files

> Migrate coordinator_test, executor_test, preflight_test, scheduler_test, and prompt_test with file-specific strategies

**Phase Status:** complete

**Complexity:** Med

**Goal:** Complete all remaining test file migrations. Some files need local wrappers for `pipeline_type` or `phase_pool`, and some keep local item builders due to incompatible signatures.

**Files:**

- `tests/coordinator_test.rs` — modify — remove `setup_test_env`, `make_item`, `make_in_progress_item`, `empty_backlog`; add `mod common;`; keep local `make_blocked_item` (refactored to use `common::make_item`), `backlog_path`, `save_and_commit_backlog`
- `tests/executor_test.rs` — modify — remove `setup_test_env`, `make_item`, `make_in_progress_item`, `make_backlog`, `default_config`; add `mod common;`; add local wrapper for `pipeline_type`; keep local `make_scoping_item` (refactored), `make_phase_result`, `default_guardrails`, `make_simple_pipeline`
- `tests/preflight_test.rs` — modify — remove `make_item`, `make_backlog`; add `mod common;`; keep local `feature_pipeline_no_workflows`, `default_config` (incompatible pipeline)
- `tests/scheduler_test.rs` — modify — remove `setup_test_env`, `make_backlog`; add `mod common;`; keep local `make_item` (3-param), `make_in_progress_item` (3-param), `make_scoping_item`, `make_ready_item`, `default_config`, `default_execution_config`, `default_pipelines`, and other scheduler-specific helpers
- `tests/prompt_test.rs` — unchanged — does not use any common helpers; keeps all local builders (hardcoded InProgress semantics, different timestamp format)

**Tasks:**

- [x] **coordinator_test.rs**: Add `mod common;` before first `use` statement. Replace `setup_test_env()` → `common::setup_test_env()`, `make_item(` → `common::make_item(`, `empty_backlog()` → `common::empty_backlog()`. Create a local wrapper for `make_in_progress_item` that calls `common::make_in_progress_item` + sets `phase_pool = Some(PhasePool::Main)` (coordinator_test has ~11 call sites — wrapper is clearly needed). Refactor `make_blocked_item` to call `common::make_item(id, ItemStatus::Blocked)` + set blocked fields. Verify refactored `make_blocked_item` produces same field defaults (especially `pipeline_type: None`). Remove extracted local definitions (locate by function name), keeping `backlog_path`, `save_and_commit_backlog`, refactored `make_blocked_item`, and the new `make_in_progress_item` wrapper.
- [x] Run `cargo test --test coordinator_test` — all tests must pass
- [x] **executor_test.rs — step 1**: Add `mod common;` before first `use` statement. Create local `make_feature_item(id: &str, status: ItemStatus) -> BacklogItem` wrapper that calls `common::make_item(id, status)` + sets `pipeline_type = Some("feature".to_string())`.
- [x] **executor_test.rs — step 2**: Create local `make_in_progress_item` wrapper that calls `common::make_in_progress_item(id, phase)` + sets `phase_pool = Some(PhasePool::Main)` and `pipeline_type = Some("feature".to_string())`. Refactor `make_scoping_item` to call `make_feature_item` + set `PhasePool::Pre`.
- [x] **executor_test.rs — step 3**: Replace call sites — `make_item(` → `make_feature_item(` (or `common::make_item(` for the few calls not needing pipeline_type), `make_in_progress_item(` → local wrapper, `setup_test_env()` → `common::setup_test_env()`, `make_backlog(` → `common::make_backlog(`, `default_config()` → `common::default_config()`.
- [x] **executor_test.rs — step 4**: Remove extracted local definitions. Verify remaining local helpers are only: `make_feature_item`, local `make_in_progress_item` wrapper, `make_scoping_item`, `make_phase_result`, `default_guardrails`, `make_simple_pipeline`.
- [x] Run `cargo test --test executor_test` — all tests must pass
- [x] **preflight_test.rs**: Add `mod common;` before first `use` statement. Create a local `make_feature_item` wrapper (same pattern as executor_test — calls `common::make_item` + sets `pipeline_type`). Replace `make_item(` → `make_feature_item(` (or `common::make_item(` + inline mutation for calls not needing pipeline_type). Replace `make_backlog(` → `common::make_backlog(`. Keep local `feature_pipeline_no_workflows()` and `default_config()` (uses incompatible pipeline).
- [x] Run `cargo test --test preflight_test` — all tests must pass
- [x] **scheduler_test.rs**: Add `mod common;` before first `use` statement. This is a **partial migration** — only replace `setup_test_env()` → `common::setup_test_env()` and `make_backlog(` → `common::make_backlog(`. Check if `empty_backlog` is used — if so, replace with `common::empty_backlog()`. Remove extracted local definitions only. **Keep ALL local item builders** (`make_item`, `make_in_progress_item`, `make_scoping_item`, `make_ready_item`) because they use 3-param signatures and different timestamp `"2026-01-01T00:00:00+00:00"`. Keep `default_config`, `default_execution_config`, `default_pipelines`, `make_snapshot`, `run_params`, and all other scheduler-specific helpers.
- [x] Run `cargo test --test scheduler_test` — all tests must pass
- [x] **prompt_test.rs**: This file does NOT use `fixture_path` or `fixtures_dir`. Do NOT add `mod common;`. No migration needed. All local helpers stay as-is (domain-specific `make_item` with hardcoded InProgress semantics and `"2026-01-01T00:00:00Z"` timestamp format).
- [x] Run `cargo test` (full suite) — all 14 test files must pass with zero failures

**Verification:**

- [x] `cargo test` (full suite) passes with zero failures
- [x] No local definitions of `setup_test_env` remain in coordinator_test, executor_test, scheduler_test
- [x] No local definitions of `make_backlog` remain in executor_test, preflight_test, scheduler_test
- [x] executor_test and preflight_test each have a local `make_feature_item` wrapper
- [x] coordinator_test has a local `make_in_progress_item` wrapper (with `phase_pool`)
- [x] scheduler_test retains its local `make_item(id, title, status)` and `make_in_progress_item(id, title, phase)`
- [x] prompt_test is unchanged (no `mod common;` added)

**Commit:** `[WRK-013][P2] Clean: Migrate remaining test files to shared common module`

**Notes:**

- Migration strategy per file:
  - **coordinator_test**: Full migration of item builders + env. Local wrapper for `make_in_progress_item` (adds `phase_pool` + `pipeline_type`). `make_blocked_item` refactored to use `common::make_item`.
  - **executor_test**: Full migration with local `make_feature_item` wrapper (pipeline_type) and local `make_in_progress_item` wrapper (pipeline_type + phase_pool). `make_scoping_item` refactored to use `make_feature_item`.
  - **preflight_test**: Item builders + backlog migrate, config stays local (incompatible pipeline). Local `make_feature_item` wrapper.
  - **scheduler_test**: Minimal — only env setup + backlog constructors migrate. All item builders stay local (3-param signatures, different timestamp). Cleaned up now-unused imports (`std::fs`, `std::process::Command`, `BacklogFile`).
  - **prompt_test**: No migration. Does not use any common helpers. All local helpers stay as-is.
- executor_test and preflight_test both define `make_feature_item` wrappers that do the same thing (call `common::make_item` + set pipeline_type). This is 2 files / 5 lines each — below the 3-file extraction threshold, so acceptable as local duplication.
- Migration order followed: coordinator_test → executor_test → preflight_test → scheduler_test (matching SPEC recommendation).
- Full test suite: 390 passed, 1 ignored — identical to Phase 1 baseline. Zero compiler warnings.
- Code review passed with no issues.

**Followups:**

---

### Phase 3: Final verification and cleanup

> Run full test suite, audit for remaining duplicates, verify migration completeness

**Phase Status:** complete

**Complexity:** Low

**Goal:** Final verification of the complete migration — confirm no duplicate helper definitions remain in migrated files, all test counts match the pre-migration baseline, and unchanged files still compile.

**Files:**

- `tests/coordinator_test.rs` — modify — fix fully-qualified type references to use imports

**Tasks:**

- [x] Run `cargo test` (full suite) — all 14 test files must pass with zero failures
- [x] Compare total passing test count against the baseline recorded in Phase 1. Must be identical (no tests added or removed).
- [x] Audit: search for remaining local definitions of extracted helpers. Run `grep -n "^fn make_item\|^fn make_backlog\|^fn empty_backlog\|^fn setup_test_env\|^fn fixture_path\|^fn fixtures_dir\|^fn default_config\|^fn make_in_progress_item" tests/*.rs` and verify only permitted local definitions remain:
  - `make_item` in scheduler_test.rs and prompt_test.rs only
  - `make_in_progress_item` in scheduler_test.rs only
  - `default_config` in preflight_test.rs and scheduler_test.rs only
  - Local wrappers in executor_test.rs, preflight_test.rs, coordinator_test.rs are permitted (different names like `make_feature_item`)
- [x] Verify all migrated files have `mod common;` declaration: backlog_test, coordinator_test, executor_test, preflight_test, scheduler_test, agent_test, migration_test. Confirm prompt_test does NOT have `mod common;` (not needed).
- [x] Verify unchanged files still compile: `cargo test --test git_test --test config_test --test lock_test --test types_test --test worklog_test --test agent_integration_test --no-run`

**Verification:**

- [x] `cargo test` passes with zero failures across all 14 test files
- [x] Test count matches Phase 1 baseline (no regressions)
- [x] Duplicate audit shows only permitted local definitions
- [x] All migrated files (7) have `mod common;`; prompt_test does not
- [x] common/mod.rs has `///` doc comments on all 8 pub functions (written in Phase 1)

**Commit:** `[WRK-013][P3] Clean: Final verification of shared test helper migration`

**Notes:**

- Full test suite: 390 passed, 1 ignored — identical to Phase 1 baseline.
- Code review found coordinator_test.rs used fully-qualified `orchestrate::types::BacklogItem` and `orchestrate::types::BacklogFile` instead of importing them. Fixed by adding to the import list. Quick-win, High impact on consistency.
- Code review also flagged `#![allow(dead_code)]` in common/mod.rs as undocumented. Deferred — the attribute is standard Rust practice for shared test modules and a doc comment would be over-documenting.
- Pre-existing clippy warnings in src/executor.rs (too-many-arguments) and src/scheduler.rs (new-without-default) are unrelated to WRK-013 changes (no src/ files were modified).

**Followups:**

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met:
  - [x] `tests/common/mod.rs` exists with shared helper functions
  - [x] `make_item(id, status)` canonical signature extracted, used by 3+ test files
  - [x] `make_in_progress_item(id, phase)` extracted, used by 3+ test files
  - [x] `make_backlog` and `empty_backlog` extracted and used by 3+ test files
  - [x] All existing tests pass (`cargo test` succeeds with no regressions)
  - [x] No duplicate definitions remain for extracted helpers (verified by audit)
  - [x] `fixture_path(name)` available in shared module
  - [x] (Should Have) `default_config` extracted for executor_test (scheduler_test and preflight_test retain local versions due to different pipeline configurations)
  - [x] (Should Have) `setup_test_env` extracted from 3 identical implementations
- [x] Tests pass
- [x] No regressions introduced
- [x] Code reviewed

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | Complete | `[WRK-013][P1]` | Created common/mod.rs with 8 helpers, migrated backlog_test, agent_test, migration_test. 390 tests pass. |
| 2 | Complete | `[WRK-013][P2]` | Migrated coordinator_test, executor_test, preflight_test, scheduler_test. prompt_test unchanged. 390 tests pass, zero warnings. |
| 3 | Complete | `[WRK-013][P3]` | Final verification passed. Fixed coordinator_test.rs qualified type refs. 390 tests pass, all audit checks green. |

## Followups Summary

### Critical

### High

### Medium

- [ ] Extract phase result builders (`phase_complete_result`, `failed_result`, `blocked_result`) to common if usage grows to 3+ files — PRD Should Have, deferred because current usage is 2 files with incompatible signatures
- [ ] Extract `default_pipelines` to common if signatures become compatible across 3+ files — PRD Should Have, deferred because only 2 files define it
- [ ] Consider builder pattern for BacklogItem if struct grows past ~30 fields — current function-based helpers would become unwieldy
- [ ] Consider sub-module split (`common/builders.rs`, `common/env.rs`, `common/fixtures.rs`) if common module grows past ~20 helpers

### Low

- [ ] Add extended item builders (`make_scoping_item`, `make_ready_item`, `make_blocked_item`) to common if usage grows to 3+ files — PRD Nice to Have
- [ ] Standardize scheduler_test and prompt_test to use canonical timestamp if a future change aligns their timestamp needs — PRD Should Have (timestamp standardization), deferred because these files keep local item builders with different semantics

## Design Details

### Key Types

Types consumed by `tests/common/mod.rs` from the orchestrate crate:

```rust
// From orchestrate::types (src/types.rs)
pub struct BacklogItem { /* 22 fields */ }
pub struct BacklogFile { schema_version: u32, items: Vec<BacklogItem>, next_item_id: u32 }
pub enum ItemStatus { New, Scoping, Ready, InProgress, Done, Blocked }
pub enum PhasePool { Pre, Main }

// From orchestrate::config (src/config.rs)
pub struct OrchestrateConfig { project, guardrails, execution, pipelines: HashMap<String, PipelineConfig> }
pub fn default_feature_pipeline() -> PipelineConfig
```

### Architecture Details

The common module sits in the standard Rust integration test sharing position:

```
tests/
├── common/
│   └── mod.rs          ← NEW: shared helpers, imported via `mod common;`
├── backlog_test.rs     ← `mod common;` + `common::make_item(...)`
├── coordinator_test.rs ← `mod common;` + local wrappers for phase_pool
├── executor_test.rs    ← `mod common;` + local `make_feature_item` wrapper
├── preflight_test.rs   ← `mod common;` + keeps local `default_config`
├── scheduler_test.rs   ← `mod common;` (partial: env/backlog only)
├── agent_test.rs       ← `mod common;` (fixtures_dir only)
├── migration_test.rs   ← `mod common;` (fixtures_dir only)
├── prompt_test.rs      ← UNCHANGED (keeps local builders, no common helpers needed)
├── git_test.rs         ← UNCHANGED (keeps local setup_temp_repo)
├── config_test.rs      ← UNCHANGED
├── lock_test.rs        ← UNCHANGED
├── types_test.rs       ← UNCHANGED
├── worklog_test.rs     ← UNCHANGED
└── agent_integration_test.rs ← UNCHANGED
```

Each test file in `tests/` compiles as an independent binary crate. The `tests/common/` directory with `mod.rs` is not treated as a test crate by Cargo — it's only compiled when imported via `mod common;` from a test file.

### Design Rationale

**Minimal defaults over realistic defaults:** All canonical helpers use `None`/empty for optional fields. This is safer because callers explicitly opt into non-default values, making test intent visible at call sites. The alternative (setting `pipeline_type: Some("feature")` as default) would require some callers to undo defaults they didn't ask for.

**Local wrappers over parameterized shared functions:** Rather than adding optional parameters to canonical helpers (e.g., `make_item(id, status, pipeline_type: Option<String>)`), files use 1-line mutation or small wrapper functions. This keeps the common API minimal and stable.

**Partial migration for scheduler_test and prompt_test:** These files have fundamentally different item builder semantics (3-param signatures, different timestamps, hardcoded status). Forcing them into the canonical pattern would add noise without reducing duplication. They migrate only compatible helpers (backlog constructors, environment setup).

---

## Assumptions

Decisions made without human input (autonomous mode):

1. **Mode: medium** — Well-understood refactoring with clear scope but multiple files and file-specific strategies.
2. **3-phase structure** — Phase 1 creates common + simple migrations, Phase 2 handles complex files, Phase 3 verifies. This provides incremental verification.
3. **executor_test local wrapper strategy** — Creating `make_feature_item` wrapper rather than keeping executor_test's entire local `make_item`. Wrapper is 5 lines vs 26 lines of duplication.
4. **coordinator_test make_in_progress_item** — Uses a local wrapper (not inline mutation) because coordinator_test has ~11 call sites for `make_in_progress_item`, well above the 3+ threshold.
5. **prompt_test no migration** — prompt_test does not use `fixture_path` or `fixtures_dir`. No `mod common;` added. All local helpers stay as-is.
6. **scheduler_test partial migration** — Only `setup_test_env` and `make_backlog`/`empty_backlog` migrate. All item builders stay local.
7. **No timestamp audit needed for Phase 1 files** — backlog_test, agent_test, and migration_test all use the canonical timestamp `"2026-02-10T00:00:00+00:00"` and no assertions compare timestamp strings directly. Phase 2 files that keep local item builders (scheduler_test, prompt_test) are not affected by timestamp standardization.
8. **preflight_test keeps local default_config** — Its config uses `feature_pipeline_no_workflows()`, incompatible with the common version that uses `default_feature_pipeline()`.
9. **`fixtures_dir()` added as implementation detail** — Not explicitly in PRD (which lists `fixture_path` only), but required because `agent_test.rs` and `migration_test.rs` use `fixtures_dir()` directly, not `fixture_path()`. The two helpers are complementary: `fixture_path(name)` = `fixtures_dir().join(name)`.
10. **Doc comments written in Phase 1** — Writing `///` doc comments when helpers are first created (Phase 1) rather than retroactively in Phase 3. This is cleaner and ensures documentation is never missing.

---

## Self-Critique Summary

The initial SPEC was reviewed by 6 internal critique agents. Key improvements applied:

**Auto-fixes applied (13):**
1. Added pre-migration baseline `cargo test` as first Phase 1 task (Critical finding — ensures green baseline)
2. Added full `cargo test` at end of Phase 2 (catches integration issues between Phase 1 and Phase 2 migrations)
3. Fixed Final Verification: `default_config` line now correctly states "for executor_test" only (scheduler_test and preflight_test retain local versions)
4. Broke executor_test.rs migration into 4 discrete sub-steps (was a single 15-line task — too complex to verify)
5. Resolved coordinator_test conditional: ~11 call sites for `make_in_progress_item` — local wrapper is clearly needed, stated decisively
6. Resolved prompt_test: does NOT use `fixture_path` or `fixtures_dir` — no migration needed, no `mod common;` added
7. Changed "critical migration order" to "recommended migration order" in Phase 2 notes (no technical dependency between file migrations)
8. Clarified `mod common;` placement: "before first `use` statement" (not "first line" which is ambiguous if file has attributes)
9. Changed line number references to function name anchors (line numbers are brittle across commits)
10. Moved doc comments from Phase 3 to Phase 1 (write documentation when creating helpers, not retroactively)
11. Added Phase 1 verification grep command for migrated files (matching Phase 3's audit rigor)
12. Added rollback note (git revert — straightforward for pure test refactoring)
13. Added deferred PRD Should Have items to Followups: phase result builders, `default_pipelines`, timestamp standardization

**Directional decisions (2, resolved autonomously):**
1. **`make_in_progress_item` default `phase_pool`**: Kept `phase_pool: None` (minimal default). Multiple critics recommended changing to `Some(PhasePool::Main)` (majority pattern). Decision: staying with `None` for consistency with the "minimal defaults" principle applied to all other helpers. The tech research initially recommended `Main`, but the Design self-critique already resolved this in favor of `None`. The tradeoff is that coordinator_test, executor_test need local wrappers — acceptable given each file's wrapper is 5 lines and makes the `phase_pool` assignment explicit at the call site.
2. **Phase 2 splitting**: Multiple critics suggested splitting Phase 2 into sub-phases. Decision: keeping as a single phase but with executor_test broken into 4 sub-steps. The file migrations are independent and an agent handles them sequentially within one phase. Splitting into separate phases would add commit overhead without improving safety (each file gets its own `cargo test` verification).

**Quality items acknowledged:**
- `fixtures_dir()` is an implementation detail beyond PRD scope — documented in Assumption 9
- executor_test + preflight_test have duplicate `make_feature_item` wrappers — 2 files / 5 lines each, below extraction threshold, documented in Phase 2 notes
- Phase 1 creates 8 helpers but only 5 are exercised by Phase 1 migrations — documented in Phase 1 notes with rationale

---

## Retrospective

### What worked well?

- Three-phase structure with incremental verification caught issues early and reduced risk.
- Creating all 8 helpers in Phase 1 (before consumers needed them) avoided modifying common/mod.rs in Phase 2, preventing unnecessary recompilation cascades.
- Local wrapper pattern (e.g., `make_feature_item`) kept the common API minimal while accommodating file-specific needs.
- Full test count comparison (390 passed, 1 ignored) at every phase boundary gave confidence that no tests were lost or broken.

### What was harder than expected?

- Nothing significant. The refactoring was well-scoped and each phase completed without surprises.

### What would we do differently next time?

- The Phase 2 agent left fully-qualified type references in coordinator_test.rs (`orchestrate::types::BacklogItem`). Phase 3 verification caught this. A more explicit SPEC task like "verify no fully-qualified type references remain when the type is already importable" would have prevented it.
