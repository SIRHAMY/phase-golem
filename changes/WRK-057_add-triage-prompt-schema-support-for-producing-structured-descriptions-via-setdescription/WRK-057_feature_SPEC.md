# SPEC: Add Triage Prompt Schema Support for Structured Descriptions

**ID:** WRK-057
**Status:** Ready
**Created:** 2026-02-20
**PRD:** ./WRK-057_triage-prompt-structured-description_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

WRK-028 introduced `StructuredDescription`, `ItemUpdate::SetDescription`, coordinator handling, and `render_structured_description` — all the infrastructure needed for structured descriptions on backlog items. The triage pipeline is the natural first consumer: when an agent triages a new item, it should produce a structured description summarizing the work. This lets all downstream phase agents see context/problem/solution/impact/sizing_rationale in their prompts without separate per-phase prompt work.

The gap: `PhaseResult` has no `description` field, the triage prompt schema doesn't include a description object, and `apply_triage_result` doesn't call `SetDescription`. This SPEC closes that gap.

## Approach

Wire the existing `StructuredDescription` infrastructure into the triage pipeline through four touchpoints:

1. **Data model** — Add `description: Option<StructuredDescription>` to `PhaseResult` (carries description from agent JSON to scheduler)
2. **Prompt schema** — Add the description object to `build_triage_output_suffix` JSON example and add a step to triage instructions asking the agent to produce it
3. **Application logic** — In `apply_triage_result`, apply `SetDescription` when description is present and non-empty
4. **Empty detection** — Add `is_empty()` method to `StructuredDescription` to detect `description: {}` → `Some(empty_struct)` deserialization edge case

All four changes are additive. No existing behavior changes. No function signatures change.

**Patterns to follow:**

- `src/types.rs:245-256` — Existing `PhaseResult` optional fields use `#[serde(default, skip_serializing_if = "Option::is_none")]`; follow same pattern for `description`
- `src/scheduler.rs:1630-1634` — Assessment application pattern in `apply_triage_result`; description application follows the same `if let Some(ref ...) { coordinator.update_item(...).await?; }` pattern
- `src/prompt.rs:451-467` — `render_structured_description` already filters empty fields; `is_empty()` mirrors this logic for the entire struct

**Implementation boundaries:**

- Do not modify: `StructuredDescription` struct definition (stable from WRK-028)
- Do not modify: `ItemUpdate::SetDescription` variant or coordinator handler (already complete from WRK-028)
- Do not modify: `render_structured_description` or `build_preamble` (already consume descriptions correctly)
- Do not refactor: existing triage prompt structure beyond adding the new instruction step and schema field

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Type Changes & Test Fixtures | Low | Add `description` field to `PhaseResult`, add `is_empty()` to `StructuredDescription`, update all test construction sites |
| 2 | Triage Prompt & Application Logic | Low | Update triage prompt schema/instructions, apply description in `apply_triage_result`, add new tests |

**Ordering rationale:** Phase 1 must come first because adding a field to `PhaseResult` breaks compilation at all construction sites. Phase 2 depends on the field existing and `is_empty()` being available.

---

## Phases

Each phase should leave the codebase in a functional, stable state. Complete and verify each phase before moving to the next.

---

### Phase 1: Type Changes & Test Fixtures

> Add `description` field to `PhaseResult`, add `is_empty()` to `StructuredDescription`, update all test construction sites

**Phase Status:** complete

**Complexity:** Low

**Goal:** Extend `PhaseResult` with an optional description field and add the `is_empty()` helper method, then mechanically update all ~30 test construction sites to include `description: None` so the codebase compiles and all existing tests pass.

**Files:**

- `src/types.rs` — modify — Add `description: Option<StructuredDescription>` field to `PhaseResult` struct; add `impl StructuredDescription { pub fn is_empty(&self) -> bool }` method
- `tests/types_test.rs` — modify — Add `description: None` to ~8 `PhaseResult` struct literals; add tests for `StructuredDescription::is_empty()` and `PhaseResult` serialization/deserialization with description
- `tests/scheduler_test.rs` — modify — Add `description: None` to ~5 test helper functions (`phase_complete_result`, `failed_result`, `blocked_result`, `subphase_complete_result`, `triage_result_with_assessments`)
- `tests/executor_test.rs` — modify — Add `description: None` to ~6 `PhaseResult` struct literals (helpers and test bodies)
- `tests/agent_test.rs` — modify — Add `description: None` to ~2 `PhaseResult` struct literals (`valid_result_json`, `make_result`)
- `tests/coordinator_test.rs` — modify — Add `description: None` to ~4 `PhaseResult` struct literals

**Patterns:**

- Follow existing `PhaseResult` optional field pattern: `#[serde(default, skip_serializing_if = "Option::is_none")]`
- The `is_empty()` method mirrors the per-field empty checks in `render_structured_description` (`src/prompt.rs:451-467`)

**Tasks:**

- [x] Add `#[serde(default, skip_serializing_if = "Option::is_none")] pub description: Option<StructuredDescription>` to `PhaseResult` in `src/types.rs` after the `duplicates` field
- [x] Add `impl StructuredDescription { pub fn is_empty(&self) -> bool { ... } }` method after the `StructuredDescription` struct definition in `src/types.rs`, checking all five fields
- [x] Update all `PhaseResult` struct literals in `tests/scheduler_test.rs` (~5 helpers) to include `description: None`
- [x] Update all `PhaseResult` struct literals in `tests/types_test.rs` (~8 sites) to include `description: None`
- [x] Update all `PhaseResult` struct literals in `tests/executor_test.rs` (~6 sites) to include `description: None`
- [x] Update all `PhaseResult` struct literals in `tests/agent_test.rs` (~2 sites) to include `description: None`
- [x] Update all `PhaseResult` struct literals in `tests/coordinator_test.rs` (~4 sites) to include `description: None`
- [x] Add test: `StructuredDescription::is_empty()` returns `true` for `StructuredDescription::default()`
- [x] Add test: `StructuredDescription::is_empty()` returns `false` when any single field is non-empty (test each field)
- [x] Add test: `PhaseResult` with `description: Some(StructuredDescription { context: "...", ... })` serializes/deserializes correctly (round-trip)
- [x] Add test: `PhaseResult` JSON without `description` field deserializes to `description: None`
- [x] Add test: `PhaseResult` JSON with `"description": null` deserializes to `description: None`
- [x] Add test: `PhaseResult` JSON with `"description": {}` (empty object) deserializes to `description: Some(StructuredDescription::default())` and `is_empty()` returns `true`

**Verification:**

- [x] `cargo build` succeeds with no warnings related to the change
- [x] `cargo test` passes — all existing tests pass with the new field
- [x] New `is_empty()` tests pass
- [x] New `PhaseResult` description serialization tests pass

**Commit:** `[WRK-057][P1] Feature: Add description field to PhaseResult and is_empty() to StructuredDescription`

**Notes:**

- The compiler will enforce exhaustive struct construction, so any missed `PhaseResult` literal will be a compile error, not a runtime bug.
- `StructuredDescription` already derives `Default`, `PartialEq`, `Serialize`, `Deserialize` — no derive changes needed.
- Phase 1 is independently valid: if Phase 2 is deferred, all tests pass with `description: None` and the `is_empty()` method is available for future use.

**Followups:**

---

### Phase 2: Triage Prompt & Application Logic

> Update triage prompt schema/instructions and apply description in `apply_triage_result`, with tests

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Update the triage prompt to ask agents for a structured description and include the description schema in the JSON example. Add logic in `apply_triage_result` to call `SetDescription` when a non-empty description is provided. Add tests for all new behavior.

**Files:**

- `src/prompt.rs` — modify — Add description instruction step 5 (renumber existing 5-6 to 6-7) in `build_triage_prompt`; add `description` object to JSON schema in `build_triage_output_suffix`
- `src/scheduler.rs` — modify — Add description application block in `apply_triage_result` after assessments (line ~1634) and before pipeline_type validation (line ~1637)
- `tests/prompt_test.rs` — modify — Add tests: triage prompt includes description instructions, triage output schema includes description field
- `tests/scheduler_test.rs` — modify — Add tests: description applied from triage result, empty description not applied, partial description applied

**Patterns:**

- Follow the assessment application pattern in `apply_triage_result` (`if let Some(ref ...) { coordinator.update_item(...).await?; }`)
- Follow existing triage prompt test patterns in `tests/prompt_test.rs` (e.g., `triage_prompt_contains_assessment_instructions`)

**Tasks:**

- [ ] In `build_triage_prompt` (`src/prompt.rs`), insert new instruction step 5 and renumber subsequent steps. Final numbering should be: 1 (read item), 2 (check duplicates), 3 (classify pipeline), 4 (assess dimensions), **5 (write structured description)**, 6 (decide routing), 7 (report assessment). Step 5 instructs the agent to produce a structured description with per-field guidance: `context` (background/origin of the work item), `problem` (what issue this addresses), `solution` (proposed approach), `impact` (expected benefit), `sizing_rationale` (reasoning behind size/complexity assessment)
- [ ] In `build_triage_output_suffix` (`src/prompt.rs`), add `description` object to the JSON schema example after `duplicates`, with five string sub-fields annotated with one-line purpose explanations. Mark as optional.
- [ ] In `apply_triage_result` (`src/scheduler.rs`), after the assessment update block (line ~1634) and before the pipeline_type block (line ~1637), add: `if let Some(ref description) = result.description { if !description.is_empty() { coordinator.update_item(item_id, ItemUpdate::SetDescription(description.clone())).await?; } }`
- [ ] Add test: triage prompt output contains description-related instruction text (e.g., "structured description")
- [ ] Add test: triage output schema contains `"description"` field with sub-fields (`context`, `problem`, `solution`, `impact`, `sizing_rationale`)
- [ ] Add test: `apply_triage_result` with a `PhaseResult` containing a non-empty description calls `SetDescription` on the item (verify item's description is set via coordinator snapshot)
- [ ] Add test: `apply_triage_result` with a `PhaseResult` where `description` is `None` does not set a description
- [ ] Add test: `apply_triage_result` with a `PhaseResult` where `description` is `Some(StructuredDescription::default())` (all-empty) does not set a description
- [ ] Add test: `apply_triage_result` with a partial description (e.g., only `context` and `problem` populated, other fields empty strings) applies the description — verify `is_empty()` returns `false` and `SetDescription` is called
- [ ] Add test: `apply_triage_result` error propagation — when `SetDescription` coordinator call fails, error is propagated via `?` (not silently swallowed), matching the assessment update error handling pattern

**Verification:**

- [ ] `cargo build` succeeds
- [ ] `cargo test` passes — all existing and new tests pass
- [ ] Triage prompt instructions are numbered 1-7 with no gaps (step 5 is description)
- [ ] Triage output schema includes `"description"` object with all five sub-fields
- [ ] Description is applied to items via `SetDescription` when `result.description` is `Some` and `is_empty()` returns `false`
- [ ] `description: None`, `description: null` (in JSON), and all-empty description do not trigger `SetDescription`
- [ ] Code review passes

**Commit:** `[WRK-057][P2] Feature: Wire triage prompt and apply_triage_result for structured descriptions`

**Notes:**

- The `SetDescription` coordinator handler is already implemented (coordinator.rs:398-401) — no changes needed there.
- Both the scheduler path (`handle_triage_success` → `apply_triage_result`) and the CLI path (`main.rs` `handle_triage` → `apply_triage_result`) call `apply_triage_result`, so both are covered by this single change.
- Description application before pipeline_type validation ensures descriptions persist even for blocked/failed triage outcomes, consistent with assessment behavior.

**Followups:**

---

## Final Verification

- [ ] All phases complete
- [ ] All PRD success criteria met:
  - [ ] `PhaseResult` has `description: Option<StructuredDescription>` with correct serde annotations
  - [ ] `build_triage_output_suffix` includes description object in JSON schema with per-field annotations
  - [ ] `apply_triage_result` applies `SetDescription` for non-empty descriptions, placed before pipeline_type validation
  - [ ] Triage prompt instructions tell the agent to produce a structured description
  - [ ] All existing tests pass with `description: None` added to construction sites
  - [ ] New tests verify: deserialization, application via coordinator, prompt schema inclusion, all-empty skipped
- [ ] Tests pass
- [ ] No regressions introduced
- [ ] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | complete | `[WRK-057][P1]` | 25 test construction sites updated, 10 new tests added, code review clean |

## Followups Summary

### Critical

### High

### Medium

### Low

## Design Details

### Key Types

```rust
// Addition to PhaseResult (src/types.rs:240-259)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PhaseResult {
    // ... existing fields ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<StructuredDescription>,
}

// Addition to StructuredDescription (src/types.rs, after line 273)
impl StructuredDescription {
    pub fn is_empty(&self) -> bool {
        self.context.is_empty()
            && self.problem.is_empty()
            && self.solution.is_empty()
            && self.impact.is_empty()
            && self.sizing_rationale.is_empty()
    }
}
```

### Design Rationale

- **Two phases instead of one:** While the total change is small, Phase 1 (type + fixture updates) is purely mechanical and must compile before Phase 2 logic can be written. Separating them gives a clean compile checkpoint.
- **`is_empty()` over `== default()`:** More idiomatic Rust, more readable, mirrors existing `render_structured_description` logic. Low maintenance burden for a stable five-field struct.
- **Description before pipeline_type in `apply_triage_result`:** Consistent with how assessments are applied unconditionally before routing decisions. Descriptions are useful context regardless of triage outcome.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
