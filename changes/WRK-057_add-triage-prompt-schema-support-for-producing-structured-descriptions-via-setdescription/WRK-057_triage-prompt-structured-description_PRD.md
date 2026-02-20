# Change: Add Triage Prompt Schema Support for Structured Descriptions

**Status:** Proposed
**Created:** 2026-02-20
**Author:** Autonomous Agent (from WRK-028 follow-ups)

## Problem Statement

After WRK-028 introduced `StructuredDescription` and `ItemUpdate::SetDescription(StructuredDescription)`, the triage pipeline is the natural place to generate structured descriptions for newly ingested backlog items. However, the triage prompt's JSON output schema (`build_triage_output_suffix` — the function that appends the JSON schema to the triage agent's prompt, instructing it on what fields to include in its result JSON) does not include a `description` field, and `PhaseResult` (the transient JSON file an agent writes upon completing a phase, consumed by the scheduler to apply updates to the backlog item) has no field to carry a `StructuredDescription`. This means triage agents cannot produce structured descriptions, leaving all triaged items with `description: None` until a later phase agent is explicitly prompted to call `SetDescription`, which requires separate per-phase prompt work.

This is the primary gap identified as a follow-up in WRK-028's spec (Followups Summary > Medium: "Triage prompt schema changes").

## User Stories / Personas

- **Orchestrator (system)** - Wants triage agents to produce a structured description so that downstream phase agents (PRD, design, spec, build) receive the five structured description fields (`context`, `problem`, `solution`, `impact`, `sizing_rationale`) pre-populated in their prompts via `build_preamble()` (the function that assembles the context prefix injected at the top of each phase agent's prompt; it already renders any existing `StructuredDescription`, so populating the description at triage time means every subsequent phase automatically benefits without further changes).

- **Human reviewer** - Wants triaged items to have structured descriptions visible in BACKLOG.yaml, reducing the need to re-read raw titles to understand item context. (Note: `orchestrate status` CLI display of descriptions is a separate follow-up — see WRK-028 Followups.)

## Desired Outcome

When a triage agent completes assessment of a backlog item, it can optionally include a `description` object in its JSON result with the five `StructuredDescription` fields (`context`, `problem`, `solution`, `impact`, `sizing_rationale`). The orchestrator deserializes this from the `PhaseResult`, and `apply_triage_result` calls `SetDescription` to persist it on the `BacklogItem`. Downstream phases then see the structured description in their prompts via `build_preamble()`.

## Success Criteria

### Must Have

- [ ] `PhaseResult` has an optional `description` field of type `Option<StructuredDescription>` (with `#[serde(default, skip_serializing_if = "Option::is_none")]`)
- [ ] `build_triage_output_suffix` includes the `description` object as a top-level optional object in the JSON schema with five string sub-fields, each annotated with a one-line explanation of its purpose
- [ ] `apply_triage_result` applies `ItemUpdate::SetDescription` when the triage result contains a non-empty description (at least one field has a non-empty string). The `SetDescription` call is placed before pipeline_type validation, so descriptions are persisted even for blocked or failed triage outcomes, consistent with how assessment updates are applied unconditionally.
- [ ] The triage prompt's `## Instructions` section tells the agent to produce a structured description summarizing the work item
- [ ] All existing tests continue to pass — this requires updating all `PhaseResult` struct literal construction sites (e.g., test helpers in `scheduler_test.rs`) to include `description: None`
- [ ] New tests verify: description field is deserialized from triage result JSON (including partial descriptions where only some fields are populated), description is applied to the item via coordinator, triage prompt output includes the description schema, all-empty description is not applied

### Should Have

- [ ] Triage prompt provides concise guidance (one line per field) on what each description field should contain (e.g., "context: background and origin of this work item", "problem: what issue this addresses", "solution: proposed approach", "impact: expected benefit", "sizing_rationale: why the size/complexity assessment was chosen")
- [ ] Description field is optional in the output schema — triage agents that omit it do not cause errors

### Nice to Have

*None — see Scope note about universal `PhaseResult` change below.*

## Scope

### In Scope

- Adding `description: Option<StructuredDescription>` to `PhaseResult`
- Updating `build_triage_output_suffix` to include the description schema
- Updating triage prompt instructions to ask for a structured description
- Updating `apply_triage_result` to apply `SetDescription` (both the scheduler and `main.rs` CLI triage paths call `apply_triage_result`, so both are covered)
- Tests for the new behavior
- **Note:** The `PhaseResult` change is universal by nature; structurally enabling other phases to carry descriptions is a zero-cost side effect of the Must Have change, not an optional addition. Other phases can adopt it later by updating their prompts — no code changes needed.

### Out of Scope

- Rendering inbox item freeform descriptions in the triage prompt (separate follow-up from WRK-028; inbox items ingested via `BACKLOG_INBOX.yaml` may include a freeform `description` that the triage agent currently does not see — this is a distinct gap tracked separately)
- `Display` impl for `StructuredDescription` / `orchestrate status` display (separate follow-up)
- Changing how non-triage phases produce or consume descriptions
- Schema migration (no migration needed — `PhaseResult` is a transient JSON, not persisted in BACKLOG.yaml)
- Changes to `StructuredDescription` struct itself (already complete from WRK-028)
- Non-triage phase prompt changes to instruct agents to produce descriptions

## Non-Functional Requirements

- **Backward compatibility:** Existing triage result JSON files without a `description` field must deserialize successfully (guaranteed by `#[serde(default)]`). Existing `PhaseResult` struct literal construction sites in tests must be updated to include `description: None`.

## Constraints

- Must use the existing `StructuredDescription` type from `types.rs` — no new description types
- Must use the existing `ItemUpdate::SetDescription` variant — no new update variants
- The description field on `PhaseResult` must be optional to avoid breaking existing phase result consumers
- Description guidance in the triage prompt should be concise — kept within the existing `## Instructions` section

## Dependencies

- **Depends On:** WRK-028 (complete) — `StructuredDescription` type, `ItemUpdate::SetDescription`, and `build_preamble()` rendering
- **Blocks:** Nothing directly, but enables structured description context for all downstream phase agents

## Risks

- [ ] Triage agents have no access to `InboxItem.description` freeform text (it is dropped during `ingest_inbox_items`); descriptions will be generated from item titles alone, which may reduce quality for items with rich inbox descriptions. Mitigation: the description is optional and can be overwritten by later phases; inbox description rendering is tracked as a separate WRK-028 follow-up.
- [ ] Adding a new field to `PhaseResult` requires updating all struct literal construction sites in tests. Mitigation: mechanical change, adding `description: None` to each site.
- [ ] `SetDescription` unconditionally overwrites any existing description (see `coordinator.rs:398`). If an item is re-triaged after a later phase has refined its description, the refined description is overwritten. Mitigation: re-triage is uncommon, and this behavior is consistent with how other triage updates (assessments, pipeline_type) work — triage always sets the authoritative initial values.

## Open Questions

- [x] Should the description be applied before or after assessments in `apply_triage_result`? **Decision:** Apply before pipeline_type validation (alongside assessments), so descriptions are persisted even when triage results in a blocked or failed outcome. This is consistent with how assessment updates (size, complexity, risk, impact) are applied unconditionally.
- [x] Should non-triage phases also be able to set descriptions via `PhaseResult`? **Decision:** Yes, the `PhaseResult` change makes this structurally possible for all phases, but only triage prompts will actively instruct agents to produce descriptions initially. Other phases can adopt it later without code changes.
- [x] What happens when a triage result includes a description but the item is merged (duplicate detection)? **Decision:** The description is not applied. When `process_merges` returns `is_merged = true`, `handle_triage_success` returns early without calling `apply_triage_result` (the item ceases to exist after merge, so this is correct behavior).
- [x] What happens when all five description fields are empty strings? **Decision:** An all-empty `StructuredDescription` is treated as absent — `apply_triage_result` should skip the `SetDescription` call. This prevents overwriting a meaningful existing description with blank content.
- [x] What about `description: null` in the JSON result? **Decision:** Treated identically to omission — `Option::None`, no `SetDescription` call. This is guaranteed by `#[serde(default, skip_serializing_if = "Option::is_none")]`.

## References

- WRK-028 SPEC: `changes/WRK-028_add-structured-description-format-for-backlog-items/WRK-028_add-structured-description-format-for-backlog-items_SPEC.md`
- `StructuredDescription` definition: `src/types.rs:261-273`
- `ItemUpdate::SetDescription`: `src/types.rs:156`
- `PhaseResult`: `src/types.rs:240-259`
- `build_triage_output_suffix`: `src/prompt.rs:152-197`
- `apply_triage_result`: `src/scheduler.rs:1623-1702`
- `handle_triage_success`: `src/scheduler.rs:1444-1509`

## Assumptions

Decisions made without human input during autonomous PRD creation:

1. **Small, well-scoped follow-up.** This is a direct follow-up from WRK-028 with clear boundaries. The problem, solution shape, and affected code are all well-understood from the parent spec's follow-ups summary.
2. **Description applied in `apply_triage_result`.** This is the function that already handles assessment updates (size, complexity, risk, impact) and pipeline type from triage results, making it the natural place for description application. Both the scheduler path (`handle_triage_success` → `apply_triage_result`) and the CLI path (`main.rs` `handle_triage` → `apply_triage_result`) call this function, so both paths are covered.
3. **Making `description` universal in `PhaseResult` is intentional.** It avoids the need for a triage-specific result type and lets other phases adopt structured descriptions later without code changes, though their prompts would need updating separately.
4. **No schema migration needed.** `PhaseResult` is a transient JSON file written by agents and consumed by the scheduler. It is not persisted in BACKLOG.yaml, so no data migration is required.
5. **Triage prompt instructs but doesn't require description.** The field is optional in the output schema. Agents that don't produce it (e.g., when blocking or failing) won't cause errors.
6. **Description update is not atomic with the triage git commit.** In `handle_triage_success`, `complete_phase` creates a git commit before `apply_triage_result` runs. The description (like assessments and pipeline_type) is persisted to BACKLOG.yaml after the triage commit, appearing in the next commit cycle. This is acceptable and consistent with existing behavior for all triage updates applied via `apply_triage_result`.
7. **Partial descriptions are acceptable.** If an agent provides only some fields (e.g., `context` and `problem` but not `solution`), `#[serde(default)]` fills missing fields with empty strings. `render_structured_description` in `prompt.rs` already skips empty fields, so only populated fields appear in downstream prompts. This is preferable to rejecting partial descriptions.
