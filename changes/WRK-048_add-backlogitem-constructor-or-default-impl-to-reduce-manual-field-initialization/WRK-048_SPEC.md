# SPEC: Add BacklogItem Default impl

**ID:** WRK-048
**Status:** Ready
**Created:** 2026-02-13
**PRD:** ./WRK-048_add-backlogitem-constructor-or-default-impl-to-reduce-manual-field-initialization_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

`BacklogItem` has 22 fields, but only 5 vary between construction sites (`id`, `title`, `status`, `created`, `updated`). The remaining 17 have natural defaults (`None`, `Vec::new()`, `false`). Every construction site manually specifies all 22 fields — ~20 lines of boilerplate each. Adding a new optional field requires updating all ~20 construction sites even though the value is always the same default. `StructuredDescription` in the same file already uses `#[derive(Default)]`, establishing the pattern.

## Approach

Add `#[derive(Default)]` to `ItemStatus` (with `#[default]` on `New`) and `BacklogItem`, then update all construction sites to use struct update syntax (`..Default::default()`). This is a pure refactor — runtime behavior is identical.

Rust's `Default` trait and serde's `#[serde(default)]` are independent mechanisms. This change only adds a Rust `Default` impl for struct literal construction. It does not change YAML deserialization behavior. Fields without `#[serde(default)]` (`id`, `title`, `created`, `updated`, `status`) remain required in YAML.

**Patterns to follow:**

- `.claude/skills/changes/orchestrator/src/types.rs:269-281` — `StructuredDescription` already derives `Default`, establishing the pattern in this codebase
- `.claude/skills/changes/orchestrator/tests/prompt_test.rs:52-59` — `make_item_with_assessments` already uses struct update syntax (`..make_item(...)`)

**Implementation boundaries:**

- Do not modify: serde attributes on any `BacklogItem` field
- Do not modify: field types, field order, or struct layout
- Do not add: `Default` impls for `SizeLevel`, `DimensionLevel`, `BlockType`, or `PhasePool` (all wrapped in `Option`, not needed)
- Do not add: builder pattern, constructor functions, or `new()` methods

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Default derives + unit test | Low | Add `#[derive(Default)]` to `ItemStatus` and `BacklogItem`, add unit test verifying all 22 field defaults |
| 2 | Production + migration site updates | Low | Update 4 construction sites in `backlog.rs` (3) and `migration.rs` (1) to use `..Default::default()`; confirm `map_v2_item` stays explicit |
| 3 | Test site updates | Low | Update 4 test helpers and ~8 inline constructions in `types_test.rs` to use `..Default::default()` |

**Ordering rationale:** Phase 1 must come first because `..Default::default()` syntax requires `Default` to be implemented. Phase 2 before Phase 3 because production code correctness is higher priority. Phase 3 is independent of Phase 2 but ordered after for review flow.

---

## Phases

---

### Phase 1: Default derives + unit test

> Add `#[derive(Default)]` to `ItemStatus` and `BacklogItem`, add unit test verifying all 22 field defaults

**Phase Status:** complete

**Complexity:** Low

**Goal:** Enable `..Default::default()` syntax for `BacklogItem` construction and verify all defaults are correct.

**Files:**

- `.claude/skills/changes/orchestrator/src/types.rs` — modify — Add `Default` to derive lists for `ItemStatus` (L5) and `BacklogItem` (L186), add `#[default]` attribute to `ItemStatus::New` (L8)
- `.claude/skills/changes/orchestrator/tests/types_test.rs` — modify — Add `test_backlogitem_default` unit test

**Patterns:**

- Follow `StructuredDescription` derive at `types.rs:269` — same file, same pattern

**Tasks:**

- [x] Add `Default` to `ItemStatus` derive list (L5): `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]`
- [x] Add `#[default]` attribute above `New` variant (L8)
- [x] Add `Default` to `BacklogItem` derive list (L186): `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]`
- [x] Add `test_itemstatus_default` test to `types_test.rs` that asserts `ItemStatus::default() == ItemStatus::New`
- [x] Add `test_backlogitem_default` test to `types_test.rs` that calls `BacklogItem::default()` and asserts all 22 fields individually:
  - `id == ""`
  - `title == ""`
  - `status == ItemStatus::New`
  - `phase == None`
  - `size == None`
  - `complexity == None`
  - `risk == None`
  - `impact == None`
  - `requires_human_review == false`
  - `origin == None`
  - `blocked_from_status == None`
  - `blocked_reason == None`
  - `blocked_type == None`
  - `unblock_context == None`
  - `tags == Vec::<String>::new()`
  - `dependencies == Vec::<String>::new()`
  - `created == ""`
  - `updated == ""`
  - `pipeline_type == None`
  - `description == None`
  - `phase_pool == None`
  - `last_phase_commit == None`
- [x] Run `cargo test` — all existing tests must pass unchanged

**Verification:**

- [x] `BacklogItem::default()` compiles and returns expected values
- [x] `test_itemstatus_default` passes
- [x] `test_backlogitem_default` passes, asserting all 22 fields individually
- [x] All existing tests pass (no behavioral change) — including YAML round-trip tests which confirm serde behavior is unchanged
- [x] `cargo build` succeeds
- [x] No serde attributes were added, removed, or modified on any field (verify via diff)
- [x] Code review passes (`/code-review` -> fix issues -> repeat until pass)

**Commit:** `[WRK-048][P1] Feature: Add Default derives for ItemStatus and BacklogItem`

**Notes:**

Only `ItemStatus` needs a new `Default` impl because it appears as a direct (non-`Option`) field in `BacklogItem`. All other enum types (`SizeLevel`, `DimensionLevel`, `BlockType`, `PhasePool`) appear only inside `Option<T>` wrappers, so `Option<T>` handles their default (`None`).

**Followups:**

---

### Phase 2: Production + migration site updates

> Update 4 construction sites in `backlog.rs` and `migration.rs` to use `..Default::default()`; confirm `map_v2_item` stays explicit

**Phase Status:** complete

**Complexity:** Low

**Goal:** Eliminate boilerplate in production and migration code by replacing explicit default-valued fields with struct update syntax.

**Files:**

- `.claude/skills/changes/orchestrator/src/backlog.rs` — modify — Update 3 construction sites: `add_item` (L146-169), `ingest_follow_ups` (L277-300), `ingest_inbox_items` (L364-387)
- `.claude/skills/changes/orchestrator/src/migration.rs` — modify — Update 2 construction sites: `map_v1_item` (L145-168), `map_v2_item` (L417-440)

**Tasks:**

- [x] Update `add_item` (backlog.rs L146-169): keep `id`, `title`, `status`, `size`, `risk`, `created`, `updated`; replace remaining 15 fields with `..Default::default()`
- [x] Update `ingest_follow_ups` (backlog.rs L277-300): keep `id`, `title`, `status`, `size`, `risk`, `origin`, `created`, `updated`; replace remaining 14 fields with `..Default::default()`
- [x] Update `ingest_inbox_items` (backlog.rs L364-387): keep `id`, `title`, `status`, `size`, `risk`, `impact`, `origin`, `dependencies`, `pipeline_type`, `created`, `updated`; replace remaining 11 fields with `..Default::default()`
- [x] Update `map_v1_item` (migration.rs L145-168): keep all fields mapped from V1 struct (`id`, `title`, `status`, `phase`, `size`, `complexity`, `risk`, `impact`, `requires_human_review`, `origin`, `blocked_from_status`, `blocked_reason`, `blocked_type`, `unblock_context`, `tags`, `dependencies`, `created`, `updated`, `pipeline_type`, `phase_pool`); replace `description: None`, `last_phase_commit: None` with `..Default::default()`
- [x] Review `map_v2_item` (migration.rs L417-440): this site maps all 22 fields from V2 — every field has an explicit value from the source struct. Keep fully explicit (no `..Default::default()`) since no fields use default values. No changes needed.
- [x] Run `cargo test` — all tests must pass unchanged

**Verification:**

- [x] All 3 `backlog.rs` construction sites use `..Default::default()`
- [x] `map_v1_item` uses `..Default::default()` for fields not in V1 schema
- [x] `map_v2_item` remains fully explicit (all 22 fields mapped from V2)
- [x] All existing tests pass (no behavioral change)
- [x] `cargo build` succeeds
- [x] Code review passes (`/code-review` -> fix issues -> repeat until pass)

**Commit:** `[WRK-048][P2] Clean: Use Default for BacklogItem construction in production and migration code`

**Notes:**

`map_v2_item` maps all 22 fields from the V2 struct — every field has an explicit value from the source. Keep it fully explicit; `..Default::default()` provides no benefit here. The benefit of `Default` for migration sites is `map_v1_item` only, where `description` and `last_phase_commit` didn't exist in V1 (2 fields saved).

**Followups:**

---

### Phase 3: Test site updates

> Update 4 test helpers and ~8 inline constructions in `types_test.rs` to use `..Default::default()`

**Phase Status:** complete

**Complexity:** Low

**Goal:** Reduce test boilerplate by using struct update syntax in test helpers and inline test constructions.

**Files:**

- `.claude/skills/changes/orchestrator/tests/common/mod.rs` — modify — Update `make_item` helper (L22-45)
- `.claude/skills/changes/orchestrator/tests/prompt_test.rs` — modify — Update local `make_item` helper (L25-49)
- `.claude/skills/changes/orchestrator/tests/scheduler_test.rs` — modify — Update local `make_item` helper (L27-50)
- `.claude/skills/changes/orchestrator/tests/worklog_test.rs` — modify — Update `make_test_item` helper (L7-30)
- `.claude/skills/changes/orchestrator/tests/types_test.rs` — modify — Update inline BacklogItem constructions in round-trip and serialization tests (~8 sites)

**Tasks:**

- [x] Update `common/mod.rs::make_item` (L22-45): keep `id`, `title`, `status`, `created`, `updated`; replace 17 defaulted fields with `..Default::default()`
- [x] Update `prompt_test.rs::make_item` (L25-49): keep `id`, `title`, `status`, `phase`, `created`, `updated`; replace remaining fields with `..Default::default()`
- [x] Update `scheduler_test.rs::make_item` (L27-50): keep `id`, `title`, `status`, `created`, `updated`; replace remaining fields with `..Default::default()`
- [x] Update `worklog_test.rs::make_test_item` (L7-30): keep `id`, `title`, `status`, `phase`, `created`, `updated`; replace remaining fields with `..Default::default()`
- [x] Update inline BacklogItem constructions in `types_test.rs` (~8 sites): apply `..Default::default()` to all sites, keeping only fields with non-default values explicit. Guideline: if a test sets >50% of fields to non-default values (like `yaml_round_trip_backlog_item_full`), keep all fields explicit for documentation clarity. If a test sets mostly defaults (like `yaml_round_trip_backlog_item_minimal`, `optional_fields_omitted_when_none`), use `..Default::default()`
- [x] Run `cargo test` — all tests must pass unchanged

**Verification:**

- [x] All test helpers use `..Default::default()` syntax
- [x] Inline test constructions in `types_test.rs` are updated where appropriate
- [x] All existing tests pass with identical assertions (no behavioral change)
- [x] `cargo build` and `cargo test` succeed
- [x] Code review passes (`/code-review` -> fix issues -> repeat until pass)

**Commit:** `[WRK-048][P3] Clean: Use Default for BacklogItem construction in tests`

**Notes:**

For `types_test.rs` inline constructions, apply the >50% rule: if a test sets more than half its fields to non-default values, keep all fields explicit for documentation clarity; otherwise use `..Default::default()`.

Helpers in `coordinator_test.rs`, `executor_test.rs`, and `preflight_test.rs` call `common::make_item` or `common::make_in_progress_item` and then mutate specific fields — these do not construct `BacklogItem` directly and need no changes since they build on the already-updated helpers.

**Followups:**

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met:
  - [x] `BacklogItem` has a `Default` impl with correct values for all 22 fields
  - [x] `ItemStatus` implements `Default` with `New` as default variant
  - [x] Production construction sites use `..Default::default()`
  - [x] All existing tests pass without assertion changes
  - [x] No change to serialization/deserialization behavior
  - [x] Unit test verifies `BacklogItem::default()` values
  - [x] Migration sites use `..Default::default()` where appropriate
  - [x] Test helpers use `..Default::default()`
- [x] Tests pass
- [x] No regressions introduced
- [x] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | complete | 1ed5d1e | Added Default derives, 2 new tests, all 528 tests pass |
| 2 | complete | 72aeb9b | Updated 4 construction sites to use ..Default::default(), all 528 tests pass |
| 3 | complete | b6b0119 | Updated 4 test helpers and 6 inline constructions to use ..Default::default(), all 528 tests pass |

## Followups Summary

No followups identified during implementation or review. All phases completed cleanly with no workarounds, concerns, or deferred work.

### Critical

None

### High

None

### Medium

None

### Low

None

## Design Details

### Key Types

No new types introduced. Changes are derive additions to existing types:

```rust
// ItemStatus — add Default derive with #[default] on New
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    #[default]
    New,
    Scoping,
    Ready,
    InProgress,
    Done,
    Blocked,
}

// BacklogItem — add Default derive
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct BacklogItem {
    // ... all 22 fields unchanged
}
```

### Design Rationale

`#[derive(Default)]` chosen over manual `impl Default` because all field types produce correct derived defaults without customization. This follows the `StructuredDescription` precedent in the same file and is automatically maintained when fields are added or removed. See Design doc for full alternatives analysis.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
