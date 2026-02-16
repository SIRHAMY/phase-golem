# WRK-028: Add structured description format for backlog items

## Problem Statement

Backlog items have a `description: Option<String>` field that accepts any free-text content. A useful convention has emerged of structuring descriptions with Context, Problem, Solution, Impact, and Sizing Rationale sections (visible across ~15 items in the current BACKLOG.yaml), but nothing enforces this structure.

Without a structured type:
- Agents produce inconsistent descriptions (some include all sections, some don't)
- Humans don't know what format is expected when writing descriptions
- Downstream phases can't reliably extract specific sections (e.g. just the "Problem" for a PR description)
- Triage assessment quality varies because agents have inconsistent context

## Proposed Approach

### 1. Define `StructuredDescription` type (`types.rs`)

```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct StructuredDescription {
    pub context: String,
    pub problem: String,
    pub solution: String,
    pub impact: String,
    pub sizing_rationale: String,
}
```

Replace `description: Option<String>` on `BacklogItem` with `description: Option<StructuredDescription>`.

### 2. Keep InboxItem description as freeform

The `InboxItem` (from WRK-026) retains `description: Option<String>` since it's the human-friendly entry format. Triage is responsible for expanding freeform inbox descriptions into the structured format when ingesting items into the backlog.

### 3. Update all creation paths

- `backlog::add_item()` — Accept `Option<StructuredDescription>` instead of `Option<String>`
- `backlog::ingest_follow_ups()` — Follow-ups currently set `description: None`; this stays the same
- `backlog::ingest_inbox_items()` — Convert from `InboxItem::description: Option<String>` to a temporary unstructured representation, or keep as `None` and let triage fill it in
- `ItemUpdate::SetDescription` — Change payload from `String` to `StructuredDescription`

### 4. Update display and prompt generation

- `prompt.rs` — Format the structured description with labeled sections instead of a single string
- CLI `--description` flag in `main.rs` — Either accept a JSON/structured format or keep as freeform for quick CLI adds (letting triage structure it later)

### 5. Migration strategy

- Add a migration step (similar to existing v1→v2 migration) that converts existing freeform description strings to `StructuredDescription` by parsing the `Context:`, `Problem:`, `Solution:`, `Impact:`, and `Sizing rationale:` sections from the text
- Items without descriptions remain `None`
- Items with descriptions that don't follow the convention get a best-effort parse or are set to `None` with a warning

### 6. BACKLOG.yaml format change

Existing:
```yaml
description: |-
  Context: The scheduler maintains...
  Problem: The HashMap grows...
  Solution: Either clear entries...
  Impact: Prevents OOM crashes...
  Sizing rationale: Small/low because...
```

New:
```yaml
description:
  context: "The scheduler maintains..."
  problem: "The HashMap grows..."
  solution: "Either clear entries..."
  impact: "Prevents OOM crashes..."
  sizing_rationale: "Small/low because..."
```

## Files Affected

- **Modified:**
  - `orchestrator/src/types.rs` — New `StructuredDescription` struct, update `BacklogItem::description` type, update `ItemUpdate::SetDescription`
  - `orchestrator/src/backlog.rs` — Update `add_item()`, `ingest_follow_ups()`, `ingest_inbox_items()`
  - `orchestrator/src/coordinator.rs` — Update `SetDescription` handler
  - `orchestrator/src/main.rs` — Update CLI `--description` handling
  - `orchestrator/src/prompt.rs` — Update description display formatting
  - `orchestrator/src/migration.rs` — Add freeform→structured migration
  - `orchestrator/tests/coordinator_test.rs` — Update description tests
  - `orchestrator/tests/types_test.rs` — Update serialization tests
  - `orchestrator/tests/backlog_test.rs` — Update inbox/description tests
  - `orchestrator/tests/prompt_test.rs` — Update display tests
  - `BACKLOG.yaml` — Migrate existing description fields to structured format

Estimated ~10 files modified, ~200-300 lines changed, ~150 lines of new tests.

## Assessment

| Dimension  | Rating | Rationale |
|------------|--------|-----------|
| Size       | Medium | ~10 files modified across source, tests, and data; migration of existing items |
| Complexity | Medium | Type change propagation is mechanical but migration parsing requires care; design decisions around CLI input format and inbox-to-structured conversion |
| Risk       | Low    | No shared interfaces beyond BacklogItem (internal to orchestrator). Serde handles missing fields gracefully. Migration can be done incrementally. |
| Impact     | High   | Standardizes the primary context that agents and humans use to understand work items. Enables structured extraction for downstream phases (PRDs, SPECs, PR descriptions). |

## Dependencies

- **WRK-026** (inbox file) — Must be completed first. WRK-028 needs to coordinate with the inbox format: InboxItem keeps freeform descriptions while BacklogItem gets structured ones. The conversion happens during triage/ingestion.

## Edge Cases

- **Existing items with non-conforming descriptions**: Some items may have descriptions that don't follow the Context/Problem/Solution/Impact/Sizing convention. Migration should handle these gracefully (set to None or best-effort parse).
- **CLI quick-add**: The `--description` CLI flag currently accepts a string. For structured descriptions, either accept a mini-format (`--description "context:... problem:... solution:..."`) or keep it freeform and let triage structure it.
- **Follow-up items**: Currently created with `description: None`. This is fine — triage fills in descriptions.
- **Empty structured fields**: Should all fields be required or can some be empty strings? Recommend requiring all fields for completeness, with triage responsible for populating them.

## Assumptions

- All five structured fields (context, problem, solution, impact, sizing_rationale) are required strings (not optional). If a description exists, it must be complete.
- The CLI `--description` flag will remain freeform for convenience. Items added via CLI get `description: None` and triage fills in the structured format.
- Migration is a one-time operation applied during schema version upgrade. Existing items with conforming descriptions are parsed; non-conforming ones are set to `None`.
- The InboxItem format (from WRK-026) retains freeform descriptions. No changes needed to the inbox format.
