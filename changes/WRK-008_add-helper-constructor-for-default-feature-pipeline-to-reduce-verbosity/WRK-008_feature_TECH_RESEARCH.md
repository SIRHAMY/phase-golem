# Tech Research: Add PhaseConfig helper constructor to reduce verbosity

**ID:** WRK-008
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-008_add-helper-constructor-for-default-feature-pipeline-to-reduce-verbosity_PRD.md
**Mode:** Light

## Overview

Researching idiomatic Rust patterns for adding a `PhaseConfig::new(name, is_destructive)` constructor with sensible defaults, and using struct update syntax at call sites that need field overrides. The change is straightforward — the main questions are confirming the approach is idiomatic and identifying any gotchas with struct update syntax or serde alignment.

## Research Questions

- [x] Is `new()` + struct update syntax the idiomatic Rust pattern for this use case?
- [x] Are there ownership/move gotchas with struct update syntax on non-Copy fields?
- [x] Does `PhaseConfig` derive the traits needed for struct update syntax?
- [x] Do existing `#[serde(default)]` attributes align with proposed constructor defaults?

---

## External Research

### Landscape Overview

Rust has no built-in constructor syntax. The idiomatic patterns for struct construction with defaults are:

1. **`new()` constructor** — associated function taking required params, defaulting the rest
2. **`Default` trait** — zero-argument construction; not suitable when required fields exist
3. **Struct update syntax** (`..source`) — override specific fields from a base instance
4. **Builder pattern** — step-by-step construction with validation; overkill for simple structs

For `PhaseConfig` (4 fields, 2 required, 2 with sensible defaults), pattern #1 + #3 is the standard approach.

### Common Patterns & Approaches

#### Pattern: Constructor + Struct Update Syntax

**How it works:** `new()` takes required params, returns fully-initialized struct. Call sites override specific fields via `PhaseConfig { field: value, ..PhaseConfig::new(...) }`.

**When to use:** Struct has fields with sensible defaults but also required fields with no meaningful default.

**Tradeoffs:**
- Pro: Minimal API surface, idiomatic, no dependencies
- Con: Struct update syntax moves non-Copy fields from source (mitigated by using inline `new()` calls)

**References:**
- [Constructor - Rust Design Patterns](https://rust-unofficial.github.io/patterns/idioms/ctor.html) — idiomatic constructor pattern
- [Rust API Guidelines: Predictability](https://rust-lang.github.io/api-guidelines/predictability.html) — `new` naming convention

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Struct update syntax moves non-Copy fields | Source instance becomes partially moved, causing borrow errors | Use inline `..PhaseConfig::new(...)` not a named variable |
| Constructor defaults diverge from serde defaults | Programmatic vs deserialized configs behave differently | Verify constructor defaults match `#[serde(default)]` attrs |
| `..source` not in last position | Syntax error | Always place `..source` as final element in struct literal |

### Key Learnings

- The PRD's proposed pattern (`new()` + struct update syntax) is the textbook idiomatic Rust approach for this case
- No `Default` impl needed (and PRD correctly excludes it — `name` has no sensible default)
- Builder pattern and `derive-new` crate are overkill for 4 fields

---

## Internal Research

### Existing Codebase State

**PhaseConfig struct** (`orchestrator/src/config.rs:50-59`):
```rust
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct PhaseConfig {
    pub name: String,
    #[serde(default)]
    pub workflows: Vec<String>,
    pub is_destructive: bool,
    #[serde(default)]
    pub staleness: StalenessAction,
}
```

**StalenessAction enum** (`orchestrator/src/config.rs:41-48`):
```rust
#[derive(Default, Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum StalenessAction {
    #[default]
    Ignore,
    Warn,
    Block,
}
```

**Key trait derives confirmed:**
- `PhaseConfig`: `Clone` — enables struct update syntax
- `StalenessAction`: `Clone`, `Default` (with `Ignore` as default) — aligns with constructor default

**No existing impl block** for PhaseConfig — the constructor will be the first.

**Relevant files/modules:**
- `orchestrator/src/config.rs:50-59` — PhaseConfig struct definition (constructor goes here)
- `orchestrator/src/config.rs:98-145` — `default_feature_pipeline()` (47 lines, 8 phases)
- `orchestrator/tests/config_test.rs` — 10 PhaseConfig construction sites
- `orchestrator/tests/executor_test.rs` — 10 sites (4 with non-default staleness at lines 376, 430, 489, 546)
- `orchestrator/tests/migration_test.rs` — 5 construction sites
- `orchestrator/tests/preflight_test.rs` — 14 construction sites
- `orchestrator/tests/prompt_test.rs` — `make_phase_config` helper (lines 12-19) + 4 call sites
- `orchestrator/tests/scheduler_test.rs` — 2 construction sites

### Existing Patterns

- **Default impls** for `ProjectConfig`, `GuardrailsConfig`, `ExecutionConfig` exist in `config.rs:68-96` — but these have meaningful defaults for all fields. `PhaseConfig` correctly does NOT get a `Default` impl.
- **`make_phase_config` test helper** (`prompt_test.rs:12-19`) — takes `(name, workflows)`, hardcodes `is_destructive: false` and `staleness: Ignore`. This becomes redundant with `PhaseConfig::new()` and should be removed per PRD.

### Reusable Components

- `StalenessAction::Ignore` as default — already established via `#[default]` attribute
- `PhaseConfig` derives `Clone` — no changes needed to support struct update syntax

### Constraints

- **Serde alignment**: Constructor defaults (`workflows: vec![]`, `staleness: StalenessAction::Ignore`) must match `#[serde(default)]` field defaults. Both are already aligned — `Vec<String>` defaults to empty vec, `StalenessAction` defaults to `Ignore`.
- **No additional derives needed**: `PhaseConfig` already has `Clone`, `StalenessAction` already has `Clone` + `Default`.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| "default workflows (1 line)" example suggests common 1-line usage | All 8 production phases in `default_feature_pipeline()` have non-empty workflows | The 1-line form (`PhaseConfig::new("build", true)`) is primarily useful in tests. Production code will always use struct update syntax for workflow overrides. Not a problem — tests benefit most from the 1-line form. |

No other concerns. The PRD approach is well-aligned with idiomatic Rust patterns.

---

## Critical Areas

### Serde Default Alignment

**Why it's critical:** If constructor defaults diverge from `#[serde(default)]` values, programmatic vs deserialized configs behave differently.

**Why it's easy to miss:** The two default sources are in different locations (constructor body vs derive attributes) and could drift when fields are added.

**What to watch for:** When adding the constructor, verify defaults match. Consider a code comment noting the alignment requirement. Future `PhaseConfig` fields with `#[serde(default)]` should have their defaults replicated in the constructor.

---

## Deep Dives

_No deep dives needed — light mode research with straightforward findings._

---

## Synthesis

### Open Questions

_None — all research questions resolved. The approach is well-understood and straightforward._

### Recommended Approaches

#### Constructor API Design

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `PhaseConfig::new(name: &str, is_destructive: bool)` + struct update syntax | Minimal API, idiomatic, matches serde defaults | Overrides require struct update syntax (4 lines) | Required fields are few, defaults are stable (this case) |
| Builder pattern (`.with_workflows()`, etc.) | Fluent API, no struct update syntax needed | Additional API surface, more code, explicitly out of scope | Many optional fields, complex validation |
| `Default` trait + struct update syntax | Standard trait, zero-argument | `name` has no meaningful default, creates invalid instances | All fields have sensible defaults (not this case) |

**Initial recommendation:** `PhaseConfig::new(name, is_destructive)` — exactly as the PRD proposes. It's idiomatic, minimal, and sufficient for the use case.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Constructor - Rust Design Patterns](https://rust-unofficial.github.io/patterns/idioms/ctor.html) | Guide | Idiomatic constructor pattern reference |
| [Rust API Guidelines: Predictability](https://rust-lang.github.io/api-guidelines/predictability.html) | Standard | API naming conventions (C-CTOR) |
| [The Default Trait - Rust Design Patterns](https://rust-unofficial.github.io/patterns/idioms/default.html) | Guide | Why Default doesn't fit here |
| [Using derive - Serde](https://serde.rs/derive.html) | Docs | `#[serde(default)]` behavior reference |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Light external research: Rust constructor patterns | Confirmed `new()` + struct update syntax is idiomatic; no alternatives needed |
| 2026-02-13 | Light internal research: PhaseConfig and construction sites | Mapped all 45 construction sites across 7 files; confirmed trait derives support the pattern |
| 2026-02-13 | PRD analysis | No significant concerns; approach is well-aligned with findings |
