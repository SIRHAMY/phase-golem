# Design: Add Inbox Description Rendering to Triage Prompt

**ID:** WRK-061
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-061_feature_PRD.md
**Tech Research:** ./WRK-061_feature_TECH_RESEARCH.md
**Mode:** Light

## Overview

Map `InboxItem.description` to `BacklogItem.description` during ingestion by converting the simple string into a `StructuredDescription` with the text placed in the `context` field. This is a single-site, three-line change using `.filter().map()` — the idiomatic Rust approach confirmed by tech research. The triage prompt rendering pipeline already handles `StructuredDescription` display, so no downstream changes are needed.

---

## System Design

### High-Level Architecture

No new components. This change adds a field mapping within the existing `ingest_inbox_items()` function at `src/backlog.rs:324-336`. The data flow is:

```
BACKLOG_INBOX.yaml
  → InboxItem { description: Option<String> }
  → ingest_inbox_items()  ← CHANGE HERE: map description field
  → BacklogItem { description: Option<StructuredDescription> }
  → build_preamble() renders description in triage prompt  (already works)
```

### Component Breakdown

#### Modified: `ingest_inbox_items()` (`src/backlog.rs:306`)

**Purpose:** Converts `InboxItem` structs (from inbox YAML) into `BacklogItem` structs for the backlog.

**Change:** Add `description` field to the `BacklogItem` struct literal (currently defaulting to `None` via `..Default::default()`).

**Mapping logic:**
```rust
description: inbox_item.description
    .as_ref()
    .filter(|d| !d.trim().is_empty())
    .map(|d| StructuredDescription {
        context: d.trim().to_string(),
        ..Default::default()
    }),
```

**Responsibilities (unchanged):**
- Generate unique IDs for new backlog items
- Map inbox fields to backlog fields
- Skip items with empty titles
- Append items to backlog

**Dependencies:** `StructuredDescription` from `types.rs` (must be added to the `use crate::types::{...}` import at `backlog.rs:9-12`; not currently imported)

### Data Flow

1. User writes `BACKLOG_INBOX.yaml` with optional `description` field
2. `ingest_inbox_items()` reads each `InboxItem`
3. **NEW:** For each item, if `description` is `Some(text)` and `text.trim()` is non-empty, create `StructuredDescription { context: text.trim(), ..Default::default() }`; otherwise leave as `None`
4. `BacklogItem` is created with the mapped description and appended to backlog
5. When triage runs, `build_preamble()` renders the `StructuredDescription` (existing behavior)

### Key Flows

#### Flow: Inbox Item with Description

> An inbox item with a description is ingested and its description appears in the triage prompt.

1. **Parse inbox** — `InboxItem` deserialized with `description: Some("Context about the work")`
2. **Filter + map** — Description is non-empty after trim, so `StructuredDescription { context: "Context about the work", ..Default::default() }` is created
3. **Set on BacklogItem** — `description: Some(structured_desc)` set in struct literal
4. **Triage renders** — `build_preamble()` sees `Some(StructuredDescription)`, calls `render_structured_description()`, which outputs `**Context:** Context about the work`

**Edge cases:**
- `description: None` — No mapping occurs, `BacklogItem.description` remains `None` via `..Default::default()` (no change from current behavior)
- `description: Some("")` — Filter catches empty string, result is `None`
- `description: Some("   ")` — Filter catches whitespace-only, result is `None`

---

## Technical Decisions

### Key Decisions

#### Decision: Direct struct init over `parse_description()`

**Context:** `parse_description()` in `migration.rs:541` can convert text to `StructuredDescription`, handling section headers. Inbox descriptions are simple freeform text.

**Decision:** Use direct `StructuredDescription { context: text, ..Default::default() }` construction.

**Rationale:** `parse_description()` is designed for v2→v3 migration of structured YAML descriptions with section headers. Inbox descriptions are simple human-written strings — using `parse_description()` would introduce unnecessary coupling to the migration module and risk unintended header parsing of freeform text.

**Consequences:** If inbox descriptions ever need section-header parsing, the mapping would need updating. This is unlikely and easily changed.

#### Decision: Map to `context` field

**Context:** `StructuredDescription` has five fields: `context`, `problem`, `solution`, `impact`, `sizing_rationale`.

**Decision:** Place the inbox description string in the `context` field.

**Rationale:** The `context` field represents background/origin context — exactly what a human-written inbox description provides. It's the natural catch-all for unstructured text. The PRD confirms this mapping.

**Consequences:** Triage agent sees the description under a "Context" label, which accurately represents its nature.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| No structured parsing | Inbox descriptions always go to `context` field | Simplicity — no parsing logic, no coupling to migration module | Inbox descriptions are freeform; structured parsing would be misleading |
| Double trim | `trim()` is called in both `.filter()` and `.map()` | Clean code — filter and map are independent operations | Negligible cost; trim is O(n) on short strings |

---

## Alternatives Considered

### Alternative: Reuse `parse_description()` from `migration.rs`

**Summary:** Call the existing `parse_description()` function which converts text to `StructuredDescription`, supporting section headers.

**How it would work:**
- Import `parse_description` from migration module
- Call `inbox_item.description.as_ref().filter(|d| !d.trim().is_empty()).map(|d| parse_description(d))`

**Pros:**
- Reuses existing tested code
- Would handle structured descriptions if users ever write them in inbox items

**Cons:**
- Couples ingestion to migration module
- Over-parses simple strings (could split on accidental header-like text)
- Still needs the `.filter()` step for empty/whitespace handling
- Semantically wrong — this isn't a migration operation

**Why not chosen:** Inbox descriptions are simple freeform text, not structured YAML descriptions being migrated. Direct struct init is simpler, explicit, and avoids accidental header parsing.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| None identified | — | — | This is a small, additive change with no behavioral changes to existing code paths |

---

## Integration Points

### Existing Code Touchpoints

- `src/backlog.rs:9-12` — Add `StructuredDescription` to the `use crate::types::{...}` import
- `src/backlog.rs:324-336` — Add `description` field to `BacklogItem` struct literal in `ingest_inbox_items()`
- `tests/backlog_test.rs` — Update test `ingest_inbox_items_creates_backlog_items_with_correct_fields`:
  - Change `assert_eq!(item.description, None)` to verify the description is preserved:
    ```rust
    let desc = item.description.as_ref().expect("description should be Some");
    assert_eq!(desc.context, "Details here");
    assert!(desc.problem.is_empty());
    assert!(desc.solution.is_empty());
    assert!(desc.impact.is_empty());
    assert!(desc.sizing_rationale.is_empty());
    ```
  - Add new test cases for edge cases (PRD "Should Have"):
    - `description: None` → `item.description == None`
    - `description: Some("")` → `item.description == None`
    - `description: Some("   ")` → `item.description == None`
    - `description: Some("text")` → `item.description.context == "text"`

### Downstream Compatibility

- **Serialization:** `StructuredDescription` already derives `Serialize`/`Deserialize` (`types.rs:263`). The `BacklogItem.description` field uses `#[serde(default, skip_serializing_if = "Option::is_none")]`, so `None` descriptions are omitted from YAML and `Some` descriptions serialize correctly. No changes needed.
- **Rendering:** `render_structured_description()` (`prompt.rs:464`) filters empty fields before rendering. A `StructuredDescription` with only `context` populated will render as `**Context:** {value}` — the other empty fields are skipped. The guard at `prompt.rs:250-251` (`if !rendered.is_empty()`) provides defense-in-depth.

### External Dependencies

None.

---

## Open Questions

None — all questions were resolved during PRD and tech research phases.

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
| 2026-02-20 | Initial design draft | Straightforward single-site change; direct `.filter().map()` approach with `StructuredDescription.context` mapping |
| 2026-02-20 | Self-critique (7 agents) | Auto-fixed: added import requirement, specific test assertions, downstream compatibility notes. No directional or critical issues found. |
