# Design: Triage Prompt Structured Description Support

**ID:** WRK-057
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-057_triage-prompt-structured-description_PRD.md
**Tech Research:** ./WRK-057_feature_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add an optional `description` field to `PhaseResult` so triage agents can produce a `StructuredDescription` in their JSON output. Update the triage prompt schema and instructions to request the description, and apply it via `SetDescription` in `apply_triage_result`. All infrastructure (`StructuredDescription`, `ItemUpdate::SetDescription`, coordinator handler, `render_structured_description`) already exists from WRK-028 — this change wires the triage prompt to produce and persist structured descriptions.

---

## System Design

### High-Level Architecture

No new components. Four existing touchpoints are modified:

1. **`PhaseResult`** (data model) — gains an optional `description` field
2. **`build_triage_output_suffix`** (prompt schema) — includes description in JSON schema
3. **`build_triage_prompt`** (prompt instructions) — tells triage agent to produce a description
4. **`apply_triage_result`** (scheduler logic) — applies `SetDescription` when description is present

Plus one addition to the `StructuredDescription` type:

5. **`StructuredDescription::is_empty()`** — method to detect all-empty descriptions

### Component Breakdown

#### PhaseResult (types.rs)

**Purpose:** Add `description: Option<StructuredDescription>` field

**Responsibilities:**
- Carry an optional structured description from agent JSON output to the scheduler
- Deserialize absent/null description as `None` via `#[serde(default, skip_serializing_if = "Option::is_none")]` (same pattern as all other optional `PhaseResult` fields)

**Interfaces:**
- Input: Agent-produced JSON with optional `description` object
- Output: Rust struct with `Option<StructuredDescription>`

**Dependencies:** `StructuredDescription` (already in types.rs)

#### StructuredDescription::is_empty() (types.rs)

**Purpose:** Check if all five fields are empty strings

**Responsibilities:**
- Return `true` when all fields (`context`, `problem`, `solution`, `impact`, `sizing_rationale`) are empty strings
- Used to skip `SetDescription` for blank descriptions (e.g., `description: {}` deserializing as `Some(empty_struct)`)

**Interfaces:**
- Input: `&self`
- Output: `bool`

**Dependencies:** None

#### Triage Prompt Schema (prompt.rs — `build_triage_output_suffix`)

**Purpose:** Add description object to the JSON schema example shown to triage agents

**Responsibilities:**
- Include a `description` object with five string sub-fields in the JSON schema
- Annotate each sub-field with a one-line purpose explanation
- Mark the field as optional

**Interfaces:**
- Input: item_id, result_path (unchanged)
- Output: String containing the updated JSON schema

**Dependencies:** None

#### Triage Prompt Instructions (prompt.rs — `build_triage_prompt`)

**Purpose:** Tell the triage agent to produce a structured description

**Responsibilities:**
- Add instruction step 5 (after assessment step 4, before routing step 5→6) asking the agent to summarize the work item as a structured description with per-field guidance:
  - `context`: background and origin of this work item
  - `problem`: what issue this addresses
  - `solution`: proposed approach
  - `impact`: expected benefit
  - `sizing_rationale`: why the size/complexity assessment was chosen

**Interfaces:**
- Input: Unchanged
- Output: Updated prompt string

**Dependencies:** None

#### Description Application (scheduler.rs — `apply_triage_result`)

**Purpose:** Apply `SetDescription` when triage result includes a non-empty description

**Responsibilities:**
- Check if `result.description` is `Some` and not empty (via `is_empty()`)
- Call `coordinator.update_item(item_id, ItemUpdate::SetDescription(...))` after assessments (line 1634) and before pipeline_type validation (line 1637), using the same `?` error propagation pattern as assessment updates
- Skip silently if description is `None` or all-empty

**Interfaces:**
- Input: `PhaseResult` with optional description (unchanged signature)
- Output: `Result<(), String>` (unchanged)

**Dependencies:** `ItemUpdate::SetDescription`, `StructuredDescription::is_empty()`

### Data Flow

1. Triage agent receives prompt with description schema in `## Structured Output` section
2. Agent writes JSON result file with optional `description` object containing 1-5 fields
3. Scheduler reads JSON, deserializes into `PhaseResult` with `description: Option<StructuredDescription>`
4. `apply_triage_result` checks if description is present and non-empty
5. If yes, calls `coordinator.update_item(item_id, ItemUpdate::SetDescription(desc))`
6. Coordinator sets `item.description = Some(description)` on the `BacklogItem`
7. Downstream phase prompts automatically see the description via `build_preamble()` → `render_structured_description()`

### Key Flows

#### Flow: Triage Agent Produces Description

> Triage agent produces a structured description and it gets persisted on the backlog item.

1. **Agent runs** — Reads item, assesses it, writes JSON with `description` object
2. **Scheduler reads result** — `PhaseResult` deserializes with `description: Some(StructuredDescription { context: "...", problem: "...", ... })`
3. **apply_triage_result** — Checks `is_empty()` → false → calls `SetDescription`
4. **Coordinator persists** — `item.description = Some(desc)` in BACKLOG.yaml
5. **Downstream phases** — `build_preamble()` renders description via `render_structured_description()`

**Edge cases:**
- Agent omits description entirely → `description: None` → no `SetDescription` call
- Agent produces `description: {}` → `description: Some(empty_struct)` → `is_empty()` returns true → no `SetDescription` call
- Agent produces partial description (e.g., only `context` field) → missing fields default to `""` via `#[serde(default)]` on `StructuredDescription` → `is_empty()` returns false → applied; `render_structured_description` skips empty fields when rendering
- Agent produces `description: "some string"` (wrong type) → serde deserialization of `PhaseResult` fails, handled by existing error path in scheduler
- Item already has a description from a previous triage → `SetDescription` overwrites it (consistent with how assessments are overwritten on re-triage)
- Item is merged during duplicate detection → `handle_triage_success` returns early before `apply_triage_result` is called, so the description is not applied (correct — the merged item ceases to exist)
- `SetDescription` coordinator call fails → error propagated via `?` to caller, same as assessment update failures

---

## Technical Decisions

### Key Decisions

#### Decision: Use `is_empty()` method on StructuredDescription

**Context:** `description: {}` in JSON deserializes as `Some(StructuredDescription::default())`, not `None`. Need to detect and skip all-empty descriptions.

**Decision:** Add an `is_empty(&self) -> bool` method to `StructuredDescription` that checks all five fields.

**Rationale:** More idiomatic and readable than `desc == StructuredDescription::default()`. Mirrors the per-field empty checks in `render_structured_description`. Reusable if other callers need the same check.

**Consequences:** Must be maintained if `StructuredDescription` gains new fields (low risk — struct is stable from WRK-028).

#### Decision: Apply description before pipeline_type validation

**Context:** `apply_triage_result` applies assessments unconditionally (lines 1630-1634) before pipeline_type validation (line 1637). Description should follow the same pattern so it persists even for blocked/failed triage outcomes.

**Decision:** Insert description application after assessments and before pipeline_type validation.

**Rationale:** Consistent with how assessments are applied. A description is always useful context regardless of triage outcome.

**Consequences:** Description is persisted even when pipeline_type is invalid and item gets blocked. This is desirable.

#### Decision: Add instruction step 5 to triage prompt

**Context:** Need to tell the triage agent to produce a structured description without disrupting existing instruction flow.

**Decision:** Add step 5 "Write a structured description" between assessment (step 4) and routing (current step 5, renumbered to 6). This places description generation after the agent has analyzed the item but before it decides routing.

**Rationale:** The agent needs to understand the item (steps 1-4) before it can write a meaningful description. Placing it before routing keeps the description step close to the assessment work it builds on.

**Consequences:** Existing steps 5-6 are renumbered to 6-7. No semantic change.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Maintenance burden of `is_empty()` | Must update method if `StructuredDescription` fields change | Clear, readable, reusable empty-check | Struct is stable; low maintenance cost |
| Description quality from title only | Triage agents don't see inbox freeform descriptions | Descriptions from triage are available immediately | Descriptions can be refined by later phases; inbox description rendering is a separate follow-up |

---

## Alternatives Considered

### Alternative: Use `== StructuredDescription::default()` instead of `is_empty()`

**Summary:** Compare against the default-constructed struct to detect empty descriptions.

**How it would work:**
- `StructuredDescription` already derives `Default` and `PartialEq`
- Check `desc == StructuredDescription::default()` in `apply_triage_result`

**Pros:**
- No new method on the struct
- Self-maintaining if fields are added (as long as `Default` derives correctly)

**Cons:**
- Less readable — "equals default" conflates "empty" with "default"
- Not reusable under a clear name

**Why not chosen:** `is_empty()` is more idiomatic Rust and more readable. The maintenance burden of keeping `is_empty()` in sync is negligible for a five-field struct.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Test sites missed when adding `description: None` | Build failure (compile error) | Low | Compiler enforces exhaustive struct construction; any missed site is a compile error, not a runtime bug |
| Triage prompt too long with description guidance | Agent output quality slightly decreases due to prompt length | Low | Guidance is concise (one line per field); existing prompt is well within context limits |

---

## Integration Points

### Existing Code Touchpoints

- `src/types.rs:240-259` — Add `description: Option<StructuredDescription>` to `PhaseResult`
- `src/types.rs:261-273` — Add `is_empty()` method to `StructuredDescription`
- `src/prompt.rs:120-144` — Add description instruction step to triage instructions
- `src/prompt.rs:152-197` — Add description schema to `build_triage_output_suffix` JSON example
- `src/scheduler.rs:1629-1634` — Add description application after assessments in `apply_triage_result`
- ~32 test sites across 6 test files — Add `description: None` to `PhaseResult` struct literals

### External Dependencies

None — all infrastructure exists in-codebase from WRK-028.

---

## Open Questions

None — all questions resolved in PRD and tech research.

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
| 2026-02-20 | Initial design draft (light mode) | Straightforward four-touchpoint design; all infrastructure exists from WRK-028 |
| 2026-02-20 | Self-critique (7 agents) + auto-fixes | Added: exact serde annotations, per-field prompt guidance, merge/error edge cases, precise apply ordering |

## Assumptions

Decisions made without human input during autonomous design:

1. **Light mode selected** — Change is small, mechanical, with well-understood patterns. No architectural decisions or complex alternatives to evaluate.
2. **Step 5 placement for description instruction** — Placed after assessment (step 4) because the agent needs to understand the item before writing a description. Before routing (step 5→6) because description is part of analysis, not routing.
3. **No additional prompt guidance beyond one line per field** — Concise guidance matches the existing triage prompt style. More verbose guidance could improve quality but risks prompt bloat for a small gain.
