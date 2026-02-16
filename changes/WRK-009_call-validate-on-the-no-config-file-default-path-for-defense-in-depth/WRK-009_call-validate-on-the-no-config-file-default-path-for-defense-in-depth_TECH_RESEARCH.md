# Tech Research: Call validate() on the no-config-file default path

**ID:** WRK-009
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-009_call-validate-on-the-no-config-file-default-path-for-defense-in-depth_PRD.md
**Mode:** Light

## Overview

Researching the existing `load_config()` implementation and validation patterns to confirm the approach for adding a `validate()` call to the no-config-file code path. The change is small and well-defined; research focuses on understanding the exact code to modify and confirming the error formatting pattern.

## Research Questions

- [x] What is the exact code structure at the modification point?
- [x] What error formatting pattern does the config-file-exists path use?
- [x] What existing test coverage exists, and will it implicitly cover the change?

---

## External Research

### Landscape Overview

Defense-in-depth configuration validation is a standard practice: every code path producing a config struct should validate it before returning, regardless of whether the source is "trusted" (defaults) or "untrusted" (user-provided file). The Rust ecosystem supports this through explicit validation functions, derive-macro-based validation crates (Garde, Validator), and type-driven approaches ("parse, don't validate").

### Common Patterns & Approaches

#### Pattern: Explicit Validation Function Applied Consistently

**How it works:** Define a single `validate()` function and call it on every code path that produces configuration.

**When to use:** When multiple code paths produce config (file, defaults, env vars) and all need identical validation.

**Tradeoffs:**
- Pro: Simple, direct, no additional dependencies
- Pro: Makes validation explicit and auditable
- Con: Requires discipline — easy to forget on new paths

**References:**
- [Rust Security Best Practices 2025](https://hub.corgea.com/articles/rust-security-best-practices) — validates-at-boundary approach
- [Rust CLI Book: Configuration Files](https://rust-cli.github.io/book/in-depth/config-files.html) — config loading patterns

### Standards & Best Practices

- Treat all config sources (including defaults) as needing validation
- Validate at the boundary — `load_config()` is the right place
- Use consistent error formatting across all validation paths

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Skipping validation on "safe" paths | Defaults can become invalid after code changes | Always validate, regardless of source |
| Inconsistent error messages between paths | Makes debugging harder | Use consistent formatting, vary only the prefix |

### Key Learnings

- The existing pattern of calling `validate()` followed by `map_err()` formatting is the standard approach — reuse it directly
- No external libraries or new patterns needed for this change

---

## Internal Research

### Existing Codebase State

`load_config()` in `orchestrator/src/config.rs` (lines 207-236) has two code paths:

1. **No config file (lines 210-213):** Creates default config, populates default pipelines, returns `Ok(config)` — **no validation**
2. **Config file exists (lines 216-233):** Parses TOML, populates default pipelines, validates, returns

The `validate()` function (lines 147-205) returns `Result<(), Vec<String>>`, collecting all errors in a single pass.

**Relevant files/modules:**
- `orchestrator/src/config.rs` — `load_config()`, `validate()`, `populate_default_pipelines()`
- `orchestrator/tests/config_test.rs` — test coverage including `load_config_defaults_when_file_missing`
- `orchestrator/src/main.rs` — calls `load_config()` in `handle_run()`, `handle_triage()`, `handle_add()`

**Existing patterns in use:**
- Error formatting: `validate(&config).map_err(|errors| format!("Config validation failed:\n{}", errors.iter().map(|e| format!("  - {}", e)).collect::<Vec<_>>().join("\n")))?;`
- Call order: `populate_default_pipelines()` then `validate()`
- Error propagation: callers use `?` operator; errors bubble to stderr via main

### Reusable Components

- `validate()` function — reuse directly, no changes needed
- Error formatting `map_err()` closure — copy the pattern, change prefix to "Default config validation failed:"

### Constraints from Existing Code

- `load_config()` signature cannot change (public API)
- Must preserve behavior when defaults are valid (existing tests must pass)
- `populate_default_pipelines()` must be called before `validate()`

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Implementation is straightforward | Confirmed — 2-3 line addition following existing pattern | No design complexity |
| Existing test implicitly covers the change | Confirmed — `load_config_defaults_when_file_missing` calls `load_config()` and asserts `Ok` | No new tests needed |
| Error prefix "Default config validation failed:" | Config-file path uses "Config validation failed:" — differentiation is appropriate | Consistent but distinguishable error messages |

No concerns — research fully aligns with PRD.

---

## Critical Areas

None. This is a minimal change following an established pattern with existing test coverage.

---

## Deep Dives

None needed — light mode research was sufficient.

---

## Synthesis

### Open Questions

None — all research questions resolved.

### Recommended Approaches

#### Validation on No-Config-File Path

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Copy `validate()` + `map_err()` pattern inline | Simple, explicit, follows existing code | Minor duplication of error formatting | Small codebase, few paths (our case) |
| Extract shared `validate_and_format()` helper | DRY, single formatting point | Over-engineering for 2 call sites | Many config paths need validation |

**Initial recommendation:** Copy the pattern inline with modified prefix. Extracting a helper is over-engineering for two call sites and is explicitly out of scope per PRD.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| `orchestrator/src/config.rs:210-213` | Source code | Exact modification point |
| `orchestrator/src/config.rs:224-233` | Source code | Error formatting pattern to replicate |
| `orchestrator/tests/config_test.rs:4-16` | Test code | Implicit coverage of the change |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Light internal + external research | Confirmed approach, no surprises. Change is straightforward 2-3 line addition. |
