# Design: Add PhaseConfig helper constructor to reduce verbosity

**ID:** WRK-008
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-008_add-helper-constructor-for-default-feature-pipeline-to-reduce-verbosity_PRD.md
**Tech Research:** ./WRK-008_feature_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add a `PhaseConfig::new(name, is_destructive)` associated function that defaults `workflows` to `vec![]` and `staleness` to `StalenessAction::Ignore`. Then update all ~45 construction sites (7 in production across 1 pre-phase + 6 main phases, ~38 in tests across 6 files) to use it, with struct update syntax for field overrides. This is the textbook idiomatic Rust pattern for structs with required + defaultable fields.

---

## System Design

### High-Level Architecture

This is a localized refactor — no new components, modules, or data flows. The change introduces a single `impl PhaseConfig` block with one `pub fn new()` constructor, then mechanically updates existing construction sites to use it.

**Components affected:**
1. `PhaseConfig` struct definition (`config.rs`) — gains an `impl` block with `new()`
2. `default_feature_pipeline()` (`config.rs`) — construction sites updated
3. Test files (6 files) — inline `PhaseConfig { ... }` literals updated; `make_phase_config` helper in `prompt_test.rs` removed

### Component Breakdown

#### PhaseConfig::new() Constructor

**Purpose:** Provide a concise way to construct `PhaseConfig` with sensible defaults for the two fields that are almost always boilerplate (`workflows`, `staleness`).

**Signature:**
```rust
impl PhaseConfig {
    /// Construct a PhaseConfig with sensible defaults for workflows and staleness.
    ///
    /// Defaults: `workflows` = `vec![]`, `staleness` = `StalenessAction::Ignore`.
    /// These MUST match the `#[serde(default)]` field attributes on the struct
    /// to keep programmatic and deserialized configs consistent.
    ///
    /// Always use inline in struct update syntax (`..PhaseConfig::new(...)`)
    /// rather than storing in a named variable, to avoid partial-move errors
    /// on non-Copy fields.
    pub fn new(name: &str, is_destructive: bool) -> Self {
        Self {
            name: name.to_string(),
            workflows: vec![],
            is_destructive,
            staleness: StalenessAction::Ignore,
        }
    }
}
```

**Interfaces:**
- Input: `name: &str` (phase name), `is_destructive: bool`
- Output: Fully-initialized `PhaseConfig` with empty workflows and `Ignore` staleness

**Dependencies:** `StalenessAction` (same module)

### Data Flow

No change to runtime data flow. This is a construction-time convenience — the resulting `PhaseConfig` values are identical to what the current struct literals produce.

### Key Flows

#### Flow: Construct PhaseConfig with default workflows and staleness

> Most common case — used by most test sites and the `research` pre-phase in production.

1. **Call `PhaseConfig::new("phase_name", false)`** — returns a complete PhaseConfig with empty workflows and `StalenessAction::Ignore`

#### Flow: Construct PhaseConfig with non-default workflows

> Used by all production phases in `default_feature_pipeline()` and some test sites.

1. **Use struct update syntax** — `PhaseConfig { workflows: vec![...], ..PhaseConfig::new("name", false) }`
2. The `workflows` field is overridden; `staleness` still defaults to `Ignore`

#### Flow: Construct PhaseConfig with non-default staleness

> Used by 4 test sites in `executor_test.rs` that test `StalenessAction::Warn` and `Block`.

1. **Use struct update syntax** — `PhaseConfig { staleness: StalenessAction::Block, ..PhaseConfig::new("name", true) }`
2. The `staleness` field is overridden; `workflows` still defaults to empty

**Edge cases:**
- Overriding both `workflows` and `staleness` — struct update syntax supports multiple field overrides in the same expression. No special handling needed.

---

## Technical Decisions

### Key Decisions

#### Decision: Constructor takes `&str` not `String` for name

**Context:** The `name` field is `String`, so the constructor could accept either `&str` or `String`.

**Decision:** Accept `&str` and call `.to_string()` internally.

**Rationale:** Every call site currently uses string literals (`"build"`, `"prd"`, etc.). Accepting `&str` avoids requiring `.to_string()` at every call site, matching the existing `make_phase_config` helper in `prompt_test.rs` which also takes `&str`.

**Consequences:** Call sites passing owned `String` values would need `.as_str()` or `&s`. This is fine — no current call site passes an owned `String`.

#### Decision: Place constructor before `default_feature_pipeline()`

**Context:** The new `impl PhaseConfig` block needs a location in `config.rs`.

**Decision:** Place it immediately after the `PhaseConfig` struct definition (after line 59), before the `PipelineConfig` struct and the `Default` impls. This keeps the constructor visually close to the struct it belongs to.

**Rationale:** Rust convention places `impl` blocks immediately after the struct definition. The existing `Default` impls for other structs (lines 68-96) follow this pattern.

**Consequences:** `default_feature_pipeline()` can reference `PhaseConfig::new()` since it comes later in the file.

#### Decision: Remove `make_phase_config` helper from `prompt_test.rs`

**Context:** `prompt_test.rs` has a `make_phase_config(name, workflows)` helper that creates `PhaseConfig` with `is_destructive: false` and `staleness: Ignore`.

**Decision:** Remove this helper and replace call sites with `PhaseConfig::new()` plus struct update syntax for workflow overrides.

**Rationale:** `PhaseConfig::new()` subsumes this helper's functionality. Keeping both creates redundancy and confusion about which to use. The helper hardcodes `is_destructive: false`, which is the same as what all its call sites need, and `PhaseConfig::new()` accepts this as a parameter.

**Consequences:** Call sites that previously used `make_phase_config("name", vec!["workflow"])` become `PhaseConfig { workflows: vec!["workflow".to_string()], ..PhaseConfig::new("name", false) }`. This is slightly more verbose for the workflow-override case, but eliminates a test-only abstraction in favor of the canonical production API. The `default_prd_config()` helper in the same file (which calls `make_phase_config`) will also be updated.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Workflow-override verbosity | Struct update syntax for workflows (4 lines) is longer than `make_phase_config` (1 line); ~10 call sites in `prompt_test.rs` expand from 1 line to 4 | Single canonical construction API; no test-only helper to maintain | The majority of test sites (~28 of ~38) use default (empty) workflows and shrink from 5 lines to 1 with `PhaseConfig::new()`. The ~10 that need workflows expand slightly but are explicit about what they're overriding. Net reduction in total test code. |
| Constructor defaults could drift from serde defaults | Constructor body and `#[serde(default)]` attributes are separate default sources | Constructor convenience for programmatic construction | Both currently align (`vec![]` and `StalenessAction::Ignore`). A doc comment on the constructor explicitly documents the alignment requirement (see constructor signature). |

---

## Alternatives Considered

### Alternative: Builder pattern (`.with_workflows()`, `.with_staleness()`)

**Summary:** Method chaining API for constructing `PhaseConfig`.

**How it would work:**
- `PhaseConfig::new("build", true).with_workflows(vec!["..."]).build()`
- Each optional field gets a `.with_*()` method

**Pros:**
- Fluent API, no struct update syntax needed
- Workflows override is a single method call

**Cons:**
- Adds 2+ methods to the public API surface
- More code to maintain for a 4-field struct
- Explicitly out of scope per PRD
- Over-engineered for current needs

**Why not chosen:** Struct update syntax is idiomatic Rust and sufficient. Builder adds unnecessary API surface for a simple struct.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Constructor defaults drift from serde defaults when new fields added | Programmatic vs deserialized configs behave differently | Low | Doc comment on constructor explicitly states alignment requirement (see constructor signature above); both default sources are in the same file. When adding new `PhaseConfig` fields with `#[serde(default)]`, update the constructor body to match. |

---

## Integration Points

### Existing Code Touchpoints

- `.claude/skills/changes/orchestrator/src/config.rs` — Add `impl PhaseConfig` block with `new()` constructor; update `default_feature_pipeline()` to use it
- `.claude/skills/changes/orchestrator/tests/config_test.rs` — Update ~10 construction sites
- `.claude/skills/changes/orchestrator/tests/executor_test.rs` — Update ~10 construction sites (4 with non-default staleness use struct update syntax)
- `.claude/skills/changes/orchestrator/tests/migration_test.rs` — Update ~5 construction sites
- `.claude/skills/changes/orchestrator/tests/preflight_test.rs` — Update ~14 construction sites
- `.claude/skills/changes/orchestrator/tests/prompt_test.rs` — Remove `make_phase_config` helper (lines 12-19) and its ~10 call sites; update `default_prd_config()` to use `PhaseConfig::new()` with struct update syntax for workflow overrides
- `.claude/skills/changes/orchestrator/tests/scheduler_test.rs` — Update ~2 construction sites

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
- [x] No major open questions remain (or they're flagged for spec phase)

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Initial design draft | Straightforward constructor + struct update syntax approach; one alternative (builder) briefly noted and rejected |
| 2026-02-13 | Self-critique (7 agents) | Auto-fixed: specified serde alignment doc comment text, fixed phase count, clarified prompt_test.rs migration scope, quantified verbosity tradeoff. No directional issues. No critical or high issues remain. |
