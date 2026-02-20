# SPEC: Add Inbox Description Rendering to Triage Prompt

**ID:** WRK-061
**Status:** Ready
**Created:** 2026-02-20
**PRD:** ./WRK-061_feature_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

When users add items to `BACKLOG_INBOX.yaml` with a `description` field, the description is silently dropped during ingestion. The `ingest_inbox_items()` function maps most `InboxItem` fields to `BacklogItem` but skips `description`, relying on `..Default::default()` which sets it to `None`. The triage prompt rendering pipeline already handles `StructuredDescription` display — the only missing piece is the ingestion-time conversion.

## Approach

Add a `.filter().map()` chain in `ingest_inbox_items()` to convert `InboxItem.description: Option<String>` to `BacklogItem.description: Option<StructuredDescription>`. The inbox description string is placed in the `context` field of `StructuredDescription` since inbox descriptions are freeform background context. Empty and whitespace-only strings are filtered to `None`, matching the existing title validation pattern.

**Patterns to follow:**

- `src/backlog.rs:316` — Title trimming and empty-check pattern (`inbox_item.title.trim().is_empty()`)
- `src/backlog.rs:324-336` — Existing `BacklogItem` struct literal construction in `ingest_inbox_items()`

**Implementation boundaries:**

- Do not modify: `src/types.rs` (types already exist), `src/prompt.rs` (rendering already works), `src/migration.rs` (not using `parse_description()`)
- Do not refactor: existing field mappings in `ingest_inbox_items()` or test structure

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Map description and update tests | Low | Add description field mapping in `ingest_inbox_items()` and update/add tests |

**Ordering rationale:** Single phase — the import, mapping logic, and test updates are tightly coupled and too small to separate.

---

## Phases

### Phase 1: Map description and update tests

> Add description field mapping in `ingest_inbox_items()` and update/add tests

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Convert `InboxItem.description` to `BacklogItem.description` during ingestion, with test coverage for all edge cases.

**Files:**

- `src/backlog.rs` — modify — Add `StructuredDescription` import; add `description` field mapping in struct literal
- `tests/backlog_test.rs` — modify — Add `StructuredDescription` import; update existing test assertion; add edge-case test

**Patterns:**

- Follow `src/backlog.rs:316` for trim + empty-check pattern
- Follow existing test at `tests/backlog_test.rs:1030` for test structure conventions

**Tasks:**

- [ ] Add `StructuredDescription` to the types import in `src/backlog.rs` (the `use crate::types::{...}` block near the top of the file)
- [ ] Add `description` field mapping to the `BacklogItem` struct literal in `ingest_inbox_items()`. Note: both `.filter()` and `.map()` call `.trim()` intentionally — they are independent operations and the cost on short strings is negligible.
  ```rust
  description: inbox_item.description
      .as_ref()
      .filter(|d| !d.trim().is_empty())
      .map(|d| StructuredDescription {
          context: d.trim().to_string(),
          ..Default::default()
      }),
  ```
- [ ] Add `StructuredDescription` to the types import in `tests/backlog_test.rs` (the `use phase_golem::types::{...}` block)
- [ ] Update assertion in `ingest_inbox_items_creates_backlog_items_with_correct_fields` (`tests/backlog_test.rs`): replace `assert_eq!(item.description, None)` with:
  ```rust
  let desc = item.description.as_ref().expect("description should be Some");
  assert_eq!(desc.context, "Details here");
  assert!(desc.problem.is_empty());
  assert!(desc.solution.is_empty());
  assert!(desc.impact.is_empty());
  assert!(desc.sizing_rationale.is_empty());
  ```
- [ ] Add new test `ingest_inbox_items_maps_description_edge_cases` covering four cases in a single test function — each case creates an `InboxItem` with a different description value and verifies the ingested result:
  - (a) `description: None` → `item.description` is `None` (regression: items without descriptions still work)
  - (b) `description: Some("")` → `item.description` is `None`
  - (c) `description: Some("   ")` → `item.description` is `None`
  - (d) `description: Some("context text")` → `item.description.context == "context text"`, all other fields empty

**Verification:**

- [ ] `cargo build` succeeds without errors or warnings
- [ ] `cargo test ingest_inbox_items` — all ingestion tests pass
- [ ] `cargo test` — full test suite passes (no regressions)
- [ ] Updated test confirms `description: Some("Details here")` is now preserved as `StructuredDescription { context: "Details here", .. }` (previously asserted `None`)
- [ ] Edge-case test verifies `None`, `""`, and `"   "` all produce `None`, and normal text maps correctly

**Commit:** `[WRK-061][P1] Feature: Map inbox description to BacklogItem during ingestion`

**Notes:**

The existing test at `backlog_test.rs:1030` currently passes `description: Some("Details here")` but asserts `item.description == None` — this documents the current (buggy) behavior. The test update is required to match the new correct behavior.

**Followups:**

---

## Final Verification

- [ ] All phases complete
- [ ] All PRD success criteria met
- [ ] Tests pass
- [ ] No regressions introduced
- [ ] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|

## Followups Summary

### Critical

### High

### Medium

### Low

- Integration test verifying ingested description flows through `build_preamble()` to triage prompt output — deferred because the rendering pipeline is already independently tested; this change only adds the data mapping
- Warning log for whitespace-only descriptions filtered to `None` — deferred because it's not in PRD scope and matches the existing title validation pattern

## Design Details

### Key Types

No new types. Uses existing types:

```rust
// src/types.rs:263
pub struct StructuredDescription {
    pub context: String,      // ← inbox description maps here
    pub problem: String,
    pub solution: String,
    pub impact: String,
    pub sizing_rationale: String,
}
```

### Design Rationale

- **Direct struct init over `parse_description()`** — `parse_description()` is a migration-module function for structured YAML with section headers. Inbox descriptions are freeform text; direct construction avoids coupling and accidental header parsing.
- **`context` field** — Inbox descriptions provide background/origin context, matching the semantic meaning of the `context` field.
- **Double trim** — `.filter(|d| !d.trim().is_empty())` and `.map(|d| d.trim().to_string())` both call trim. This is intentional: filter and map are independent operations, and the cost is negligible on short strings.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
