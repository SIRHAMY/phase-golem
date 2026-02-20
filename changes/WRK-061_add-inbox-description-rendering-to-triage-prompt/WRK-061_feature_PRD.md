# Change: Add Inbox Description Rendering to Triage Prompt

**Status:** Proposed
**Created:** 2026-02-20
**Author:** Autonomous Agent (WRK-061)

## Problem Statement

When users add items to `BACKLOG_INBOX.yaml` with a `description` field, the description is silently dropped during ingestion. The `ingest_inbox_items()` function in `src/backlog.rs:306` maps most `InboxItem` fields to `BacklogItem` but skips `description`. This means the triage agent never sees the human-provided context, leading to less informed assessments and potentially redundant questions.

The triage prompt rendering pipeline is already complete — `build_preamble()` in `src/prompt.rs:249` renders `StructuredDescription` when present on a `BacklogItem`. The only missing piece is the ingestion-time conversion from `InboxItem.description: Option<String>` to `BacklogItem.description: Option<StructuredDescription>`.

## User Stories / Personas

- **Orchestrator User** — Adds work items to `BACKLOG_INBOX.yaml` with description text to provide context. Expects the triage agent to see and use that context when assessing the item.

## Desired Outcome

When an inbox item has a `description` field, that text should appear in the triage prompt's "Description" section. The triage agent should be able to read and incorporate this context into its assessments, routing decisions, and structured description output.

## Success Criteria

### Must Have

- [ ] `ingest_inbox_items()` converts `InboxItem.description` (simple string) to `StructuredDescription` and sets it on the created `BacklogItem`
- [ ] The inbox description text is placed in the `context` field of `StructuredDescription` (as it represents background/origin context from the human)
- [ ] Description is trimmed before conversion; empty or whitespace-only descriptions produce `None` (consistent with title trimming at `backlog.rs:316`)
- [ ] When triage runs on an ingested inbox item that had a description, the description appears in the triage prompt output
- [ ] Items without a description continue to work unchanged (no regression)
- [ ] Update existing test `ingest_inbox_items_creates_backlog_items_with_correct_fields` in `tests/backlog_test.rs:1030` — it currently asserts `item.description == None` despite passing `description: Some("Details here")`, and must be updated to verify the description is preserved as a `StructuredDescription`

### Should Have

- [ ] Unit test verifying description preservation with cases: (a) normal text maps to `StructuredDescription.context`, (b) `None` stays `None`, (c) empty string `""` becomes `None`, (d) whitespace-only `"  "` becomes `None`

### Nice to Have

- [ ] (None identified)

## Scope

### In Scope

- Modifying `ingest_inbox_items()` in `src/backlog.rs` to map the description field
- Converting the simple string to `StructuredDescription` with the text in the `context` field
- Adding a unit test for description preservation

### Out of Scope

- Changing `InboxItem` schema (it already has `description: Option<String>`)
- Modifying the triage prompt rendering (already works via `render_structured_description()`)
- Changing `StructuredDescription` type or its `Display` impl (covered by WRK-063)
- Flexible deserialization for `StructuredDescription` (covered by WRK-064)
- Any changes to the triage agent's output schema

## Non-Functional Requirements

- **Performance:** Negligible — one string clone per inbox item during ingestion

## Constraints

- The `InboxItem.description` is a simple `Option<String>` — it must be converted to `Option<StructuredDescription>` for `BacklogItem`
- The natural mapping is to place the entire string in the `context` field, since inbox descriptions provide background context rather than structured problem/solution breakdowns

## Dependencies

- **Depends On:** None — all prerequisite infrastructure (StructuredDescription type, prompt rendering) already exists
- **Blocks:** Nothing directly
- **Related:** WRK-063 (Display impl for StructuredDescription) and WRK-064 (flexible deserialization) are independent enhancements that can ship in any order

## Risks

- [ ] (None — this is a small, additive change to an existing ingestion function with no behavioral changes to other code paths)

## Open Questions

- (None — the scope and approach are clear)

## Assumptions

- The inbox description string should map to `StructuredDescription.context` rather than being parsed for structure via `parse_description()`. Inbox items are human-written freeform text and the `context` field is the appropriate catch-all. The `parse_description()` function in `migration.rs` is for v2→v3 migration of structured YAML descriptions, not for simple inbox strings.
- Empty or whitespace-only descriptions should be treated as absent (no `StructuredDescription` created), using `.trim()` before checking emptiness — consistent with the title validation pattern at `backlog.rs:316`

## References

- `src/backlog.rs:306` — `ingest_inbox_items()` function (the change site)
- `src/types.rs:343` — `InboxItem` struct with `description: Option<String>`
- `src/types.rs:263` — `StructuredDescription` struct
- `src/prompt.rs:249` — Description rendering in `build_preamble()`
- `src/prompt.rs:464` — `render_structured_description()` function
- `tests/backlog_test.rs:1030` — Existing test that asserts `description == None` (must be updated)
