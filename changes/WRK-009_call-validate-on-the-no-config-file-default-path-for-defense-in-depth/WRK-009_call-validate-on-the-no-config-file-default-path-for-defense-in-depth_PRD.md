# Change: Call validate() on the no-config-file default path for defense-in-depth

**Status:** Proposed
**Created:** 2026-02-13
**Author:** Orchestrator (autonomous)

## Problem Statement

In `config.rs`, the `load_config()` function has two code paths:

1. **Config file exists:** Parses the TOML, calls `populate_default_pipelines()`, then calls `validate()` before returning. If validation fails, an error is returned.
2. **No config file:** Creates `OrchestrateConfig::default()`, calls `populate_default_pipelines()`, and returns immediately — **without calling `validate()`**.

Today the defaults are known to be valid, so skipping validation has no practical impact. However, this creates a latent risk: if `OrchestrateConfig::default()` or `populate_default_pipelines()` is ever modified to produce an invalid configuration, the no-config-file path would silently return that invalid config. The config-file-exists path would catch the same defect because it runs validation.

Defense-in-depth requires that every path producing a config runs validation, regardless of whether the config is "known good" at the time of writing.

## User Stories / Personas

- **Orchestrator developer** — Wants confidence that any future changes to default config values or pipeline construction will be caught by validation, not silently accepted.

## Desired Outcome

Both code paths in `load_config()` call `validate()` on the assembled config before returning `Ok(config)`. If the default config ever becomes invalid (due to a code change), the error is surfaced immediately rather than causing downstream failures.

## Success Criteria

### Must Have

- [ ] The no-config-file path in `load_config()` calls `validate()` on the default config before returning
- [ ] Validation errors on the default path are surfaced as `Err(String)` using the same formatting as the config-file-exists path (prefixed with "Default config validation failed:" followed by indented error list)
- [ ] Existing test `load_config_defaults_when_file_missing` continues to pass

## Scope

### In Scope

- Adding a `validate()` call to the no-config-file branch of `load_config()` in `config.rs`
- Ensuring error formatting is consistent between both paths

### Out of Scope

- Changing the `validate()` function itself
- Changing `OrchestrateConfig::default()` or `populate_default_pipelines()`
- Adding new validation rules
- Adding a dedicated test that the default config passes validation (the existing `load_config_defaults_when_file_missing` test implicitly covers this: it calls `load_config()` which will now return `Err` if defaults are invalid, causing the test to fail)

## Non-Functional Requirements

- **Performance:** Negligible — validation runs once at startup on a small struct

## Constraints

- Must not change the public API of `load_config()`
- Must not change behavior when defaults are valid (existing tests must pass)

## Dependencies

- **Depends On:** None
- **Blocks:** None

## Risks

- [ ] Extremely low risk — adds a check that currently always passes. The only behavioral change would occur if defaults are modified to be invalid in the future, which is exactly the scenario we want to catch.

## Open Questions

None — this is a straightforward defense-in-depth hardening with a single obvious implementation.

## Assumptions

- **Current defaults are valid:** `OrchestrateConfig::default()` combined with `populate_default_pipelines()` produces a config that passes all validation rules. This is verified by the existing test suite.
- **Call ordering preserved:** `populate_default_pipelines()` is called before `validate()` in both code paths, matching the existing pattern.
- **No interview needed:** All requirements are clear from the backlog item title and codebase inspection. This is a well-defined, small change with a single obvious implementation.

## References

- `orchestrator/src/config.rs` lines 207-236 — `load_config()` function
- `orchestrator/src/config.rs` lines 147-205 — `validate()` function
- `orchestrator/tests/config_test.rs` — existing test coverage
