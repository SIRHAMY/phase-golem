# Design: Add Phase Name Validation During V1-to-V2 Migration

**ID:** WRK-011
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-011_add-phase-name-validation-during-v1-to-v2-migration_PRD.md
**Tech Research:** ./WRK-011_add-phase-name-validation-during-v1-to-v2-migration_TECH_RESEARCH.md
**Mode:** Light

## Overview

After converting V1 items to V2 format, validate each item's phase name against the target pipeline's configured phases (pre_phases + main phases). Invalid phase names are logged with a warning and cleared to `None` (along with `phase_pool`), allowing the scheduler to reassign the correct first phase on the next run. The migration function signature is extended to accept a `&PipelineConfig` parameter, keeping the function testable and decoupled from config loading. The `backlog::load()` function gains a `project_root: &Path` parameter so it can load pipeline config and pass it to the migration function.

---

## System Design

### High-Level Architecture

The change is localized to three touchpoints:

1. **`migrate_v1_to_v2` in `migration.rs`** — Accepts `&PipelineConfig`, builds a `HashSet<&str>` of valid phase names from `pre_phases` + `phases`, and validates each item's phase after `map_v1_item()` returns.
2. **`backlog::load()` in `backlog.rs`** — Accepts new `project_root: &Path` parameter, loads config via `load_config(project_root)`, extracts the `"feature"` pipeline, and passes it to the migration function.
3. **Call sites of `backlog::load()`** — Updated to pass `project_root` (already available at all call sites in `main.rs`).

No new modules, types, or files are introduced.

### Component Breakdown

#### Validation Logic (in `migrate_v1_to_v2`)

**Purpose:** Check that each migrated item's phase name exists in the target pipeline configuration.

**Responsibilities:**
- Build a `HashSet<&str>` from `pipeline.pre_phases` and `pipeline.phases` names (done once, before the item loop)
- After `map_v1_item()` returns a V2 item and before pushing it to the items vector, check if `item.phase` is `Some(name)` where `name` is not in the valid set
- If invalid: log a `log_warn!` with item ID, invalid phase name, and pipeline context; clear `phase` to `None` and `phase_pool` to `None`; increment `validation_clears` counter
- If valid or `None`: no action
- Phase name matching is case-sensitive and exact (consistent with the scheduler's `p.name == phase_name` behavior)

**Interfaces:**
- Input: `&PipelineConfig` (new parameter on `migrate_v1_to_v2`)
- Output: Validated `BacklogFile` (same return type)

**Dependencies:** `PipelineConfig` from `config.rs` (same crate, already available)

#### Call Site Update (`backlog.rs`)

**Purpose:** Supply pipeline config to the migration function.

**Responsibilities:**
- Accept `project_root: &Path` as a new parameter on `backlog::load()`
- Load `OrchestrateConfig` via `load_config(project_root)`
- Extract the `"feature"` pipeline from `config.pipelines`; if missing, return `Err("Migration requires 'feature' pipeline in config, but none found")`
- Pass `&PipelineConfig` to `migrate_v1_to_v2`

**Interfaces:**
- Input: `path: &Path` (existing), `project_root: &Path` (new)
- Output: unchanged `Result<BacklogFile, String>`

**Dependencies:** `load_config` from `config.rs`

#### Call Site Updates (`main.rs` and tests)

**Purpose:** Pass project root to `backlog::load()`.

**Responsibilities:**
- All `backlog::load()` call sites in `main.rs` pass the existing `root` variable as `project_root`
- All test call sites either pass a test fixture directory or construct a temp dir with appropriate config

### Data Flow

1. `backlog::load(path, project_root)` detects v1 schema, loads config via `load_config(project_root)`, extracts feature pipeline
2. Calls `migrate_v1_to_v2(path, &pipeline_config)`
3. `migrate_v1_to_v2` builds `valid_phases: HashSet<&str>` from pipeline config
4. For each V1 item: `map_v1_item()` converts to V2 (status mapping, Researching clearing)
5. Post-mapping, pre-push: if `v2_item.phase` is `Some(name)` and `name` is not in `valid_phases`, warn and clear
6. Item is pushed to the items vector
7. Summary log includes `validation_clears` counter alongside existing `status_changes`, `phase_clears`, `unchanged`

### Key Flows

#### Flow: Migration with Valid Phases (Happy Path)

> All V1 phase names map to valid V2 pipeline phases. No validation warnings.

1. **Load config** — `backlog::load()` calls `load_config(project_root)`, gets default feature pipeline (with phases: `["research", "prd", "tech-research", "design", "spec", "build", "review"]`)
2. **Extract pipeline** — Get `"feature"` pipeline from config (succeeds with default config)
3. **Invoke migration** — Calls `migrate_v1_to_v2(path, &pipeline_config)`
4. **Build valid set** — `HashSet` built from all pre_phase and phase names
5. **Map items** — Each V1 item converted via `map_v1_item()`
6. **Validate phases** — Each item's phase checked against set (case-sensitive exact match); all valid, no warnings. Items with `phase: None` are skipped.
7. **Write and return** — Atomically write V2 file to original path via `NamedTempFile`, return `Ok(BacklogFile)`

#### Flow: Migration with Invalid Phase

> A V1 item has a phase that doesn't exist in the V2 pipeline (e.g., user customized pipeline, removing "prd").

1. **Load config** — Custom pipeline loaded (missing "prd" phase)
2. **Extract pipeline** — Get `"feature"` pipeline from config (succeeds)
3. **Invoke migration** — Same call
4. **Map items** — V1 item with `phase: prd` mapped to `phase: Some("prd")`
5. **Validate** — `"prd"` not in valid set (case-sensitive exact match)
6. **Warn and clear** — `log_warn!("  WRK-001: phase 'prd' not found in feature pipeline phases; cleared")`, set `phase = None`, `phase_pool = None`, increment `validation_clears`
7. **Continue** — Validation continues for remaining items (does not short-circuit on first invalid phase)
8. **Summary** — Log includes `validation_clears: 1`

#### Flow: Migration with Missing Feature Pipeline

> User's custom config defines pipelines but none named "feature".

1. **Load config** — Custom config loaded with pipelines like "custom-flow" but no "feature"
2. **Extract pipeline** — `config.pipelines.get("feature")` returns `None`
3. **Error** — Return `Err("Migration requires 'feature' pipeline in config, but none found")`
4. **No migration performed** — Original V1 file remains unchanged

**Edge cases:**
- Item with `phase: None` (no phase set) — skipped, no validation needed
- Researching item — phase already cleared by `map_v1_item()`, validation sees `None`, skips
- Config loading fails (corrupted `orchestrate.toml`) — `load_config()` error propagates, migration aborted before any items processed
- No config file exists — `load_config()` falls back to default config (includes "feature" pipeline), migration proceeds normally
- Empty pipeline (zero pre_phases and zero phases) — Cannot occur with valid config; `config::validate()` requires at least one main phase per pipeline

---

## Technical Decisions

### Key Decisions

#### Decision: Accept `&PipelineConfig` as parameter

**Context:** The migration function needs access to valid phase names. Options: accept `&PipelineConfig`, accept `HashSet<String>`, or load config internally.

**Decision:** Accept `&PipelineConfig`.

**Rationale:** The type is in the same crate, provides clear API semantics, and is self-documenting. The migration function builds its own `HashSet` internally from the config. Tests can inject custom `PipelineConfig` values to verify validation behavior without needing real config files.

**Consequences:** The call site in `backlog.rs` must load config and extract the feature pipeline. The function signature changes from `(path: &Path)` to `(path: &Path, pipeline: &PipelineConfig)`.

#### Decision: Add `project_root: &Path` parameter to `backlog::load()`

**Context:** `backlog::load()` currently only receives `path: &Path` (the backlog file path) but needs access to pipeline config for migration validation. Options: (A) add `project_root: &Path` parameter, (B) derive project root from backlog path by walking up the directory tree, (C) pass `&PipelineConfig` directly to `load()`.

**Decision:** Add `project_root: &Path` parameter (Option A).

**Rationale:** Explicit over implicit — follows the coding style guide. The caller (`main.rs`) already has the project root variable (`root`). Deriving project root from the backlog path (Option B) is fragile and assumes directory structure. Passing `&PipelineConfig` (Option C) pushes too much knowledge to the caller about when config is needed. Adding a `project_root` parameter keeps `load()` self-contained while making the dependency explicit.

**Consequences:** All `backlog::load()` call sites must be updated to pass `project_root`. In `main.rs`, the `root` variable is already available at all call sites. Tests need to either provide a real project root or construct a temp dir.

#### Decision: Validate in migration loop, not inside `map_v1_item()`

**Context:** Validation could happen inside `map_v1_item()` (per-item conversion) or in the migration loop after mapping.

**Decision:** Validate in the migration loop after `map_v1_item()` returns.

**Rationale:** Keeps `map_v1_item()` focused on data conversion. Validation at the loop level has access to counters, logging, and config without threading extra parameters through the mapping function. The ordering naturally ensures validation runs after Researching-status clearing.

**Consequences:** `map_v1_item()` remains unchanged. Validation is a few lines added to the existing loop in `migrate_v1_to_v2`.

#### Decision: Warnings only, not errors

**Context:** Should invalid phase names cause migration to fail or just warn?

**Decision:** Warn and clear (not error).

**Rationale:** Clearing an invalid phase is self-correcting — the scheduler assigns the correct first phase to items without a phase. Blocking migration on a fixable condition would force manual intervention for something the system can handle automatically.

**Consequences:** Migration always succeeds (assuming config loads). Items with invalid phases are auto-corrected via clearing. Original invalid phase names are only preserved in the warning log output, not in the migrated file — operators must review logs to see what was corrected.

#### Decision: Clear `phase_pool` when clearing `phase`

**Context:** When validation clears an invalid phase, should `phase_pool` also be cleared?

**Decision:** Yes, clear both.

**Rationale:** `phase_pool` indicates which pool (Pre or Main) the item's current phase belongs to. Without a valid phase, the pool value is semantically meaningless and could confuse the scheduler. The existing Researching-status handler in `map_v1_item()` already follows this pattern — clearing phase and phase_pool together. Consistency with existing patterns reduces cognitive load.

**Consequences:** Items with cleared phases have both `phase: None` and `phase_pool: None`, which is the same state as a newly created item.

#### Decision: Separate `validation_clears` counter

**Context:** Should validation-based phase clears use the existing `phase_clears` counter or a new separate counter?

**Decision:** Add a separate `validation_clears` counter.

**Rationale:** The existing `phase_clears` counter tracks items whose phase was cleared by status-based logic (specifically, Researching → Scoping items). The new `validation_clears` counter tracks items whose phase was cleared because the phase name doesn't exist in the pipeline config. These are distinct reasons for clearing, and distinguishing them in logs helps operators understand what happened. The PRD's "Should Have" criteria explicitly asks for this distinction.

**Consequences:** The summary log format expands to include the new counter. The `validation_clears` counter is mutually exclusive with `phase_clears` — an item whose phase was already cleared by Researching handling won't be validated (phase is `None`), so no double-counting occurs.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Signature changes | Breaking change to `migrate_v1_to_v2` and `backlog::load()` function signatures; all callers and tests must update | Testable, explicit dependencies; no hidden config loading inside migration | Only one production call site (`main.rs` → `backlog::load()` → `migrate_v1_to_v2`) and one test file need updating; small blast radius |
| Warn-and-clear vs error | Items silently corrected; original invalid phase names not preserved in migrated file; operators may not notice if logs aren't reviewed | Migration always succeeds; system self-corrects; no manual intervention needed | The `log_warn!` is logged to stderr; the summary counter provides visibility; blocking would be worse for UX |

---

## Alternatives Considered

### Alternative: Accept `HashSet<String>` instead of `&PipelineConfig`

**Summary:** Pass a pre-built set of valid phase name strings instead of the full pipeline config.

**How it would work:**
- Caller builds `HashSet<String>` from pipeline's `pre_phases` and `phases`
- Migration function does a simple set membership check

**Pros:**
- Maximum decoupling — migration doesn't depend on `PipelineConfig` type
- Slightly simpler function body (no need to build HashSet internally)

**Cons:**
- Less self-documenting — caller must know to combine pre_phases and phases
- Loses context — can't produce helpful error messages referencing pipeline structure
- More work at each call site to pre-build the set

**Why not chosen:** The types are in the same crate, so decoupling provides no real benefit. `&PipelineConfig` is clearer at the call site and allows the migration function to produce better log messages.

### Alternative: Derive project root from backlog path

**Summary:** Instead of adding `project_root` parameter to `backlog::load()`, walk up from the backlog file path looking for `orchestrate.toml` or `.git`.

**How it would work:**
- In `backlog::load()`, take the backlog file path's parent directory and walk upward until finding a project root marker

**Pros:**
- No signature change to `backlog::load()`
- Fewer call site updates

**Cons:**
- Implicit and fragile — depends on directory structure assumptions
- Can fail in edge cases (backlog at filesystem root, symlinks, relative paths)
- Violates "explicit over implicit" principle

**Why not chosen:** The caller already has the project root, so passing it explicitly is simpler and more reliable.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Config loading fails at migration time | Migration aborted, backlog stays v1 | Low — `load_config` falls back to defaults when no config file exists; only fails with corrupted config | Error propagation with clear message; user fixes config and re-runs |
| No `"feature"` pipeline in custom config | All migrated items would fail validation since they get `pipeline_type: "feature"` | Very Low — `populate_default_pipelines` adds feature pipeline when no pipelines are configured | Explicit check in `backlog.rs`: `config.pipelines.get("feature").ok_or_else(...)` returns clear error before invoking migration |
| Test maintenance from signature changes | All existing migration tests must update to pass `&PipelineConfig` | High (intentional) | Existing tests use `default_feature_pipeline()`; one new test uses a custom pipeline config that excludes a V1 phase name |

---

## Integration Points

### Existing Code Touchpoints

- `.claude/skills/changes/orchestrator/src/migration.rs` — Add `pipeline: &PipelineConfig` parameter to `migrate_v1_to_v2`; add `use crate::config::PipelineConfig` import; add `HashSet` construction and validation logic in the item loop (after `map_v1_item()`, before `items.push()`); add `validation_clears` counter to summary log
- `.claude/skills/changes/orchestrator/src/backlog.rs` — Add `project_root: &Path` parameter to `load()`; add config loading and `"feature"` pipeline extraction with error handling; update call to `migrate_v1_to_v2` to pass pipeline config
- `.claude/skills/changes/orchestrator/src/main.rs` — Update all `backlog::load()` calls to pass `&root` as project root (the `root` variable is already available at all call sites)
- `.claude/skills/changes/orchestrator/tests/migration_test.rs` — Update all `migrate_v1_to_v2` calls to pass `&default_feature_pipeline()` for existing tests; add new test with custom pipeline config that excludes a V1 phase name to verify validation behavior

### External Dependencies

None — all types and functions are within the same crate.

---

## Deferred Items

- **Edit distance suggestion in warnings** (PRD Nice to Have) — Deferred to future work. Adds complexity (edit distance library or manual Levenshtein implementation) for marginal UX benefit given the small number of pipeline phases and the low likelihood of this validation firing in practice. If validation warnings become common, this can be added as a follow-up.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain (or they're flagged for spec phase)

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Initial design draft | Recommended approach: accept `&PipelineConfig`, validate in migration loop, warn-and-clear |
| 2026-02-12 | Self-critique and refinement | Resolved project root derivation (add `project_root` param to `backlog::load()`); added missing "feature" pipeline error handling flow; clarified case-sensitivity, counter semantics, phase_pool clearing rationale; added "derive from path" alternative; deferred edit distance suggestion; documented test maintenance risk |

## Assumptions

Decisions made autonomously (no human available for input):

- **Project root derivation approach:** Chose to add `project_root: &Path` parameter to `backlog::load()` rather than deriving from the backlog file path. Rationale: explicit over implicit, caller already has root, avoids fragile directory traversal.
- **Counter semantics:** Chose mutually exclusive counters (`phase_clears` for status-based clearing, `validation_clears` for validation-based clearing) rather than overlapping or merged counters. Rationale: cleaner semantics, no double-counting possible due to ordering.
- **Edit distance deferred:** Chose to explicitly defer the PRD's "Nice to Have" edit distance suggestion rather than including it in the design. Rationale: disproportionate complexity for the scope of this change.
