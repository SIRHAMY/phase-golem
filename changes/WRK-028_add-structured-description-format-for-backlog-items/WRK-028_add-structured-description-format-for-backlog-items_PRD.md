# Change: Add Structured Description Format for Backlog Items

**Status:** Proposed
**Created:** 2026-02-13
**Author:** Orchestrator (autonomous)

## Problem Statement

The orchestrator's `BacklogItem` type uses `description: Option<String>` — an unstructured free-text field — to hold what has become the primary context that both agents and humans use to understand work items. A five-section convention (Context, Problem, Solution, Impact, Sizing Rationale) was adopted for manually-created items and is followed by ~7 of the 9 items that have descriptions, but nothing in the type system enforces this structure.

The lack of structural enforcement creates three concrete problems:

1. **Agents produce inconsistent output.** When triage or follow-up ingestion creates items, there is no schema telling the agent what sections to populate. Follow-up ingestion always sets `description: None`. Triage does not set descriptions at all — the `SetDescription` update variant exists in the coordinator but is never called by the scheduler or triage pipeline.

2. **Downstream phases cannot reliably parse descriptions.** The unused `build_context_preamble` function (marked `#[allow(dead_code)]`) renders descriptions as a single `**Description:** {desc}` line. There is no way to extract just the "Problem" or "Solution" for a specific phase.

3. **The production prompt path (`build_preamble`) does not render descriptions at all.** Descriptions in BACKLOG.yaml are invisible to phase agents today. Only `build_context_preamble` (unused dead code) includes them.

Without structural enforcement, the convention erodes over time: 36 of 45 active backlog items have no description, and 2 of the 9 with descriptions use ad-hoc prose instead of the convention.

## User Stories / Personas

- **Triage agent** — Receives items with titles and optional freeform descriptions but no structured context. Needs a clear schema to know what sections to populate when expanding a freeform inbox description into a backlog-ready description.

- **Downstream phase agent** — Automated agents that execute specific workflow phases (PRD, design, spec, build, review). Consume descriptions in prompts to understand what to build and why. Need structured, consistent descriptions so the agent can reason about the problem, solution, and impact without guessing what sections exist.

- **Human operator** — Writes inbox items with freeform descriptions and reviews the backlog to understand item status. Needs a clear format expectation for descriptions and confidence that agents will produce consistent, useful descriptions.

- **Follow-up generator** — Creates new items during phase execution with a `context` string. Needs a path for that context to flow into a structured description during triage.

## Desired Outcome

After this change:

- `BacklogItem::description` is `Option<StructuredDescription>` where `StructuredDescription` has five required `String` fields: `context`, `problem`, `solution`, `impact`, and `sizing_rationale`.
- Existing BACKLOG.yaml items with descriptions that follow the five-section convention (containing `Context:`, `Problem:`, `Solution:`, `Impact:`, and `Sizing rationale:` headers) are automatically migrated to the structured type via a v2→v3 schema migration.
- Items with non-conforming descriptions (freeform prose without section headers) are migrated with the full text placed in the `context` field and other fields set to empty strings. This preserves information while signaling that the description needs expansion during triage.
- Items without descriptions retain `description: None`.
- `InboxItem::description` remains `Option<String>` — the inbox stays human-friendly. Triage is responsible for expanding freeform descriptions into structured format during ingestion.
- Serde enforces that if a `StructuredDescription` exists, all five fields are present (though fields may contain empty strings for partially-populated items).
- The production prompt rendering (`build_preamble`) includes structured descriptions with labeled sections so phase agents can see them.

## Success Criteria

### Must Have

- [ ] `StructuredDescription` struct defined in `types.rs` with five `String` fields and serde derives
- [ ] `BacklogItem::description` changed from `Option<String>` to `Option<StructuredDescription>`
- [ ] `ItemUpdate::SetDescription` payload changed from `String` to `StructuredDescription`
- [ ] v2→v3 migration in `migration.rs` that parses existing freeform descriptions by section headers (case-insensitive matching of `Context:`, `Problem:`, `Solution:`, `Impact:`, `Sizing rationale:`) and places non-conforming descriptions in the `context` field with other fields as empty strings
- [ ] `EXPECTED_SCHEMA_VERSION` bumped from 2 to 3 in `backlog.rs`
- [ ] Schema version in BACKLOG.yaml bumped to 3 with existing descriptions migrated to structured format (committed atomically with code changes)
- [ ] `build_context_preamble` in `prompt.rs` renders structured descriptions with labeled sections (format: `**Context:** {text}\n**Problem:** {text}\n...` for each non-empty field)
- [ ] `build_preamble` (the production prompt path used by `build_prompt` and `build_triage_prompt`) updated to render structured descriptions so phase agents can see them
- [ ] All existing tests updated and passing, including new tests for: migration of convention-formatted descriptions, migration of non-conforming descriptions, migration of items without descriptions
- [ ] `InboxItem::description` remains `Option<String>` (no change to inbox format)
- [ ] Migration is idempotent — loading an already-migrated v3 file does not re-trigger migration

### Should Have

- [ ] Migration handles all edge cases tested against the 9 existing descriptions in current BACKLOG.yaml: 7 convention-formatted items parsed into fields, 2 freeform-prose items placed in `context` field
- [ ] Coordinator `SetDescription` handler updated to accept `StructuredDescription` and tested
- [ ] CLI `--description` flag on `orchestrate add` removed — items added via CLI start with `description: None` and get structured descriptions during triage (aligns with the inbox model where human input is freeform and agents expand it)

### Nice to Have

- [ ] `Display` impl for `StructuredDescription` for consistent formatting in CLI output
- [ ] Flexible deserialization that accepts both a structured YAML object and a plain string (like `FollowUp`'s custom `Deserialize` impl), mapping plain strings to the `context` field with other fields as empty strings — for robustness against agent output variations

## Scope

### In Scope

- New `StructuredDescription` type definition with serde derives
- Type change on `BacklogItem::description` and `ItemUpdate::SetDescription`
- v2→v3 migration logic (V2 struct definitions, parsing, schema version bump)
- Migration of existing BACKLOG.yaml data (happens automatically on load after schema bump)
- Prompt rendering updates for structured descriptions in both `build_context_preamble` and `build_preamble`
- Coordinator handler update for `SetDescription`
- `add_item()` signature update in `backlog.rs`
- `ingest_inbox_items()` update — sets `description: None` on ingested items (the freeform inbox description is passed to the triage agent via the prompt's item rendering, not copied to the structured description field)
- `ingest_follow_ups()` — already sets `description: None`, no change needed
- Test updates across all test files (~7 test files plus `common/mod.rs`). Most test `BacklogItem` constructions use `description: None` which requires no change since the field is still `Option<_>`.

### Out of Scope

- **Inbox format changes** — `InboxItem::description` stays `Option<String>`. Inbox ergonomics are WRK-026's concern.
- **Triage prompt schema changes** — Making triage output structured descriptions via its JSON output schema. This involves prompt engineering and should be a follow-up item so triage can produce `SetDescription` updates.
- **Agent prompt schema changes for SetDescription** — Making phase output JSON include description update fields so agents can set/update descriptions mid-phase.
- **Worklog description rendering** — `write_archive_worklog_entry` does not currently include descriptions.
- **Content validation rules** — No enforcement of non-empty fields, minimum lengths, or content quality beyond serde requiring the struct fields to be present when the struct exists.
- **Backward compatibility for external v2 readers** — The orchestrator owns the BACKLOG.yaml file format exclusively; no known external readers exist.
- **CLI restructuring** — No new `--context`, `--problem`, etc. flags on `orchestrate add`.
- **Comprehensive prompt re-engineering** — Only minimal rendering updates are included; broader prompt improvements are deferred.

## Non-Functional Requirements

- **Performance:** Migration parsing should complete within 1 second for backlogs with 100+ items. String splitting by section headers is O(n) per description, bounded by description length (typically 100-500 words based on existing items).
- **Observability:** Migration should log warnings for items with descriptions that couldn't be parsed into the five-section convention format, including the item ID and a preview of the text. These items will still be migrated (full text in `context` field) but the warning alerts operators that the description may need manual expansion.

## Constraints

- Must follow the existing v1→v2 migration pattern (define `V2BacklogFile`/`V2BacklogItem` structs, write `map_v2_item` function).
- `InboxItem::description` must remain `Option<String>` to preserve the human-friendly inbox format established by WRK-026.
- All five `StructuredDescription` fields are `String` (not `Option<String>`) — if the struct exists, all sections are present. This eliminates null-checking for individual fields in consuming code and aligns with the existing convention where all five sections are always included together. Empty strings indicate "not yet populated."
- Schema version must be bumped (2→3) to trigger automatic migration on first load. The version bump in code and in BACKLOG.yaml must be committed atomically in the same commit.

## Dependencies

- **Depends On:** WRK-026 (inbox file) — completed (went through build and review phases). The `InboxItem` type with `description: Option<String>` is stable.
- **Blocks:** Nothing directly, but downstream features (triage prompt improvements to produce structured descriptions, agent description output) build on this type.

## Risks

- [ ] **Migration parsing fragility:** Existing freeform descriptions mostly follow the convention but have variations (e.g., "Sizing Rationale" vs "Sizing rationale", varying punctuation). Parser must be case-insensitive and tolerant of minor format variations. Mitigation: test against all 9 existing descriptions in BACKLOG.yaml; non-conforming descriptions are placed in `context` field rather than discarded.
- [ ] **Test update volume:** ~30+ `BacklogItem` constructions across 7 test files reference the description field. Mitigation: most use `description: None` which requires no change since the field is still `Option<_>`. Only tests that construct `Some(description)` values need updating.
- [ ] **Agent output mismatch:** Agents that currently could emit `SetDescription(String)` will need to emit `SetDescription(StructuredDescription)`. Until triage prompts are updated (out of scope), the `SetDescription` variant exists but won't be called in production. Mitigation: this is safe — `SetDescription` is not currently called in production, only in tests.
- [ ] **Migration failure risk:** If the migration parser has a bug, BACKLOG.yaml could become unreadable. Mitigation: the existing migration pattern writes the migrated file only after successful parsing and validation. Git history serves as the backup — the pre-migration BACKLOG.yaml is always recoverable via `git checkout`.

## Assumptions

Decisions made without human input during autonomous PRD creation:

1. **`build_preamble` rendering promoted to Must Have.** The Problem Statement identifies "descriptions invisible to agents" as a core problem. Shipping structured descriptions without wiring them into the production prompt path would not address this problem. Promoted from Should Have.
2. **Non-conforming descriptions preserved in `context` field.** Chose information preservation (option a) over lossy `None` (option b). Two existing items (WRK-050, WRK-051) have freeform prose descriptions that would be lost with the lossy approach.
3. **CLI `--description` flag removed.** Items added via CLI start with `description: None`. This aligns with the inbox model where human input is freeform and triage agents expand it into structured format.
4. **FollowUp items do not carry descriptions.** Follow-ups remain lightweight with `description: None`. Triage expands them into structured descriptions during processing.
5. **Migration is idempotent.** Loading an already-migrated v3 file does not re-trigger migration. The existing migration pattern already handles this via schema version checking.
6. **WRK-026 is complete.** The `InboxItem` type is stable and the inbox format is established.

## Open Questions

- [ ] **Should there be a minimum quality threshold for structured descriptions?** Currently, a `StructuredDescription` with all empty-string fields is valid. Should at least one field be non-empty, or is this acceptable for items in early pipeline stages?
- [ ] **Should `orchestrate status` display structured descriptions?** Currently it shows nothing. Options: show nothing (inspect YAML directly), show a one-line summary from the `problem` field, or show all five fields. Not blocking for v1.

## References

- [WRK-028 Scoping Document](./SCOPE.md)
- [WRK-026 Inbox File Feature](../WRK-026_add-backlog-inbox-file/) — Established the inbox format with freeform descriptions
- Current BACKLOG.yaml — Live examples of both convention-formatted and freeform descriptions
- `orchestrator/src/types.rs` — Current `BacklogItem` and `InboxItem` type definitions
- `orchestrator/src/migration.rs` — Existing v1→v2 migration pattern to follow
