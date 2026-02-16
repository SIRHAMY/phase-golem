# Tech Research: Log warning when load_config silently injects default pipeline

**ID:** WRK-010
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-010_log-warning-when-load-config-silently-injects-default-pipeline_PRD.md
**Mode:** Light

## Overview

Research how to add `log_warn!()` messages in `load_config()` when the default pipeline is silently injected. The change is straightforward — we need to verify the PRD's assumptions about macro availability, code structure, message format conventions, and interaction with related changes (WRK-008, WRK-009).

## Research Questions

- [x] Is `log_warn!` available in `config.rs` without additional imports?
- [x] What message format convention does the codebase use for log tags?
- [x] Where exactly in `load_config()` should the log calls go?
- [x] Are there conflicts with WRK-008 or WRK-009 changes?

---

## External Research

### Landscape Overview

Rust's logging ecosystem is mature, built around the `log` crate facade with implementations like `env_logger` and `tracing`. Warning when defaults are silently applied is a well-established pattern — `warn!` is the correct log level for "something unexpected happened but we recovered."

### Common Patterns & Approaches

#### Pattern: Direct Logging at Decision Point

**How it works:** Call `warn!()` at the exact location where a default is about to be applied.

**When to use:** When the default-injection decision is localized and happens rarely (e.g., once in `load_config()`).

**Tradeoffs:**
- Pro: Clear, easy to understand, obvious at the call site
- Pro: Minimal code change
- Con: Requires logger to be initialized before this code path runs (not a concern here — logger is initialized in `main()`)

**References:**
- [log crate documentation](https://docs.rs/log) — Core facade for Rust logging
- [log crate warn! macro](https://docs.rs/log/latest/log/macro.warn.html) — Standard warn! macro

### Standards & Best Practices

- Use `warn` for "something unexpected happened but we recovered" — matches default-injection scenario perfectly
- Be selective about what gets warned — only warn for significant defaults (missing config sections), not every field
- Include actionable context in warning messages (which file to edit, what was injected)

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Logger not initialized before warn call | Messages silently dropped | Logger is initialized early in `main()` — not a concern |
| Over-logging defaults | Creates noise for every small default | Only logging for the significant case (entire pipeline section missing) |

### Key Learnings

- The "direct logging at decision point" pattern is the right fit — simple, explicit, minimal code
- `warn` is the correct log level per Rust conventions
- This codebase uses a custom `log_warn!` macro (not the `log` crate), but the pattern is identical

---

## Internal Research

### Existing Codebase State

The project uses a custom logging macro system in `log.rs` with level-gated macros (`log_warn!`, `log_info!`, `log_debug!`, `log_error!`) that output to stderr via `eprintln!`. The default log level is `Info`, which includes `Warn` — so warnings are visible by default.

**Relevant files/modules:**
- `.claude/skills/changes/orchestrator/src/config.rs` — Contains `load_config()` (lines 207-236), `populate_default_pipelines()` (lines 238-244), and `default_feature_pipeline()` (lines 98-145)
- `.claude/skills/changes/orchestrator/src/log.rs` — Defines `log_warn!` macro (lines 49-56) with `#[macro_export]`
- `.claude/skills/changes/orchestrator/tests/config_test.rs` — Comprehensive test coverage for config loading

**Existing patterns in use:**
- Log messages use `[tag]` prefix: e.g., `log_warn!("[pre] Pruned {} stale dependency reference(s) from backlog", pruned_count);`
- Macros are `#[macro_export]` — available at crate root without `use` statements
- Output goes to stderr, so tests that assert on return values are unaffected

### Key Code: `load_config()` (lines 207-236)

```rust
pub fn load_config(project_root: &Path) -> Result<OrchestrateConfig, String> {
    let config_path = project_root.join("orchestrate.toml");

    if !config_path.exists() {
        // Path 1: No config file
        let mut config = OrchestrateConfig::default();
        populate_default_pipelines(&mut config);
        return Ok(config);
    }

    // Path 2: Config file exists
    let contents = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read {}: {}", config_path.display(), e))?;

    let mut config: OrchestrateConfig = toml::from_str(&contents)
        .map_err(|e| format!("Failed to parse {}: {}", config_path.display(), e))?;

    populate_default_pipelines(&mut config);

    validate(&config).map_err(|errors| { ... })?;

    Ok(config)
}
```

### Key Code: `populate_default_pipelines()` (lines 238-244)

```rust
fn populate_default_pipelines(config: &mut OrchestrateConfig) {
    if config.pipelines.is_empty() {
        config.pipelines.insert("feature".to_string(), default_feature_pipeline());
    }
}
```

### Key Code: `log_warn!` macro (lines 49-56 of log.rs)

```rust
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        if $crate::log::current_log_level() >= $crate::log::LogLevel::Warn {
            eprintln!($($arg)*)
        }
    };
}
```

### Reusable Components

- `log_warn!` macro — ready to use, no imports needed
- `[tag]` message format convention — use `[config]` tag to match PRD examples

### Constraints from Existing Code

- `populate_default_pipelines()` only checks `config.pipelines.is_empty()` — doesn't know why pipelines are empty
- Only `load_config()` has the context to distinguish "no config file" vs. "config file without pipelines"
- The log must happen BEFORE `populate_default_pipelines()` (or check the condition separately) since after that call `config.pipelines` is no longer empty

### Related Changes

**WRK-009** (validate on no-config-file path): Adds `validate()` call to Path 1. Modifies the same code path but is independent — WRK-010 adds logging, WRK-009 adds validation. No conflict.

**WRK-008** (default_feature_pipeline helper): Adds `PhaseConfig::new()` constructor. Affects `default_feature_pipeline()` function internals only. Unrelated to WRK-010.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| `log_warn!` available via `#[macro_export]` without imports | Confirmed — `#[macro_export]` at line 49 of `log.rs` | No additional `use` statements needed |
| `[config]` tag format for messages | Consistent with existing `[pre]` tag usage | Use `[config]` as specified |
| Check belongs in `load_config()` not `populate_default_pipelines()` | Confirmed — only `load_config()` knows which code path was taken | Implement exactly as PRD describes |
| Existing tests unaffected | Confirmed — `log_warn!` outputs to stderr, tests don't capture stderr | No test changes needed |

No concerns found. PRD assumptions are all confirmed by research.

---

## Critical Areas

No critical areas identified. This is a minimal, low-risk addition of two log statements to well-understood code paths.

---

## Deep Dives

None needed — the scope is clear and all assumptions verified.

---

## Synthesis

### Open Questions

None. All research questions answered.

### Recommended Approaches

#### Implementation Approach

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Check `is_empty()` before `populate_default_pipelines()` | Explicit, matches PRD, check happens at natural decision point | Duplicates the `is_empty()` check from `populate_default_pipelines()` | Always — this is the correct approach |

**Initial recommendation:** Check `config.pipelines.is_empty()` in `load_config()` BEFORE calling `populate_default_pipelines()` in both code paths. Log the appropriate message, then call `populate_default_pipelines()`. This is the simplest and most explicit approach.

**Implementation sketch:**

```rust
// Path 1: No config file
if !config_path.exists() {
    let mut config = OrchestrateConfig::default();
    if config.pipelines.is_empty() {
        log_warn!("[config] No orchestrate.toml found; using default 'feature' pipeline");
    }
    populate_default_pipelines(&mut config);
    return Ok(config);
}

// Path 2: Config file exists, after parsing
if config.pipelines.is_empty() {
    log_warn!("[config] No pipelines defined in orchestrate.toml; using default 'feature' pipeline");
}
populate_default_pipelines(&mut config);
```

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [log crate documentation](https://docs.rs/log) | Docs | Standard Rust logging patterns |
| [Serde default attributes](https://serde.rs/attr-default.html) | Docs | Understanding config deserialization defaults |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Light internal + external research | All PRD assumptions confirmed; implementation approach clear |

## Assumptions

- No human was available for questions during this research
- Mode defaulted to "light" based on item assessments (small size, low complexity, low risk)
- All PRD assumptions were verified and found accurate — no concerns raised
