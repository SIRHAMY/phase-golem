# Tech Research: Add Phase Name Validation During V1-to-V2 Migration

**ID:** WRK-011
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-011_add-phase-name-validation-during-v1-to-v2-migration_PRD.md
**Mode:** Light

## Overview

Research how to add phase name validation to the v1-to-v2 migration in `migration.rs`. The migration currently converts V1 `WorkflowPhase` enum variants to string names but doesn't verify those strings exist in the target pipeline configuration. We need to understand the exact current migration flow, pipeline config structure, and integration points to add validation cleanly.

## Research Questions

- [x] What is the exact migration function signature and flow?
- [x] How is the pipeline config structured (pre_phases vs main phases)?
- [x] Where exactly should validation logic be inserted in the migration flow?
- [x] What patterns exist in the codebase for phase name resolution and validation?
- [x] How should the function signature change to accept pipeline config?

---

## External Research

Not applicable — this is a purely internal validation concern within the orchestrator's migration logic. No external patterns or libraries needed. The implementation involves straightforward `HashSet` membership checks, which is a well-understood pattern.

---

## Internal Research

### Existing Codebase State

The orchestrator implements a two-schema versioning system for BACKLOG.yaml:
- **V1 Schema**: Fixed `V1WorkflowPhase` enum with 6 variants (`Prd`, `Research`, `Design`, `Spec`, `Build`, `Review`), each converting to lowercase strings via `.as_str()`
- **V2 Schema**: Arbitrary string phase names defined in `PipelineConfig` via `orchestrate.toml`

The migration currently performs status mapping and Researching-status special handling, but does NOT validate that resulting phase name strings exist in the target pipeline configuration.

**Relevant files/modules:**

- `orchestrator/src/migration.rs` (304 lines) — Migration logic. Key function: `pub fn migrate_v1_to_v2(path: &Path) -> Result<BacklogFile, String>`. Converts V1 items to V2, handles Researching status clearing, writes atomically via `NamedTempFile`.
- `orchestrator/src/config.rs` (245 lines) — Pipeline config types. `PipelineConfig` has `pre_phases: Vec<PhaseConfig>` and `phases: Vec<PhaseConfig>`. `PhaseConfig` has `name: String`. `load_config(project_root: &Path)` reads `orchestrate.toml` or falls back to defaults. `default_feature_pipeline()` defines 1 pre_phase ("research") and 6 main phases ("prd", "tech-research", "design", "spec", "build", "review").
- `orchestrator/src/scheduler.rs` (1256 lines) — Phase lookup via `pipeline.pre_phases.iter().chain(pipeline.phases.iter()).find(|p| p.name == phase_name)`. Returns `None` for unrecognized phases, effectively skipping the item.
- `orchestrator/src/executor.rs` (518 lines) — Phase lookup via exact match; blocks execution with "Phase '...' not found in pipeline" if not found.
- `orchestrator/src/types.rs` (243 lines) — `BacklogItem` has `phase: Option<String>`, `phase_pool: Option<PhasePool>`, `pipeline_type: Option<String>`.
- `orchestrator/src/log.rs` (108 lines) — `log_warn!()`, `log_info!()`, `log_debug!()` macros using `eprintln!()`.
- `orchestrator/tests/migration_test.rs` (269 lines) — Tests using `tempfile::TempDir`, fixture files from `tests/fixtures/`.

**Existing patterns in use:**

- **HashSet validation**: `config.rs::validate()` uses `HashSet<&String>` to check phase name uniqueness via `seen_names.insert(&phase.name)`
- **Exact string match**: All phase lookups use `p.name == phase_name` — case-sensitive, no normalization
- **Migration summary counters**: `status_changes`, `phase_clears`, `unchanged` tracked and logged at migration end
- **Option-based clearing**: Phase clearing done by setting to `None`; phase_pool also cleared for consistency

### Reusable Components

- `log_warn!()` macro — already imported in migration.rs, ready for validation warnings
- HashSet pattern from `config.rs::validate()` — directly applicable for building valid phase name set
- Migration summary counter pattern — extend with `validation_clears` counter
- Existing phase clearing logic in Researching handler — same pattern for validation-based clearing

### Constraints from Existing Code

1. **Function signature**: `migrate_v1_to_v2(path: &Path)` takes only a path. Must be extended to accept pipeline config. Adding a parameter changes the call site(s).
2. **Idempotency**: V2 input returns as-is (early return at lines 188-192). Validation must NOT run on v2-passthrough.
3. **Ordering**: Researching status clearing happens BEFORE validation should run. Validation only applies to items that still have a phase after status-based clearing.
4. **phase_pool consistency**: When phase is cleared, phase_pool must also be cleared.
5. **All items get `pipeline_type: "feature"`**: V1 has no pipeline_type concept; all migrated items validate against the feature pipeline.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| V1 phase names match default v2 pipeline | Default v2 pipeline has "tech-research" not "research" as a main phase; "research" exists only as a pre_phase | V1 `Research` phase → "research" string will match the pre_phase, so it validates correctly. No issue here, but worth noting the pre_phase vs main phase distinction. |
| Function signature change is straightforward | Only one call site needs updating (wherever `migrate_v1_to_v2` is called) | Need to identify and update the call site; likely in `main.rs` or a command handler |
| Phase pool should be cleared when phase is cleared | Existing Researching handler already does this pattern | Consistent with established codebase pattern; no concern |

---

## Critical Areas

### Validation Ordering Relative to Researching Handling

**Why it's critical:** The Researching status handler already clears phases for Researching items. If validation runs BEFORE this handler, it would redundantly warn about phases that are going to be cleared anyway.

**Why it's easy to miss:** The PRD specifies validation runs "after existing status-based logic" but the implementation must ensure this ordering is correct within `map_v1_item()` or in the post-mapping loop.

**What to watch for:** Validation should check items AFTER `map_v1_item()` returns the converted item. Since `map_v1_item()` already handles Researching clearing, the validation loop only needs to check items that still have a phase set. This is the natural placement — validate in the migration loop after mapping, not inside `map_v1_item()`.

### Pipeline Config Availability

**Why it's critical:** The migration function currently has no access to pipeline config. The function signature must change, which affects callers.

**Why it's easy to miss:** Need to trace the call chain to understand what the caller has available and whether `load_config` has already been called upstream.

**What to watch for:** The caller should load config via `load_config()` and pass the relevant `PipelineConfig` (the "feature" pipeline) to the migration function. This keeps migration testable — tests can pass custom pipelines to verify validation behavior.

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| Accept `&PipelineConfig` or `&HashSet<String>` as parameter? | Determines coupling between migration and config types | `&PipelineConfig` is more flexible and self-documenting; `HashSet<String>` is simpler but loses context. **Recommend `&PipelineConfig`** — it follows existing patterns and allows the migration to extract what it needs. |
| Should validation warnings be errors? | Affects migration behavior on invalid phases | **Recommend warnings only** — clearing the phase is a safe self-correcting action, and making it an error would block migration unnecessarily. |
| Where does validation happen — inside `map_v1_item()` or in the migration loop? | Affects code organization and testability | **Recommend in the migration loop** (after `map_v1_item()` returns). This keeps `map_v1_item` focused on data conversion and puts validation logic at the migration level where it has access to counters and config. |

### Recommended Approaches

#### Function Signature Change

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Accept `&PipelineConfig` | Type-safe, self-documenting, allows migration to build HashSet internally; tests can inject custom pipelines | Couples migration module to config types (already in same crate) | You want clear API semantics and testability |
| Accept `&HashSet<String>` | Decoupled from config types; migration only knows about strings | Loses context (caller must pre-build set); less self-documenting | You want maximum decoupling |

**Initial recommendation:** Accept `&PipelineConfig` — the types are in the same crate, and it provides clearer API semantics. The migration function can build the `HashSet` internally from `pre_phases` and `phases`.

#### Validation Placement

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| In migration loop, after `map_v1_item()` | Clean separation; validation has access to counters and logging; clear ordering after Researching handling | Slightly more code in the loop | Natural placement, recommended |
| Inside `map_v1_item()` | Keeps all item logic together | Requires passing config/set into mapping function; mixes conversion with validation | Want to keep all item logic contained |

**Initial recommendation:** In migration loop — keeps `map_v1_item()` focused on data conversion, puts validation at the orchestration level.

#### Counter for Validation Clears

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Separate `validation_clears` counter | Distinguishes validation clears from status-based clears; clear in logs | One more counter to track | Want clear diagnostics (recommended) |
| Reuse existing `phase_clears` counter | Simpler | Loses distinction between reasons for clearing | Want minimal changes |

**Initial recommendation:** Separate `validation_clears` counter — the PRD's "Should Have" criteria explicitly asks for this distinction.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| `orchestrator/src/migration.rs` | Source | Primary implementation target |
| `orchestrator/src/config.rs` | Source | `PipelineConfig` type definition, `default_feature_pipeline()`, `load_config()` |
| `orchestrator/src/config.rs:147-175` | Source | HashSet validation pattern to follow |
| `orchestrator/src/types.rs` | Source | `BacklogItem`, `PhasePool` type definitions |
| `orchestrator/src/log.rs` | Source | `log_warn!` macro for validation warnings |
| `orchestrator/tests/migration_test.rs` | Source | Test patterns and fixtures to extend |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Internal codebase analysis (light) | Mapped migration flow, config structure, integration points, and test infrastructure. All research questions answered. |
