# SPEC: Add Phase Name Validation During V1-to-V2 Migration

**ID:** WRK-011
**Status:** Ready
**Created:** 2026-02-12
**PRD:** ./WRK-011_add-phase-name-validation-during-v1-to-v2-migration_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

The orchestrator's v1-to-v2 migration converts V1 `WorkflowPhase` enum variants to string names but doesn't validate those strings against the target pipeline configuration. If a user customizes their pipeline (e.g., renames or removes phases), migrated items can carry phase names that don't match any configured phase, causing silent scheduling failures downstream. This change adds defense-in-depth validation at migration time: invalid phase names are logged and cleared, letting the scheduler reassign them automatically.

## Approach

Extend `migrate_v1_to_v2` in `migration.rs` to accept a `&PipelineConfig` parameter. Before the item loop, build a `HashSet<&str>` from the pipeline's `pre_phases` and `phases` names. After each item is mapped via `map_v1_item()`, check if the item's phase (if present) exists in the valid set. If not, log a warning and clear both `phase` and `phase_pool` to `None`. Track these clears with a separate `validation_clears` counter in the migration summary.

The `backlog::load()` function gains a `project_root: &Path` parameter so it can load config via `load_config(project_root)`, extract the `"feature"` pipeline, and pass it to the migration function. All call sites in `main.rs` already have the project root available.

**Patterns to follow:**

- `.claude/skills/changes/orchestrator/src/config.rs:147-175` — `HashSet<&String>` validation pattern for phase name uniqueness checking in `validate()`
- `.claude/skills/changes/orchestrator/src/migration.rs:206-262` — Existing migration loop with per-item counters (`status_changes`, `phase_clears`, `unchanged`) and logging pattern
- `.claude/skills/changes/orchestrator/src/migration.rs:134-138` — Existing Researching-status phase clearing pattern (clearing `phase` to `None`)

**Implementation boundaries:**

- Do not modify: `config.rs`, `types.rs`, `scheduler.rs`, `executor.rs`
- Do not refactor: `map_v1_item()` internals — validation happens in the migration loop, not inside the mapping function
- Do not implement: edit distance suggestions (deferred per PRD/design)

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Core validation and signature changes | Med | Add validation logic to `migrate_v1_to_v2`, update `backlog::load()` signature, update all call sites and tests |

**Ordering rationale:** This is a single-phase change — all signature changes, validation logic, call site updates, and tests must be done atomically since the codebase cannot compile in any intermediate state.

---

## Phases

### Phase 1: Core validation and signature changes

> Add phase validation to migration, update function signatures, update all call sites and tests

**Phase Status:** complete

**Complexity:** Med

**Goal:** Add phase name validation during v1-to-v2 migration so invalid phase names are detected and cleared at migration time, with all existing tests passing and new test coverage for the validation path.

**Files:**

- `.claude/skills/changes/orchestrator/src/migration.rs` — modify — Add `&PipelineConfig` parameter, `HashSet` construction, validation logic after `map_v1_item()`, `validation_clears` counter
- `.claude/skills/changes/orchestrator/src/backlog.rs` — modify — Add `project_root: &Path` parameter to `load()`, load config, extract feature pipeline, pass to migration
- `.claude/skills/changes/orchestrator/src/main.rs` — modify — Update 6 `backlog::load()` call sites to pass `root`
- `.claude/skills/changes/orchestrator/tests/migration_test.rs` — modify — Update 11 existing `migrate_v1_to_v2` calls to pass `&default_feature_pipeline()`, add new validation tests
- `.claude/skills/changes/orchestrator/tests/backlog_test.rs` — modify — Update 14 `backlog::load()` calls to pass project root
- `.claude/skills/changes/orchestrator/tests/coordinator_test.rs` — modify — Update 6 `backlog::load()` calls to pass project root

**Patterns:**

- Follow `config.rs:147-175` for `HashSet` construction from phase names
- Follow `migration.rs:206-262` for counter pattern and summary logging
- Follow existing test patterns in `migration_test.rs` for inline YAML fixture construction

**Tasks:**

- [x] In `migration.rs`: add `use std::collections::HashSet;` and `use crate::config::PipelineConfig;` imports
- [x] In `migration.rs`: change `migrate_v1_to_v2` signature from `(path: &Path)` to `(path: &Path, pipeline: &PipelineConfig)`
- [x] In `migration.rs`: before the item loop, build `valid_phases: HashSet<&str>` from `pipeline.pre_phases.iter().chain(pipeline.phases.iter()).map(|p| p.name.as_str())`
- [x] In `migration.rs`: add `validation_clears: usize = 0` counter alongside existing counters
- [x] In `migration.rs`: change `let v2_item = map_v1_item(v1_item);` to `let mut v2_item = map_v1_item(v1_item);` to allow in-place mutation for validation clearing
- [x] In `migration.rs`: after existing per-item logging (after the `phase_cleared` tracking and before `items.push()`), add validation: if `v2_item.phase` is `Some(ref name)` and `!valid_phases.contains(name.as_str())`, then `log_warn!("  {}: phase '{}' not found in feature pipeline phases; cleared", v1_item.id, name)`; set `v2_item.phase = None` and `v2_item.phase_pool = None`; increment `validation_clears`. This placement ensures validation runs after status-based clearing (Researching items already have `phase: None` from `map_v1_item()`, so they are naturally skipped).
- [x] In `migration.rs`: update summary `log_info!` to include `validation_clears` in the format string and arguments
- [x] In `backlog.rs`: add `use crate::config::load_config;` import
- [x] In `backlog.rs`: change `load()` signature from `(path: &Path)` to `(path: &Path, project_root: &Path)`
- [x] In `backlog.rs`: in the v1 migration branch (before calling `migrate_v1_to_v2`), add: load config via `load_config(project_root)?`, extract feature pipeline via `config.pipelines.get("feature").ok_or_else(|| "Migration requires 'feature' pipeline in config, but none found".to_string())?`, pass `&pipeline_config` to `migrate_v1_to_v2`
- [x] In `main.rs`: update all 6 `backlog::load(&backlog_file_path)` calls to `backlog::load(&backlog_file_path, root)` in `handle_run`, `handle_triage`, `handle_add`, `handle_status`, `handle_advance`, `handle_unblock`
- [x] In `migration_test.rs`: add `use orchestrate::config::default_feature_pipeline;` import
- [x] In `migration_test.rs`: update all 11 existing `migrate_v1_to_v2(&target)` calls to `migrate_v1_to_v2(&target, &default_feature_pipeline())`
- [x] In `migration_test.rs`: add new test `migrate_v1_invalid_phase_cleared_by_validation` — construct a custom `PipelineConfig` that excludes `"prd"` from its phases (keep other phases intact), create a V1 YAML with an item having `phase: prd` and `status: in_progress`, call `migrate_v1_to_v2(&target, &custom_pipeline)`, assert `item.phase == None` AND `item.phase_pool == None` (both must be cleared)
- [x] In `migration_test.rs`: add new test `migrate_v1_none_phase_skips_validation` — create a V1 YAML with an item having no phase set and a custom pipeline, call `migrate_v1_to_v2`, assert the item passes through without warnings (phase remains `None`)
- [x] In `backlog_test.rs`: update all 14 `backlog::load(...)` calls to include a project root parameter — for fixture-based tests use `fixture_path("...").parent().unwrap()`, for temp dir tests use `dir.path()` or `path.parent().unwrap()`. Since these all load v2 files, the `project_root` is not exercised by migration (v2 files skip migration early).
- [x] In `coordinator_test.rs`: update all 6 `backlog::load(...)` calls to include `dir.path()` as project root (these all load v2 files, so project_root is not exercised)
- [x] Verify all existing tests pass: `cargo test` in the orchestrator directory
- [x] Verify `cargo clippy` passes with no new warnings

**Verification:**

- [x] `cargo test` passes — all existing tests pass, no regressions
- [x] `cargo clippy` passes with no new warnings
- [x] New test `migrate_v1_invalid_phase_cleared_by_validation` passes — confirms validation clears both `phase` and `phase_pool` for invalid phases
- [x] New test `migrate_v1_none_phase_skips_validation` passes — confirms items with no phase skip validation
- [x] Existing `migrate_v1_full_fixture` test still passes — confirms valid phases (all default V1 phases exist in default pipeline) are not cleared
- [x] Existing `migrate_v1_empty_backlog` test still passes — confirms empty backlog migration works with new signature
- [x] Code review passes (`/code-review` -> fix issues -> repeat until pass)

**Commit:** `[WRK-011][P1] Feature: Add phase name validation during v1-to-v2 migration`

**Notes:** Code review identified that the `unchanged` counter didn't account for validation clears — fixed by adding a per-item `validation_cleared` flag to the unchanged condition.

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
| P1 | complete | f101a1a | All tasks done, tests pass, code review passed |

## Followups Summary

### Critical

### High

### Medium

### Low

- [x] [Low] Edit distance suggestion in validation warnings — deferred per PRD "Nice to Have" and design decision; adds complexity for marginal UX benefit

## Design Details

### Key Types

No new types are introduced. Existing types used:

```rust
// From config.rs — accepted as parameter
pub struct PipelineConfig {
    pub pre_phases: Vec<PhaseConfig>,
    pub phases: Vec<PhaseConfig>,
}

pub struct PhaseConfig {
    pub name: String,
    // ...other fields not relevant to validation
}
```

### Architecture Details

The validation is a simple set-membership check inserted into the existing migration loop:

```
backlog::load(path, project_root)
  └── if v1: load_config(project_root) → extract "feature" pipeline
        └── migrate_v1_to_v2(path, &pipeline)
              └── Build HashSet<&str> from pre_phases + phases names
              └── For each item after map_v1_item():
                    if phase ∈ valid_phases → keep
                    if phase ∉ valid_phases → warn + clear + count
```

### Design Rationale

- **`&PipelineConfig` over `HashSet<String>`:** Same-crate type provides clear API semantics, self-documenting at call sites, allows migration to build its own HashSet internally. See Design doc for full rationale.
- **`project_root` parameter over path derivation:** Explicit over implicit; caller already has root; avoids fragile directory traversal.
- **Validate in loop, not in `map_v1_item()`:** Keeps mapping function focused on data conversion; validation has access to counters, logging, and config.
- **Warn-and-clear over error:** Self-correcting — scheduler assigns correct first phase to items without a phase. No manual intervention needed.
- **Separate `validation_clears` counter:** Distinguishes from status-based `phase_clears`; mutually exclusive (no double-counting).

---

## Assumptions

Decisions made autonomously (no human available for input):

- **Single-phase structure:** Chose a single atomic phase rather than splitting into multiple phases. The codebase cannot compile in any intermediate state (signature changes break all call sites), so splitting would be artificial and confusing.
- **Test scope:** Added two new tests (invalid phase cleared, none phase skips validation) rather than exhaustive edge case testing (case sensitivity, mass clearing, scheduler integration). The two tests cover the critical paths; additional edge cases are deferred as followups since the feature is defense-in-depth and unlikely to fire in practice.
- **No integration test through `backlog::load()`:** The new test validates at the `migrate_v1_to_v2` level directly, not through `backlog::load()`. This is sufficient because `backlog::load()` is a thin wrapper that only adds config loading, and the config loading path is already tested by `config_test.rs`.
- **Project root for v2 test files:** For `backlog_test.rs` and `coordinator_test.rs`, using `.parent().unwrap()` or `dir.path()` as `project_root` is safe because v2 files skip migration entirely (early return before config is loaded).
- **Dismissed self-critique items:** Dismissed mass-clearing safeguard (over-engineering for defense-in-depth feature), backup mechanism (atomic write already provides safety), and scheduler integration test (out of scope per PRD).

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
