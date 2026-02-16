# WRK-028: Add Structured Description Format for Backlog Items

## Scoping Summary

### What This Involves

Replace the `description: Option<String>` field on `BacklogItem` with a structured `Option<StructuredDescription>` type containing five required fields: `context`, `problem`, `solution`, `impact`, and `sizing_rationale`. This enforces the convention already followed by ~15 items in the current BACKLOG.yaml and makes descriptions machine-parseable for downstream phases. The `InboxItem` type retains its freeform `description: Option<String>` — triage expands freeform descriptions into structured format during ingestion.

### Key Files

**Source (modified):**
- `orchestrator/src/types.rs` — New `StructuredDescription` struct; change `BacklogItem::description` from `Option<String>` to `Option<StructuredDescription>`; update `ItemUpdate::SetDescription` payload
- `orchestrator/src/backlog.rs` — Update `add_item()`, `ingest_inbox_items()`, `ingest_follow_ups()` signatures and field initialization
- `orchestrator/src/coordinator.rs` — Update `SetDescription` handler (line 374)
- `orchestrator/src/prompt.rs` — Format structured description with labeled sections instead of a flat string (line 333)
- `orchestrator/src/main.rs` — Update CLI `--description` flag handling (line 68, 511); likely keep as freeform and let triage structure it
- `orchestrator/src/migration.rs` — Add freeform-to-structured migration logic for schema v2→v3 (parse `Context:`, `Problem:`, etc. sections from existing text)

**Tests (modified):**
- `orchestrator/tests/types_test.rs` — Serialization/deserialization tests for StructuredDescription
- `orchestrator/tests/backlog_test.rs` — Update `description: None` fields in test helpers; update inbox ingestion tests
- `orchestrator/tests/coordinator_test.rs` — Update SetDescription tests
- `orchestrator/tests/prompt_test.rs` — Update description display tests
- `orchestrator/tests/migration_test.rs` — Add v2→v3 migration tests

**Data (modified):**
- `BACKLOG.yaml` — Migrate all existing freeform description strings to structured YAML maps (schema_version bump to 3)

### Approach Sketch

1. Define `StructuredDescription` struct with serde derive in `types.rs`. All five fields are required `String`s — if a description exists it must be complete.
2. Change `BacklogItem::description` to `Option<StructuredDescription>` with the same `#[serde(default, skip_serializing_if = "Option::is_none")]` attributes.
3. Change `ItemUpdate::SetDescription(String)` to `SetDescription(StructuredDescription)`.
4. Add a v2→v3 migration in `migration.rs` that parses existing freeform descriptions by splitting on `Context:`, `Problem:`, `Solution:`, `Impact:`, and `Sizing rationale:` prefixes. Items with non-conforming descriptions get `description: None` with a warning log. Bump `EXPECTED_SCHEMA_VERSION` to 3.
5. Update `prompt.rs` to render structured descriptions with labeled sections.
6. Keep CLI `--description` as freeform `Option<String>` — items added via CLI get `description: None` and triage fills in the structured format. (Alternatively, drop the `--description` flag for CLI adds since it can't produce a structured description.)
7. Keep `InboxItem::description` as `Option<String>` — no changes to the inbox format.
8. Migrate the existing BACKLOG.yaml data as part of the schema version bump.

### Risks or Concerns

- **Migration parsing fragility**: Existing freeform descriptions mostly follow the convention but some may have slight variations (e.g., "Sizing Rationale" vs "Sizing rationale", varying punctuation). The parser needs to be case-insensitive and tolerant of minor format variations.
- **Schema version bump**: Going from v2 to v3 means the existing v1→v2 migration chain gets extended. This is mechanical but adds to migration.rs complexity.
- **CLI `--description` awkwardness**: The CLI flag currently accepts a freeform string which won't match the structured type. Either remove it, keep it as freeform (set `description: None` on the BacklogItem and rely on triage), or add a mini-format parser.
- **Agent prompt changes**: Agents that currently emit `SetDescription(String)` need to emit `SetDescription(StructuredDescription)` instead. The triage prompt and any prompts that produce descriptions need updating to match the new type.
- **WRK-026 dependency**: The idea file lists WRK-026 as a dependency. WRK-026 (inbox file) appears to have been completed (it went through spec and review phases). The inbox format and `InboxItem` type already exist, so this dependency is satisfied.

### Assessment

| Dimension  | Rating | Rationale |
|------------|--------|-----------|
| Size       | Medium | ~10-11 files modified across source, tests, and data. ~200-300 lines changed plus migration logic and test updates. |
| Complexity | Medium | Type change propagation is mechanical but migration parsing requires care. Multiple design decisions: CLI flag handling, migration tolerance, agent prompt updates. |
| Risk       | Low    | Isolated to the orchestrator crate. Serde handles missing fields gracefully. Existing tests cover all touch points. Migration can be validated against current BACKLOG.yaml. |
| Impact     | High   | Standardizes the primary context agents and humans use to understand work items. Enables structured extraction for downstream phases (PRDs, SPECs, PR descriptions). Eliminates inconsistent descriptions. |

### Assumptions

- WRK-026 (inbox file) is complete and the `InboxItem` type is stable.
- All five structured fields are required strings — partial descriptions are not allowed.
- CLI `--description` flag will be simplified to set `description: None`, deferring structured description creation to triage.
- Schema version will bump from 2 to 3 to trigger automatic migration.
- Migration parser will be best-effort for non-conforming descriptions, defaulting to `None` with a warning.
