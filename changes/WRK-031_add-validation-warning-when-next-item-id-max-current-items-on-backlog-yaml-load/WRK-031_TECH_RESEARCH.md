# Tech Research: Validate next_item_id on BACKLOG.yaml Load

**ID:** WRK-031
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-031_PRD.md
**Mode:** Light

## Overview

Research how to add a post-deserialization validation warning in `backlog::load()` that detects when `next_item_id` is less than the max item ID suffix. The goal is to understand existing codebase patterns, confirm the implementation approach, and identify any gotchas.

## Research Questions

- [x] What pattern should we use for post-deserialization validation in Rust?
- [x] What existing code can we reuse for ID parsing and max-suffix computation?
- [x] Are there any gotchas around ID parsing, config loading, or the load() function flow?

---

## External Research

### Landscape Overview

Post-deserialization validation in Rust is a well-established pattern. The dominant approach is: deserialize with serde, then run a separate validation pass on the resulting struct before returning. This is recommended by the serde ecosystem itself — serde intentionally does not provide built-in validation hooks (see [serde issue #642](https://github.com/serde-rs/serde/issues/642)). For advisory (non-fatal) issues, the standard practice is to log a warning at WARN level and continue.

### Common Patterns & Approaches

#### Pattern: Inline Post-Deserialization Validation

**How it works:** After serde successfully deserializes a struct, a standalone validation function (or inline checks) runs on the result. If the check fails, log a warning or return an error depending on severity.

**When to use:** When validation is simple (a few cross-field invariant checks) and does not warrant a separate validation library.

**Tradeoffs:**
- Pro: Simple, no dependencies, logic visible at call site
- Con: Can clutter the load function if many validations accumulate

**References:**
- [serde issue #642: Post-deserialization validation hooks](https://github.com/serde-rs/serde/issues/642) — Confirms external post-deserialize validation is the intended approach
- [rust-analyzer config validation #11950](https://github.com/rust-lang/rust-analyzer/issues/11950) — Real-world example in a major Rust project

#### Pattern: Validation via Derive Macros (`validator`, `serde_valid`)

**How it works:** Libraries like `validator` and `serde_valid` provide attribute-based validation (`#[validate(range(min = 1))]`).

**When to use:** When you have many fields with complex validation rules.

**Tradeoffs:**
- Pro: Powerful, declarative
- Con: Adds a dependency, overkill for a single cross-field check

**References:**
- [Keats/validator on GitHub](https://github.com/Keats/validator) — Popular Rust validation crate (~17M downloads)

### Standards & Best Practices

- **WARN level is correct** for advisory data integrity issues — the system compensates and continues, so it's not an ERROR
- **Validation logic should be testable** — extract or reuse computation so it can be unit-tested independently of file I/O
- **Include actionable information** in warnings — state the problem, give values, include file path, suggest a fix
- **Use the same parsing logic** for validation and generation to avoid disagreements

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Leading zeros parsed as octal | Would give wrong values for IDs like `WRK-031` | Non-issue in Rust: `str::parse::<u32>()` always uses base 10 |
| Using `trim_start_matches` instead of `strip_prefix` | `trim_start_matches` removes greedily (all occurrences from start), `strip_prefix` removes exactly once | Use `strip_prefix` as `generate_next_id()` already does |
| Validation logic diverging from generation logic | Warning could report a different max than what `generate_next_id()` computes | Reuse the exact same `strip_prefix` + `parse::<u32>` + `filter_map` + `max` chain |
| Config loading failure blocking backlog load | Validation is advisory; it should never prevent loading | Wrap `load_config()` call in `.ok()` to skip validation gracefully |

### Key Learnings

- Inline post-deserialization validation is the standard Rust approach for this use case
- No new dependencies are needed
- The parsing logic already exists in `generate_next_id()` and can be reused directly

---

## Internal Research

### Existing Codebase State

The codebase has all necessary building blocks already in place:

**Relevant files/modules:**
- `src/backlog.rs:23-66` — `load()` function: reads YAML, checks schema version, runs migrations if needed, parses into `BacklogFile`. Returns `Result<BacklogFile, String>`
- `src/backlog.rs:108-125` — `generate_next_id()`: implements ID parsing with `strip_prefix` + `parse::<u32>` + `filter_map` + `max` chain
- `src/types.rs:227-237` — `BacklogFile` struct with `next_item_id: u32` field
- `src/config.rs:366-396` — `load_config(project_root: &Path) -> Result<PhaseGolemConfig, String>`: returns defaults if config file missing (graceful)
- `src/log.rs:49-56` — `log_warn!` macro: checks log level, calls `eprintln!`

**Existing patterns in use:**
- ID parsing: `filter_map` + `strip_prefix` + `parse::<u32>().ok()` + `.max().unwrap_or(0)` (in `generate_next_id`)
- Warning logging: `log_warn!("context: message with {} values", var)` (seen in migration.rs, backlog.rs line 317)
- Config access: `load_config(project_root)?` (used in v1 migration path)

### Reusable Components

- **ID parsing logic** from `generate_next_id()` (lines 109-121) — exact pattern needed for computing max suffix
- **`log_warn!` macro** — already imported in backlog.rs (line 8)
- **`load_config()`** — already imported in backlog.rs (line 7), returns config with `project.prefix`
- **`path.display()`** — standard Rust pattern for user-friendly file paths

### Constraints from Existing Code

- **Return type unchanged:** `load()` must continue returning `Result<BacklogFile, String>`
- **Multiple exit points in `load()`:** The migration path returns early on line 49 (`return Ok(backlog)`), while the normal v3 path returns on line 65. Validation needs to cover both paths.
- **Config loading behavior:** `load_config()` returns defaults when config file is missing (not an error). It only errors on parse failures of an existing file.
- **Log-level filtering:** `log_warn!` is subject to log-level configuration; warning only displays if level is Warn or higher

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Validation goes "after successful parse and after migrations" at a single point | `load()` has two exit points: line 49 (migration path) and line 65 (normal v3 path) | Either add validation before both return points, or refactor to a single exit point. A helper function or a shared validation block before each `Ok(backlog)` return would work. |
| `load_config()` call is straightforward | `load_config()` returns defaults on missing file but errors on malformed TOML | Using `.ok()` handles both cases; the "skip gracefully" behavior works as designed |

---

## Critical Areas

### Dual Return Paths in `load()`

**Why it's critical:** The `load()` function returns `Ok(backlog)` from two different places — line 49 (after v2→v3 migration) and line 65 (direct v3 load). If the validation is only added before one return, migrated backlogs would skip the check.

**Why it's easy to miss:** The PRD describes adding validation "in `backlog::load()`" as if there's a single place, but the migration early-return is easy to overlook.

**What to watch for:** The design/spec should explicitly handle both paths. Options: (a) add validation before each `Ok(backlog)` return, (b) extract a validation helper called from both paths, or (c) restructure to a single exit point. Option (b) is cleanest.

---

## Deep Dives

_No deep dives needed — light mode research with clear answers._

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| Should validation also run after migration? | Migrated files might have inconsistent `next_item_id` | Yes — the validation is valuable regardless of how the file was loaded. Cover both return paths. |

### Recommended Approaches

#### Validation Placement

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Extract validation helper, call before each `Ok(backlog)` return | Clean, DRY, testable | Slightly more code | Always — this is the right approach |
| Add inline validation before each return | Simple, no new function | Duplicated logic | Only if you want minimal diff |
| Restructure `load()` to single exit point | Single validation point | Larger refactor, out of scope | Not recommended for this change |

**Initial recommendation:** Extract a small helper function (e.g., `warn_if_next_id_behind()`) that takes `&BacklogFile`, `&Path`, and `&str` (prefix). Call it before each `Ok(backlog)` return. This keeps the validation logic DRY, testable, and doesn't require restructuring `load()`.

#### Config Loading for Prefix

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Call `load_config(project_root).ok()` at each validation point | Simple, matches PRD | Config loaded twice if migration also loaded it | For v3 direct loads (most common case) |
| Load config once, pass to validation helper | Efficient | Slightly more restructuring | If perf matters (it doesn't here) |

**Initial recommendation:** Call `load_config(project_root).ok()` and pass the prefix to the validation helper. The config TOML parse overhead is negligible.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [serde #642](https://github.com/serde-rs/serde/issues/642) | Issue | Confirms post-deserialization validation is the standard approach |
| [rust-analyzer #11950](https://github.com/rust-lang/rust-analyzer/issues/11950) | Issue | Real-world validation pattern in major Rust project |
| `src/backlog.rs:108-125` | Code | ID parsing logic to reuse |
| `src/backlog.rs:23-66` | Code | `load()` function with dual return paths |
| `src/log.rs:49-56` | Code | `log_warn!` macro definition |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Initial light research (external + internal) | Confirmed inline post-deserialization validation is standard; identified dual return paths as key design consideration |
