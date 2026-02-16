# SPEC: Add PhaseConfig helper constructor to reduce verbosity

**ID:** WRK-008
**Status:** Ready
**Created:** 2026-02-13
**PRD:** ./WRK-008_add-helper-constructor-for-default-feature-pipeline-to-reduce-verbosity_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

`PhaseConfig` construction is verbose throughout the codebase. Every construction site must specify all 4 fields (`name`, `workflows`, `is_destructive`, `staleness`), even though `staleness` is almost always `StalenessAction::Ignore` and `workflows` is frequently empty. The `default_feature_pipeline()` function is 47 lines of struct literals for 7 phases, and tests have 40+ inline constructions across 6 files.

This was identified as a low-priority followup in the WRK-003 (Pipeline Engine V2) spec.

## Approach

Add a `PhaseConfig::new(name: &str, is_destructive: bool)` constructor that defaults `workflows` to `vec![]` and `staleness` to `StalenessAction::Ignore`. Then update all construction sites to use it, with struct update syntax (`..PhaseConfig::new(...)`) for field overrides.

This is the textbook idiomatic Rust pattern for structs with required + defaultable fields. No new types, modules, or external dependencies.

**Patterns to follow:**

- `.claude/skills/changes/orchestrator/src/config.rs:68-96` — existing `Default` trait impls for `ProjectConfig`, `GuardrailsConfig`, `ExecutionConfig` demonstrate the convention of placing impl blocks immediately after struct definitions

**Implementation boundaries:**

- Do not modify: `PipelineConfig` construction patterns, `StalenessAction` enum, serde derives/attributes on `PhaseConfig`
- Do not add: Builder pattern methods, `Default` impl for `PhaseConfig`, test helper modules (that's WRK-013)

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Constructor & Production Update | Low | Add `PhaseConfig::new()` constructor, update `default_feature_pipeline()`, add constructor tests |
| 2 | Test Migration | Low | Update all test construction sites to use the new constructor; remove `make_phase_config` helper |

**Ordering rationale:** Phase 1 introduces the constructor that Phase 2 depends on. Production code and test code are separated into distinct phases so each leaves the codebase in a passing state.

---

## Phases

Each phase should leave the codebase in a functional, stable state. Complete and verify each phase before moving to the next.

---

### Phase 1: Constructor & Production Update

> Add `PhaseConfig::new()` constructor and update `default_feature_pipeline()` to use it

**Phase Status:** complete

**Complexity:** Low

**Goal:** Introduce the `PhaseConfig::new()` constructor and update the production construction sites in `default_feature_pipeline()`.

**Files:**

- `.claude/skills/changes/orchestrator/src/config.rs` — modify — Add `impl PhaseConfig` block with `new()` constructor after line 59; update `default_feature_pipeline()` to use constructor with struct update syntax
- `.claude/skills/changes/orchestrator/tests/config_test.rs` — modify — Add constructor unit tests (correctness and serde alignment)

**Patterns:**

- Follow `config.rs:68-96` for placement convention (impl blocks after struct definitions)
- Constructor doc comment must explicitly state that defaults match `#[serde(default)]` attributes

**Tasks:**

- [x] Add `impl PhaseConfig` block immediately after the `PhaseConfig` struct definition (after line 59, before `PipelineConfig` struct on line 61). Constructor signature: `pub fn new(name: &str, is_destructive: bool) -> Self`. Body: sets `name: name.to_string()`, `workflows: vec![]`, `is_destructive`, `staleness: StalenessAction::Ignore`. Include doc comment noting serde alignment requirement.
- [x] Update `default_feature_pipeline()` to use `PhaseConfig::new()` with struct update syntax for workflow overrides. All 7 phases (1 pre-phase + 6 main) have non-empty workflows, so all use `PhaseConfig { workflows: vec![...], ..PhaseConfig::new("name", is_destructive) }`. The `build` phase is the only one with `is_destructive: true`.
- [x] Add a unit test in `config_test.rs` verifying the constructor produces correct values: `PhaseConfig::new("test", false)` has `name == "test"`, `is_destructive == false`, `workflows` is empty, `staleness == StalenessAction::Ignore`. Also test with `is_destructive: true`. Also verify serde alignment: deserialize a TOML phase with only `name` and `is_destructive` set, and assert the result equals `PhaseConfig::new()` output.

**Verification:**

- [x] `cargo build -p orchestrate` succeeds
- [x] `cargo test -p orchestrate` succeeds (all existing tests still pass, plus new constructor tests)
- [x] Constructor defaults match `#[serde(default)]` values: `workflows` = empty vec, `staleness` = `StalenessAction::Ignore` (verified by unit test)
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-008][P1] Clean: Add PhaseConfig::new() constructor and update default_feature_pipeline()`

**Notes:**

**Followups:**

---

### Phase 2: Test Migration

> Update all test construction sites to use the new constructor; remove `make_phase_config` helper

**Phase Status:** complete

**Complexity:** Low

**Goal:** Update all ~40 inline `PhaseConfig { ... }` construction sites in test files to use `PhaseConfig::new()`, and remove the now-redundant `make_phase_config` helper from `prompt_test.rs`.

**Files:**

- `.claude/skills/changes/orchestrator/tests/config_test.rs` — modify — Update 10 `PhaseConfig { ... }` construction sites. 8 use default staleness; 2 use `StalenessAction::Block` and need struct update syntax.
- `.claude/skills/changes/orchestrator/tests/executor_test.rs` — modify — Update 10 construction sites including `make_simple_pipeline()` helper. 4 sites use non-default staleness (`Block` or `Warn`) and use struct update syntax; the rest become 1-liners.
- `.claude/skills/changes/orchestrator/tests/migration_test.rs` — modify — Update 5 construction sites. All use default staleness.
- `.claude/skills/changes/orchestrator/tests/preflight_test.rs` — modify — Update 14 construction sites including the `feature_pipeline_no_workflows()` helper function. All use default staleness.
- `.claude/skills/changes/orchestrator/tests/prompt_test.rs` — modify — Remove `make_phase_config` helper (lines 12-19); update `default_prd_config()` to use `PhaseConfig::new()` with struct update syntax for workflow override; update all direct `make_phase_config(...)` call sites to use constructor with struct update syntax.
- `.claude/skills/changes/orchestrator/tests/scheduler_test.rs` — modify — Update 2 construction sites to use `PhaseConfig::new()`.

**Tasks:**

- [x] Update `config_test.rs`: Replace each `PhaseConfig { name: "...".to_string(), workflows: vec![], is_destructive: ..., staleness: StalenessAction::Ignore }` with `PhaseConfig::new("...", is_destructive)`. For the site with `StalenessAction::Block`, use `PhaseConfig { staleness: StalenessAction::Block, ..PhaseConfig::new("...", is_destructive) }`.
- [x] Update `executor_test.rs`: Replace default-staleness sites with `PhaseConfig::new(...)`. For the 4 non-default staleness sites, use struct update syntax to override staleness. Update `make_simple_pipeline()` helper similarly.
- [x] Update `migration_test.rs`: Replace all 5 `PhaseConfig { ... }` constructions with `PhaseConfig::new(...)`.
- [x] Update `preflight_test.rs`: Replace all 14 constructions including `feature_pipeline_no_workflows()` helper. All use default staleness and empty workflows, so all become `PhaseConfig::new("name", is_destructive)`.
- [x] Update `prompt_test.rs`: Remove `make_phase_config` helper (lines 12-19). Update `default_prd_config()` to use `PhaseConfig { workflows: vec![...], ..PhaseConfig::new("prd", false) }`. Replace all former `make_phase_config(...)` call sites with `PhaseConfig { workflows: vec![...], ..PhaseConfig::new("name", false) }` or `PhaseConfig::new("name", false)` when workflows are not needed. Update the inline `PhaseConfig { ... }` in `triage_prompt_with_multiple_pipelines_lists_all` test.
- [x] Update `scheduler_test.rs`: Replace 2 `PhaseConfig { ... }` constructions with `PhaseConfig::new(...)`.

**Verification:**

- [x] `cargo test -p orchestrate` succeeds (all tests pass, no behavioral changes)
- [x] `grep -r "make_phase_config" orchestrator/tests/` returns no results (helper fully removed)
- [x] `feature_pipeline_no_workflows()` in `preflight_test.rs` and `make_simple_pipeline()` in `executor_test.rs` updated to use constructor
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-008][P2] Clean: Migrate test PhaseConfig constructions to use new constructor`

**Notes:**

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
| 1 | complete | `[WRK-008][P1] Clean: Add PhaseConfig::new() constructor and update default_feature_pipeline()` | All tests pass, code review clean |
| 2 | complete | `[WRK-008][P2] Clean: Migrate test PhaseConfig constructions to use new constructor` | All 449 tests pass, make_phase_config removed, unused StalenessAction imports cleaned up |

## Followups Summary

### Critical

### High

### Medium

### Low

## Design Details

### Key Types

```rust
impl PhaseConfig {
    /// Construct a PhaseConfig with sensible defaults for workflows and staleness.
    ///
    /// Defaults: `workflows` = `vec![]`, `staleness` = `StalenessAction::Ignore`.
    /// These MUST match the `#[serde(default)]` field attributes on the struct
    /// to keep programmatic and deserialized configs consistent.
    pub fn new(name: &str, is_destructive: bool) -> Self {
        Self {
            name: name.to_string(),
            workflows: vec![],
            is_destructive,
            staleness: StalenessAction::Ignore,
        }
    }
}
```

### Construction Patterns

**Default workflows and staleness (most common — ~34 of ~45 sites):**
```rust
PhaseConfig::new("build", true)
```

**Custom workflows, default staleness (all 7 production phases, ~6 test sites):**
```rust
PhaseConfig {
    workflows: vec!["path/to/workflow.md".to_string()],
    ..PhaseConfig::new("build", true)
}
```

**Custom staleness, default workflows (4 test sites in executor_test.rs):**
```rust
PhaseConfig {
    staleness: StalenessAction::Block,
    ..PhaseConfig::new("build", true)
}
```

### Design Rationale

Constructor + struct update syntax is the idiomatic Rust pattern for structs with required fields plus defaultable fields. It provides minimal API surface (one `pub fn`), works with Rust's ownership model, and requires no additional crate dependencies. The builder pattern was considered and rejected as over-engineered for a 4-field struct.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
