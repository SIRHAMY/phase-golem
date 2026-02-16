# Change: Add PhaseConfig helper constructor to reduce verbosity

**Status:** Proposed
**Created:** 2026-02-13
**Author:** Autonomous Agent

## Problem Statement

`PhaseConfig` construction is verbose throughout the codebase. Every construction site must specify all 4 fields (`name`, `workflows`, `is_destructive`, `staleness`), even though `staleness` is almost always `StalenessAction::Ignore` and `workflows` is frequently an empty vec or a single path. This leads to:

1. **Production verbosity** — `default_feature_pipeline()` in `config.rs` is 47 lines of struct literals for 7 phases. Each phase requires 5 lines to specify all fields explicitly, when only `name`, `is_destructive`, and optionally `workflows` vary.
2. **Test verbosity** — 30+ inline `PhaseConfig { ... }` constructions across 6 test files (`config_test.rs`, `executor_test.rs`, `migration_test.rs`, `preflight_test.rs`, `prompt_test.rs`, `scheduler_test.rs`), most with identical default field values.
3. **Maintenance friction** — Adding a new field to `PhaseConfig` requires updating every construction site. The `staleness` field addition (from WRK-003) already demonstrated this cost.

This was identified as a low-priority followup in the WRK-003 (Pipeline Engine V2) spec: "`default_feature_pipeline()` is verbose — consider a helper constructor if more default pipelines are added."

## User Stories / Personas

- **Orchestrator developer** — Wants to add or modify phases in `default_feature_pipeline()` without wading through field-by-field struct literals. Wants new `PhaseConfig` fields to have defaults that don't require touching every construction site.

## Desired Outcome

`PhaseConfig` has a `new(name, is_destructive)` constructor that defaults `workflows` to `vec![]` and `staleness` to `StalenessAction::Ignore`. Construction sites that need non-default values use struct update syntax to override specific fields:

```rust
// Before (5 lines):
PhaseConfig {
    name: "build".to_string(),
    workflows: vec!["path/to/workflow.md".to_string()],
    is_destructive: true,
    staleness: StalenessAction::Ignore,
}

// After — default workflows (1 line):
PhaseConfig::new("build", true)

// After — with workflows (4 lines, but workflows is the only override):
PhaseConfig {
    workflows: vec!["path/to/workflow.md".to_string()],
    ..PhaseConfig::new("build", true)
}
```

After this change:
- `default_feature_pipeline()` reads as a concise list of phase names and their properties
- Test code creates `PhaseConfig` values in 1 line instead of 5 (when using default workflows and staleness)
- Adding new fields with sensible defaults to `PhaseConfig` doesn't require updating construction sites that don't care about the new field

## Success Criteria

### Must Have

- [ ] `PhaseConfig` has a `pub fn new(name: &str, is_destructive: bool) -> PhaseConfig` constructor that defaults `workflows` to `vec![]` and `staleness` to `StalenessAction::Ignore`
- [ ] `default_feature_pipeline()` uses the new constructor with struct update syntax for workflow overrides
- [ ] All existing tests pass without behavioral changes
- [ ] Constructor defaults match the existing `#[serde(default)]` field defaults (empty vec for `workflows`, `Ignore` for `staleness`)
- [ ] No new `pub` API surfaces beyond the constructor itself

### Should Have

- [ ] Test files that construct `PhaseConfig` inline are updated to use the new constructor — this covers all 30+ construction sites across `config_test.rs`, `executor_test.rs`, `migration_test.rs`, `preflight_test.rs`, `prompt_test.rs`, and `scheduler_test.rs`; sites with non-default staleness values use struct update syntax to override
- [ ] The `make_phase_config` helper in `prompt_test.rs` is removed and its call sites replaced with `PhaseConfig::new(...)` plus struct update syntax for workflow overrides

## Scope

### In Scope

- `PhaseConfig::new()` constructor method on the struct (in `config.rs`)
- Updating `default_feature_pipeline()` to use the constructor
- Updating inline `PhaseConfig` construction sites in test files

### Out of Scope

- Changing `PipelineConfig` construction patterns (separate concern)
- Adding builder pattern or method chaining (`.with_workflows()`, etc.) for `PhaseConfig`
- Adding `Default` impl for `PhaseConfig` (`name` has no meaningful default)
- Extracting test helpers to `tests/common/mod.rs` (that's WRK-013)
- Changing any runtime behavior or config file format

## Constraints

- Must remain compatible with `serde::Deserialize` — the constructor is for programmatic construction only; TOML deserialization is unaffected
- Constructor defaults must align with `#[serde(default)]` field-level defaults to avoid inconsistency between programmatic and deserialized configs

## Dependencies

- **Depends On:** None
- **Blocks:** Nothing directly, but simplifies future work adding phases or `PhaseConfig` fields

## Risks

- [ ] If `PhaseConfig` gains fields where the correct default isn't obvious, the constructor could hide important configuration decisions — mitigated by keeping the constructor minimal (only defaulting fields that are truly boilerplate: `workflows` and `staleness`)

## Assumptions

- The `staleness` field's default of `StalenessAction::Ignore` is correct for the constructor since every current production construction site uses `Ignore`. The 4 test cases that use non-default staleness values (`executor_test.rs` lines 376, 426, 489, 542) will use struct update syntax to override.
- An empty `workflows` vec is a safe default since workflows are only required for actual execution, not for config validation or phase resolution.
- Struct update syntax is the preferred pattern for overriding defaults, not method chaining — this avoids additional API surface and is idiomatic Rust.

## References

- WRK-003 spec followup: "default_feature_pipeline() is verbose — consider a helper constructor"
- `orchestrator/src/config.rs:98-145` — current `default_feature_pipeline()` implementation
- `orchestrator/tests/prompt_test.rs:12-18` — existing `make_phase_config` test helper
