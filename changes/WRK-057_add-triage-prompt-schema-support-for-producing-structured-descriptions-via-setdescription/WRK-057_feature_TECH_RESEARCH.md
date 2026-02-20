# Tech Research: Triage Prompt Structured Description Support

**ID:** WRK-057
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-057_triage-prompt-structured-description_PRD.md
**Mode:** Light

## Overview

Research how to add a `description: Option<StructuredDescription>` field to `PhaseResult`, update the triage prompt schema and instructions to produce it, and apply it via `SetDescription` in `apply_triage_result`. All infrastructure (types, coordinator handler, prompt rendering) already exists from WRK-028; this research verifies the integration points and identifies any gotchas.

## Research Questions

- [x] What serde pattern should `PhaseResult.description` use? → Same as all other optional fields: `#[serde(default, skip_serializing_if = "Option::is_none")]`
- [x] How should all-empty descriptions be handled? → Check if all five fields are empty strings; skip `SetDescription` if so
- [x] How many test sites need updating? → ~32 `PhaseResult` struct literals across 6 test files
- [x] Where should the description be applied in `apply_triage_result`? → After assessments, before pipeline_type validation

---

## External Research

### Landscape Overview

This change uses three well-established patterns: (1) serde optional field deserialization in Rust, (2) prompting LLMs to produce structured JSON with optional fields, and (3) empty-as-absent struct checks. All three are mainstream with no surprising complexity. The codebase already uses each pattern.

### Common Patterns & Approaches

#### Pattern: Serde `#[serde(default, skip_serializing_if)]` for Optional Nested Structs

**How it works:** `#[serde(default)]` uses `Default::default()` when a field is absent from input JSON. Combined with `skip_serializing_if = "Option::is_none"`, an `Option<T>` field round-trips cleanly: absent in JSON → `None` in Rust → omitted in output JSON. Inner struct fields with `#[serde(default)]` on `String` fields default to `""`.

**When to use:** Optional nested structs where the outer container may or may not include the field, and inner fields may be partially populated.

**Tradeoffs:**
- Pro: Zero-cost backward compatibility. Existing JSON without the field deserializes cleanly.
- Pro: `PhaseResult` already uses this exact pattern for `updated_assessments`, `pipeline_type`, etc.
- Con: `description: {}` (empty object) deserializes as `Some(StructuredDescription::default())`, not `None`. Requires explicit empty-check.

#### Pattern: Empty-as-Absent Struct Check

**How it works:** Check if all fields equal their default/empty values. Treat the entire struct as logically absent if so.

**When to use:** When an LLM might produce `"description": {}` or `"description": {"context": ""}` and you want to treat fully-empty content as "no description provided."

**Tradeoffs:**
- Option A: Hand-written `is_empty()` method — explicit, idiomatic, but must be maintained if fields are added
- Option B: `desc == StructuredDescription::default()` — self-maintaining since struct derives `Default` + `PartialEq`, but slightly less readable
- Either works; the existing `render_structured_description` in prompt.rs already uses per-field empty checks

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| `description: {}` deserialization trap | Serde produces `Some(empty_struct)`, not `None` — could overwrite meaningful existing descriptions | Add explicit empty-check before calling `SetDescription` |
| Missing `#[serde(default)]` on inner fields | LLM providing partial JSON (only some description fields) causes deserialization failure | Already handled — `StructuredDescription` has `#[serde(default)]` on all fields |
| Test site updates missed | Adding a field to `PhaseResult` requires `description: None` at every struct literal site | Search comprehensively; ~32 sites across 6 test files |
| Prompt schema drift | Schema in prompt doesn't match Rust struct — fields silently dropped or defaulted | Keep prompt schema and struct in sync |

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Serde Field Attributes](https://serde.rs/field-attrs.html) | Official Docs | Canonical reference for `default`, `skip_serializing_if` |
| [Structured Outputs Guide (Agenta)](https://agenta.ai/blog/the-guide-to-structured-outputs-and-function-calling-with-llms) | Article | Overview of structured output approaches for LLMs |

---

## Internal Research

### Existing Codebase State

The codebase has complete infrastructure from WRK-028:

- **`StructuredDescription`** (types.rs:261-273) — Five string fields with `#[serde(default)]`, derives `Debug`, `Clone`, `Default`, `PartialEq`, `Serialize`, `Deserialize`
- **`ItemUpdate::SetDescription`** (types.rs:156) — Enum variant carrying `StructuredDescription`, already handled by coordinator
- **`PhaseResult`** (types.rs:240-259) — 8 fields, all optional fields use `#[serde(default, skip_serializing_if = "Option::is_none")]`
- **Coordinator handler** (coordinator.rs:398-401) — Already implements `item.description = Some(description)` with timestamp update
- **`render_structured_description`** (prompt.rs:451-467) — Filters out empty fields, returns empty string when all fields are empty. Already used by `build_preamble()` for downstream phase prompts

**Relevant files/modules:**

| File | Lines | Purpose | Relevance |
|------|-------|---------|-----------|
| `src/types.rs` | 240-259 | `PhaseResult` struct | Add `description: Option<StructuredDescription>` |
| `src/types.rs` | 261-273 | `StructuredDescription` definition | Type to use; no changes needed |
| `src/types.rs` | 156 | `ItemUpdate::SetDescription` | Already exists; use in `apply_triage_result` |
| `src/prompt.rs` | 81-149 | `build_triage_prompt` | Update Instructions section (lines 122-144) |
| `src/prompt.rs` | 152-197 | `build_triage_output_suffix` | Add description schema to JSON example |
| `src/prompt.rs` | 451-467 | `render_structured_description` | Reference for empty-check pattern |
| `src/scheduler.rs` | 1623-1723 | `apply_triage_result` | Add description application before pipeline_type validation |
| `src/scheduler.rs` | 1444-1509 | `handle_triage_success` | Flow context: `complete_phase` before `apply_triage_result` |
| `src/main.rs` | 763-857 | CLI `handle_triage` | Also calls `apply_triage_result` (line 851-857); covered by same change |
| `src/coordinator.rs` | 398-401 | `SetDescription` handler | Already implemented; no changes needed |

### Existing Patterns

1. **Optional PhaseResult fields** — All use `#[serde(default, skip_serializing_if = "Option::is_none")]`. New field follows same pattern.
2. **Assessment application** — In `apply_triage_result`, assessments are applied unconditionally (lines 1630-1634) before pipeline_type validation. Description should follow the same pattern.
3. **Empty field filtering** — `render_structured_description` filters out empty strings. Same logic applies when checking if a description is worth persisting.
4. **Test helpers** — `phase_complete_result` (scheduler_test.rs:101-115) and `make_phase_result` (executor_test.rs:42-56) construct `PhaseResult` with all fields explicit.

### Reusable Components

- `StructuredDescription` — Directly reusable, no changes needed
- `ItemUpdate::SetDescription` — Ready to use, coordinator handles it
- `render_structured_description` — Reference for empty-field logic
- Serde attribute pattern — Copy from existing `PhaseResult` optional fields

### Constraints from Existing Code

- `PhaseResult` field must be optional — backward compatibility with existing JSON
- `StructuredDescription` struct is fixed (from WRK-028) — no modifications
- `apply_triage_result` signature unchanged — description comes from `PhaseResult`
- CLI and scheduler paths both call `apply_triage_result` — one change covers both

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Apply description before pipeline_type validation | Confirmed: assessments are applied unconditionally at lines 1630-1634, before pipeline_type check at 1636. Same position is appropriate for description. | No conflict — PRD is correct |
| All-empty description → skip SetDescription | Confirmed: `description: {}` deserializes as `Some(empty_struct)`, not `None`. Explicit check needed. | Implementation must add empty-check; two approaches available (see Synthesis) |
| ~32 test sites need updating | Confirmed: internal research found ~32 PhaseResult struct literal sites across 6 test files | Mechanical but must be comprehensive |

No significant conflicts between PRD and research findings. The PRD is thorough and accurate.

---

## Critical Areas

### Empty Description Detection

**Why it's critical:** Without this check, `description: {}` from an LLM would call `SetDescription` with blank content, potentially overwriting a meaningful existing description.

**Why it's easy to miss:** Serde's `Option::Some` vs `Option::None` behavior with empty objects is non-obvious. The deserialized value is `Some(StructuredDescription::default())`, which looks truthy.

**What to watch for:** The empty-check must cover all five fields, not just one. A partial description (e.g., only `context` populated) is valid and should be applied.

---

## Deep Dives

*None needed — light mode research.*

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| Where to put `is_empty` logic? | Code organization — method on `StructuredDescription` vs. inline in `apply_triage_result` | Recommend: method on `StructuredDescription` (reusable, testable) or use `== StructuredDescription::default()` (self-maintaining) |

### Recommended Approaches

#### Empty-Check Implementation

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `impl StructuredDescription { fn is_empty() }` | Explicit, idiomatic, testable, reusable | Must maintain if fields added | Preferred for clarity |
| `desc == StructuredDescription::default()` | Self-maintaining, no new code on struct | Less readable, conflates "empty" with "default" | Acceptable alternative |
| Inline check in `apply_triage_result` | No struct changes | Not reusable, verbose | Not recommended |

**Initial recommendation:** Add `is_empty()` method to `StructuredDescription`. It's the most idiomatic Rust approach, mirrors the filtering logic already in `render_structured_description`, and is reusable if other phases adopt descriptions later.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Serde Field Attributes](https://serde.rs/field-attrs.html) | Official Docs | Canonical reference for `default`, `skip_serializing_if` |
| `src/types.rs:240-259` | Codebase | `PhaseResult` pattern to follow |
| `src/prompt.rs:451-467` | Codebase | `render_structured_description` empty-check reference |
| `src/scheduler.rs:1623-1723` | Codebase | `apply_triage_result` integration point |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Light internal + external research | All infrastructure exists from WRK-028; change is mechanical. No conflicts with PRD. Main gotcha is `description: {}` → `Some(empty_struct)` deserialization, which PRD already accounts for. |

## Assumptions

Decisions made without human input during autonomous research:

1. **Light mode selected** — Change is small, well-scoped, and all infrastructure already exists. No deep dives needed.
2. **`is_empty()` method recommended over `== default()` comparison** — More idiomatic, more readable, mirrors existing `render_structured_description` pattern. Design phase can finalize.
3. **No external dependencies needed** — Crates like `serde_nothing` or `is_empty` are overkill for a single five-field struct.
