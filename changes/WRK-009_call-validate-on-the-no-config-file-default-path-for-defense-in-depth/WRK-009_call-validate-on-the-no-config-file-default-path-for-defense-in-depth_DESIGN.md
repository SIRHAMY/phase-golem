# Design: Call validate() on the no-config-file default path for defense-in-depth

**ID:** WRK-009
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-009_call-validate-on-the-no-config-file-default-path-for-defense-in-depth_PRD.md
**Tech Research:** ./WRK-009_call-validate-on-the-no-config-file-default-path-for-defense-in-depth_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add a `validate()` call to the no-config-file branch of `load_config()` in `config.rs`, mirroring the existing validation on the config-file-exists branch. This ensures defense-in-depth: any future change to defaults that produces an invalid config will be caught immediately, rather than silently propagating. The implementation copies the existing `validate()` + `map_err()` pattern with a distinct error prefix.

---

## System Design

### High-Level Architecture

No new components or architectural changes. This is a single-site code modification within the existing `load_config()` function. The function already has validation on one of its two branches; this design adds it to the other.

### Component Breakdown

#### `load_config()` — Modified Function

**Purpose:** Loads or constructs the orchestrator configuration, validates it, and returns it.

**Responsibilities:**
- Detect whether a config file exists
- If no file: construct default config, populate default pipelines, **validate**, return
- If file exists: parse TOML, populate default pipelines, validate, return

**Interfaces:**
- Input: `&Path` (project root)
- Output: `Result<OrchestrateConfig, String>`

**Dependencies:** `validate()`, `populate_default_pipelines()`, `OrchestrateConfig::default()`

### Data Flow

1. `load_config()` checks if `orchestrate.toml` exists
2. **No-config-file path (modified):**
   a. Create `OrchestrateConfig::default()`
   b. Call `populate_default_pipelines(&mut config)`
   c. Call `validate(&config)` — **new step**
   d. If validation fails, return `Err` with "Default config validation failed:" prefix — **new step**
   e. Return `Ok(config)`
3. Config-file-exists path: unchanged

### Key Flows

#### Flow: No-config-file load with validation

> Load default config, validate it, and return — failing fast if defaults are invalid.

1. **Check config path** — `orchestrate.toml` does not exist
2. **Create defaults** — `OrchestrateConfig::default()`
3. **Populate pipelines** — `populate_default_pipelines(&mut config)`
4. **Validate** — `validate(&config)` checks all validation rules
5. **Format errors on failure** — `map_err()` produces "Default config validation failed:\n  - error1\n  - error2"
6. **Return** — `Ok(config)` if valid, `Err(message)` if not

**Edge cases:**
- Default config is valid (expected case) — validation passes, no behavioral change
- Default config becomes invalid due to future code change — validation catches it, returns descriptive error

---

## Technical Decisions

### Key Decisions

#### Decision: Inline pattern copy vs. extracted helper

**Context:** The `validate()` + `map_err()` pattern already exists on the config-file path. We need the same pattern on the no-config-file path.

**Decision:** Copy the pattern inline with a different error prefix ("Default config validation failed:" vs. "Config validation failed:").

**Rationale:** Two call sites don't justify a helper function. The inline approach is simpler, more explicit, and avoids over-engineering. The PRD explicitly scopes this as an inline addition.

**Consequences:** Minor code duplication (the `map_err` formatting closure). Acceptable for two call sites.

#### Decision: Error prefix differentiation

**Context:** Both paths now call `validate()`. Errors should be distinguishable to aid debugging.

**Decision:** Use "Default config validation failed:" prefix for the no-config-file path, keeping "Config validation failed:" for the file-exists path.

**Rationale:** If a developer sees this error, they immediately know it came from the default config construction, not from a parsed file. This matches the PRD requirement.

**Consequences:** Slightly different error messages between paths, which is intentional and useful.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Minor code duplication | Two similar `validate()` + `map_err()` blocks | Simplicity, no new abstractions | Two call sites is below the threshold for extraction |
| Negligible startup cost | One extra validation call on default path | Defense-in-depth safety | Validation is fast, runs once at startup |

---

## Alternatives Considered

### Alternative: Extract a shared `validate_and_format()` helper

**Summary:** Create a helper function that combines `validate()` and the `map_err()` formatting, parameterized by an error prefix string.

**How it would work:**
- New function: `fn validate_and_format(config: &OrchestrateConfig, prefix: &str) -> Result<(), String>`
- Both code paths call this helper instead of inline `validate()` + `map_err()`

**Pros:**
- DRY — single formatting point
- Easy to add more config paths in the future

**Cons:**
- Over-engineering for exactly two call sites
- Adds indirection for minimal benefit
- Out of scope per PRD

**Why not chosen:** The PRD explicitly scopes this as adding a `validate()` call, not refactoring validation. Two call sites don't warrant a new abstraction. The simpler inline approach is preferred.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Default config becomes invalid after code change | Test failure on CI | Very low | This is exactly the scenario we want to catch — the validation does its job |

---

## Integration Points

### Existing Code Touchpoints

- `orchestrator/src/config.rs` lines 210-213 — The no-config-file branch of `load_config()`. Insert `validate()` + `map_err()` between `populate_default_pipelines()` and `return Ok(config)`.

**Current code (lines 210-214):**
```rust
if !config_path.exists() {
    let mut config = OrchestrateConfig::default();
    populate_default_pipelines(&mut config);
    return Ok(config);
}
```

**After change:**
```rust
if !config_path.exists() {
    let mut config = OrchestrateConfig::default();
    populate_default_pipelines(&mut config);
    validate(&config).map_err(|errors| {
        format!(
            "Default config validation failed:\n{}",
            errors
                .iter()
                .map(|e| format!("  - {}", e))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })?;
    return Ok(config);
}
```

**Note:** `populate_default_pipelines()` must be called before `validate()` because validation rules check pipeline structure (e.g., phase uniqueness, non-empty phases) that `populate_default_pipelines()` creates. This matches the call order on the config-file-exists path (lines 222-233).

### External Dependencies

None.

---

## Open Questions

None.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Initial design draft | Straightforward inline pattern copy, one alternative noted and rejected |
| 2026-02-13 | Self-critique (7 agents) | No critical or directional issues. Auto-fixed: added concrete before/after code snippet and call ordering note to Integration Points. |
