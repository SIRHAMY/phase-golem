# Change: Add Phase Name Validation During V1-to-V2 Migration

**Status:** Proposed
**Created:** 2026-02-12
**Author:** Orchestrator (autonomous)

## Problem Statement

The orchestrator's BACKLOG.yaml has two schema versions: version 1 (v1) uses a fixed `WorkflowPhase` enum (`prd`, `research`, `design`, `spec`, `build`, `review`), while version 2 (v2) uses arbitrary string phase names defined in the pipeline configuration. The pipeline configuration groups phases into two categories: **pre-phases** (executed before the main sequence, e.g., `"research"`) and **main phases** (executed in order, e.g., `"prd"`, `"tech-research"`, `"design"`, `"spec"`, `"build"`, `"review"`). Each item also has a `pipeline_type` field (e.g., `"feature"`) that determines which pipeline's phases apply to it.

When migrating a v1 BACKLOG.yaml to v2, the migration code (`migration.rs`) converts the V1 `WorkflowPhase` enum variants to their string equivalents (e.g., `V1WorkflowPhase::Prd` becomes `"prd"`). However, the migration does not validate that the resulting phase name strings actually exist in the target pipeline configuration.

The V1 enum variants (prd, research, design, spec, build, review) currently match the default v2 feature pipeline phase names exactly, but this alignment is coincidental rather than enforced — v2 allows arbitrary phase names. If a user customizes their pipeline phases (e.g., renames `"prd"` to `"planning"` or removes a phase), migrated items could carry phase names that don't match any configured phase. Items with unrecognized phase names cause downstream issues: the scheduler's `build_run_phase_action` returns no action for the item (effectively skipping it), and if the item does reach execution, it fails with `"Phase '...' not found in pipeline"`. In either case, the root cause (a phase name mismatch introduced during migration) is not surfaced at the point where it could be most easily understood and fixed.

This is a defense-in-depth improvement: validate phase names at migration time so invalid mappings are detected early rather than causing confusing scheduling or execution failures downstream.

## User Stories / Personas

- **Orchestrator operator** — Wants confidence that after migration, all items have valid phase assignments that will be recognized by the scheduler. Wants clear warnings about any phase name mismatches rather than items silently getting stuck.

## Desired Outcome

During v1-to-v2 migration, each migrated item's phase name (if present) is validated against the configured pipeline phases (both pre-phases and main phases) for the item's `pipeline_type` (the pipeline configuration loaded via `load_config`, which reads from `orchestrate.toml` or falls back to the default feature pipeline when no config file exists). If a phase name doesn't match any phase in the item's pipeline, the migration emits a warning log and clears the phase. The scheduler automatically assigns the appropriate first phase to items with no phase set when they are next scheduled, so clearing an invalid phase allows the system to self-correct rather than leaving the item stuck.

After migration, every item either carries a valid phase name recognized by the pipeline or has no phase assigned.

Items with no phase set (`None`) are unaffected by validation and pass through unchanged.

## Success Criteria

### Must Have

- [ ] During migration, each item's phase name is checked against the phases (both pre-phases and main phases) defined in the target pipeline configuration
- [ ] Items with phase names not found in the configured pipeline produce a `log_warn!` message identifying the item ID, the invalid phase name, and the pipeline type
- [ ] Items with invalid phase names have their phase cleared (set to `None`) so the scheduler can reassign them, rather than silently skipping them
- [ ] Phase name matching is case-sensitive, consistent with the scheduler's exact-match behavior (`p.name == phase_name`)
- [ ] Items with `phase: None` (no phase set) skip validation entirely — no warning, no clearing
- [ ] The existing special handling of `Researching` status items (which clears their phase regardless, because `Researching` maps to `Scoping` status which belongs to the pre-phases pool, making any v1 main-phase assignment invalid) takes precedence — validation only applies to items whose phase was not already cleared by status-based logic
- [ ] When a phase is cleared due to validation failure, `phase_pool` is also cleared to maintain consistency
- [ ] Existing tests continue to pass
- [ ] New test(s) cover the case where a v1 phase name doesn't match any v2 pipeline phase

### Should Have

- [ ] The migration summary log includes a separate counter for items with phase names cleared due to validation failure (distinct from the existing `phase_clears` counter which tracks status-based phase clearing)
- [ ] The validation uses the same pipeline config that the scheduler would use (loaded via `load_config`)

### Nice to Have

- [ ] Warning message suggests the closest matching phase name using edit distance (e.g., "phase 'research' not found in main phases; did you mean 'tech-research'?")

## Scope

### In Scope

- Phase name validation within `migrate_v1_to_v2` in `migration.rs`
- Passing pipeline configuration (or phase name sets) into the migration function
- Warning logs for invalid phase names
- Clearing invalid phase names
- Test coverage for the new validation path

### Out of Scope

- Validating phase names at backlog load/save time (separate concern, tracked elsewhere)
- Validating phase names in the scheduler or executor (already handled there)
- Changes to the V1 enum or V1 parsing logic
- Changes to the config validation logic in `config.rs`
- Phase name format validation (e.g., regex patterns for allowed characters) — phase names are assumed to be valid identifier strings from a trusted config
- Interactive prompts during migration (migration should remain non-interactive)
- Rollback mechanism for cleared phases — clearing is a safe operation since the scheduler assigns the appropriate first phase to items with no phase set when they are next scheduled

## Non-Functional Requirements

- **Performance:** Validation is a set-membership check (O(1) per item using a `HashSet`) during a one-time migration. No measurable impact expected for typical backlog sizes (< 1000 items).

## Constraints

- The `migrate_v1_to_v2` function currently takes only a `Path` parameter. Adding pipeline config awareness requires either passing the config as a parameter or loading it within the function. The approach should be determined during design/spec.
- Migration must remain idempotent — running on an already-v2 file must still be a no-op (skip validation entirely since v2 files are returned as-is).
- The V1 phase `"research"` is already handled specially (cleared for `Researching` status items). The new validation runs after this existing logic and only validates phases that survived the status-based clearing.

## Dependencies

- **Depends On:** None — the migration and config code are already in place
- **Blocks:** None

## Risks

- [ ] If the pipeline config cannot be loaded at migration time (e.g., corrupted `orchestrate.toml`), migration should fail with a clear error rather than proceeding with unvalidated phase names. Mitigation: the caller loads config via `load_config` (which falls back to defaults when no config file exists) and passes it to the migration function. Config loading errors propagate as migration errors.
- [ ] If a user's custom config defines pipelines but none named `"feature"`, all migrated items (which get `pipeline_type: "feature"`) would have no pipeline to validate against. Mitigation: `load_config` injects the default feature pipeline when no pipelines are configured, but if the user defines other pipelines without `"feature"`, this edge case should produce a clear error.

## Assumptions

- The migration function will be updated to accept pipeline configuration or phase name sets as a parameter, rather than loading config internally (keeps the function testable and avoids coupling migration to config loading).
- When a phase name is invalid, clearing it (setting to `None`) is preferred over blocking the item, since the scheduler already handles phase assignment for items without a phase.
- The `"research"` phase in V1 maps to the `"research"` pre-phase in V2's default pipeline. Since the existing code already clears the phase for `Researching` status items (which map to `Scoping`), validation of the `"research"` phase string is only relevant for non-Researching items that happen to have `phase: research` set (which shouldn't occur in well-formed v1 data — i.e., data where `Researching` status items have the `Research` phase — but could occur in manually edited or corrupted data).
- All migrated items receive `pipeline_type: "feature"` (v1 has no pipeline_type concept). Validation checks against the feature pipeline's phases.
- Validation checks phase names against the combined set of all phase names (pre-phases + main phases) for the item's pipeline, without regard to `phase_pool`. Pool-aware validation (checking pre_phases vs main phases separately based on `phase_pool`) is not needed because the migration already handles pool assignment correctly via status-based logic, and a phase name valid in either pool is acceptable.
- If pipeline config cannot be loaded, migration fails with an error rather than silently skipping validation. This is acceptable because `load_config` falls back to defaults when no config file exists, so failure only occurs with actively corrupted configuration.

## Open Questions

- [ ] Should the function signature change to accept a `&PipelineConfig` (the full pipeline configuration object) or a `HashSet<String>` (a pre-built set of valid phase name strings)?
- [ ] Should validation warnings be promoted to errors (returning `Err`) if any items have invalid phases, or should migration always succeed with warnings?
- [ ] For future multi-pipeline support: should validation look up the specific pipeline by the item's `pipeline_type`, or use a flat set of all phase names across all pipelines? (Currently moot since all migrated items are `"feature"` type.)

## References

- `orchestrator/src/migration.rs` — Current migration logic
- `orchestrator/src/config.rs` — Pipeline config, validation, and `default_feature_pipeline()`
- `orchestrator/src/scheduler.rs` — Phase lookup and scheduling (where invalid phases cause `build_run_phase_action` to return no action, and `run_scheduler` to return `PhaseExecutionResult::Failed`)
- `orchestrator/src/executor.rs` — Phase config lookup and transition resolution (where invalid phases cause items to be blocked)
