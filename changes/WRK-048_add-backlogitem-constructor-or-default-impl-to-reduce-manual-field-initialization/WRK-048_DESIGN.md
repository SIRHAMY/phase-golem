# Design: Add BacklogItem Default impl

**ID:** WRK-048
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-048_add-backlogitem-constructor-or-default-impl-to-reduce-manual-field-initialization_PRD.md
**Tech Research:** ./WRK-048_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add `#[derive(Default)]` to `BacklogItem` and `ItemStatus`, then update all construction sites to use struct update syntax (`..Default::default()`). This eliminates 15-17 lines of boilerplate per construction site by letting Rust's `Default` derive fill `Option` fields with `None`, `Vec` fields with `Vec::new()`, `bool` fields with `false`, and `String` fields with `""`. The approach follows the existing `StructuredDescription` pattern already established in `types.rs`.

**Important:** Rust's `Default` trait and serde's `#[serde(default)]` attribute are independent mechanisms. This change only adds a Rust `Default` impl for use in struct literal construction (`..Default::default()`). It does not change how YAML deserialization handles missing fields. Fields without `#[serde(default)]` (like `id`, `title`, `created`, `updated`, `status`) remain required in YAML — missing them causes a deserialization error regardless of whether `Default` is implemented.

---

## System Design

### High-Level Architecture

No new components or modules. This is a pure refactor within existing code:

1. **types.rs** — Add `#[derive(Default)]` to `ItemStatus` (with `#[default]` on `New`) and `BacklogItem`
2. **Construction sites** — Replace explicit default-valued fields with `..Default::default()`
3. **New test** — Add `test_backlogitem_default` in `types_test.rs` that calls `BacklogItem::default()` and asserts all 22 fields individually: `id == ""`, `title == ""`, `status == ItemStatus::New`, all `Option` fields are `None`, both `Vec` fields are empty, `requires_human_review == false`, `created == ""`, `updated == ""`

### Component Breakdown

#### ItemStatus Default (types.rs)

**Purpose:** Enable `#[derive(Default)]` on `BacklogItem` by providing a `Default` impl for `ItemStatus`.

**Changes:**
- Add `Default` to ItemStatus's derive list: `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]`
- Add `#[default]` attribute to the `New` variant

**Dependencies:** None — `#[default]` on enum variants is stable since Rust 1.62.

#### BacklogItem Default (types.rs)

**Purpose:** Provide sensible defaults for all 22 fields so construction sites can use struct update syntax.

**Changes:**
- Add `Default` to BacklogItem's derive list: `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]`

**Derived defaults (all correct without customization):**

| Field | Type | Default Value |
|-------|------|---------------|
| `id` | `String` | `""` |
| `title` | `String` | `""` |
| `status` | `ItemStatus` | `ItemStatus::New` |
| `phase` | `Option<String>` | `None` |
| `size` | `Option<SizeLevel>` | `None` |
| `complexity` | `Option<DimensionLevel>` | `None` |
| `risk` | `Option<DimensionLevel>` | `None` |
| `impact` | `Option<DimensionLevel>` | `None` |
| `requires_human_review` | `bool` | `false` |
| `origin` | `Option<String>` | `None` |
| `blocked_from_status` | `Option<ItemStatus>` | `None` |
| `blocked_reason` | `Option<String>` | `None` |
| `blocked_type` | `Option<BlockType>` | `None` |
| `unblock_context` | `Option<String>` | `None` |
| `tags` | `Vec<String>` | `[]` |
| `dependencies` | `Vec<String>` | `[]` |
| `created` | `String` | `""` |
| `updated` | `String` | `""` |
| `pipeline_type` | `Option<String>` | `None` |
| `description` | `Option<StructuredDescription>` | `None` |
| `phase_pool` | `Option<PhasePool>` | `None` |
| `last_phase_commit` | `Option<String>` | `None` |

**Prerequisite:** `ItemStatus`, `SizeLevel`, `DimensionLevel`, `BlockType`, `StructuredDescription`, and `PhasePool` must all implement `Default`. `StructuredDescription` already does. The others are `Option`-wrapped so their `Default` impl is not required — `Option<T>` defaults to `None` regardless of whether `T` implements `Default`.

**Correction:** Only `ItemStatus` needs a `Default` impl added because it appears as a direct (non-Option) field (`status: ItemStatus`). All other enum types (`SizeLevel`, `DimensionLevel`, `BlockType`, `PhasePool`) appear only inside `Option<T>` wrappers, so their `Default` is not needed for the derive.

#### Construction Site Updates

**Purpose:** Replace boilerplate field initialization with struct update syntax.

**Pattern — Before:**
```rust
BacklogItem {
    id: new_id,
    title: item_title,
    status: ItemStatus::New,
    phase: None,
    size: None,
    complexity: None,
    risk: None,
    impact: None,
    requires_human_review: false,
    origin: Some("inbox".to_string()),
    blocked_from_status: None,
    blocked_reason: None,
    blocked_type: None,
    unblock_context: None,
    tags: Vec::new(),
    dependencies: Vec::new(),
    created: now.clone(),
    updated: now,
    pipeline_type: None,
    description: None,
    phase_pool: None,
    last_phase_commit: None,
}
```

**Pattern — After:**
```rust
BacklogItem {
    id: new_id,
    title: item_title,
    status: ItemStatus::New,
    origin: Some("inbox".to_string()),
    created: now.clone(),
    updated: now,
    ..Default::default()
}
```

**Sites to update (3 categories):**

1. **Production** (`backlog.rs`): `add_item`, `ingest_follow_ups`, `ingest_inbox_items` — each sets `id`, `title`, `status`, `created`, `updated`, plus 1-3 optional fields
2. **Migration** (`migration.rs`): `map_v1_item`, `map_v2_item` — explicitly set fields mapped from old schema, default the rest
3. **Tests** (across `types_test.rs`, `prompt_test.rs`, `scheduler_test.rs`, `worklog_test.rs`, `common/mod.rs`) — update helpers and inline constructions

**Migration pattern example** — Migration sites must explicitly set all fields mapped from the old schema version, using `..Default::default()` only for fields that didn't exist in that version:

```rust
// map_v1_item: v1 had id, title, status, created, updated, and a few others
// Fields like description, phase_pool, last_phase_commit didn't exist in v1
BacklogItem {
    id: old.id,
    title: old.title,
    status: map_v1_status(old.status),
    origin: old.origin,
    tags: old.tags,
    dependencies: old.dependencies,
    created: old.created,
    updated: old.updated,
    // Fields not in v1 schema get defaults via struct update syntax
    ..Default::default()
}
```

### Data Flow

No change to data flow. `BacklogItem` instances are constructed identically at runtime — the only difference is syntactic (fewer lines of source code).

### Key Flows

#### Flow: Adding a new optional field to BacklogItem (future maintenance)

> The primary maintenance benefit of this change.

1. **Add field** — Developer adds new `Option<T>` field to `BacklogItem` struct in `types.rs` with `#[serde(default, skip_serializing_if = "Option::is_none")]`
2. **Compile** — Sites using `..Default::default()` compile without changes (new field defaults to `None`)
3. **Update specific sites** — Only sites that need a non-default value for the new field need updating
4. **Run tests** — Existing tests pass; update the `BacklogItem::default()` unit test to assert the new field's default

**Before this change:** Step 2 would fail — every construction site must be updated to include the new field, even when it defaults to `None`.

**Note:** This flow applies to `Option<T>`, `Vec<T>`, and `bool` fields. Adding a non-optional field (e.g., a new required `String`) would still require updating all construction sites, since the compiler enforces that non-defaulted fields are explicitly set when `..Default::default()` is used. This is the same behavior as today.

---

## Technical Decisions

### Key Decisions

#### Decision: Use `#[derive(Default)]` rather than manual `impl Default`

**Context:** Both approaches provide the same functionality. Manual impl allows custom default values.

**Decision:** Use `#[derive(Default)]` for both `ItemStatus` and `BacklogItem`.

**Rationale:** All derived defaults match the values currently used at every construction site. No custom logic is needed. This follows the existing `StructuredDescription` pattern in the codebase. Derived impls are automatically maintained when fields are added/removed.

**Consequences:** If a future field needs a non-standard default (e.g., a `bool` that defaults to `true`), the derive would need to be replaced with a manual impl. This is straightforward and unlikely given current usage patterns.

#### Decision: Only add Default to ItemStatus, not other enums

**Context:** `SizeLevel`, `DimensionLevel`, `BlockType`, and `PhasePool` are also enums in types.rs.

**Decision:** Only add `Default` to `ItemStatus` because it's the only enum used as a direct (non-Option) field in `BacklogItem`.

**Rationale:** `Option<T>` defaults to `None` without requiring `T: Default`. Adding `Default` to other enums is unnecessary for this change and would be scope creep. Can be done separately if needed.

**Consequences:** If a future change uses one of these enums as a direct field, its `Default` would need to be added then.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Loss of compile-time enforcement for 4 fields | Construction sites using `..Default::default()` won't get a compiler error if they forget to set `id`, `title`, `created`, or `updated` — these would silently default to `""` | Elimination of 15-17 lines of boilerplate per construction site and easier future field additions (new optional fields require zero construction site changes) | All existing construction sites set these fields. A unit test verifies default values are correct. Code review catches missing required fields. This matches the established `StructuredDescription` pattern. The PRD explicitly evaluated and rejected the builder pattern alternative as over-engineering for ~20 construction sites. |

---

## Alternatives Considered

### Alternative: Builder Pattern

**Summary:** Create a `BacklogItemBuilder` with methods like `.id()`, `.title()`, `.build()` that enforces required fields at compile time.

**How it would work:**
- Builder struct with `Option` wrappers for all fields
- `.build()` returns `Result<BacklogItem, Error>` or panics if required fields missing
- Type-state builder pattern could enforce required fields at compile time

**Pros:**
- Compile-time enforcement of required fields
- Self-documenting API

**Cons:**
- Significant new code (~50-80 lines) for a struct that's only constructed in 20 places
- Builder pattern is not idiomatic for simple data structs in this codebase
- Over-engineering for the problem (PRD explicitly scopes this out)
- Doesn't follow existing patterns (`StructuredDescription` uses `Default`, not a builder)

**Why not chosen:** Over-engineered for this use case. `Default` + struct update syntax is simpler, idiomatic, and sufficient. The PRD explicitly excludes builder patterns.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Forgotten required field at a construction site using `..Default::default()` | Item created with empty id/title/timestamps | Low — all existing sites already set these fields | Unit test for `BacklogItem::default()` + code review convention |
| Incorrect default for a field | Silent behavioral change | Very Low — all derived defaults exactly match current explicit values at every site | Unit test verifying all 22 field defaults |

---

## Integration Points

### Existing Code Touchpoints

- `types.rs` (L5-14) — Add `Default` derive + `#[default]` attribute to `ItemStatus` enum
- `types.rs` (L186-227) — Add `Default` derive to `BacklogItem` struct
- `backlog.rs` — Update 3 construction sites in `add_item`, `ingest_follow_ups`, `ingest_inbox_items`
- `migration.rs` — Update 2 construction sites in `map_v1_item`, `map_v2_item`
- Test files — Update ~15 construction sites across `types_test.rs`, `prompt_test.rs`, `scheduler_test.rs`, `worklog_test.rs`, `common/mod.rs`
- New test — Add unit test for `BacklogItem::default()` field values

### External Dependencies

None.

---

## Open Questions

None — all questions from the PRD have been resolved by tech research.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements — Default derive, construction site updates, unit test, no serde changes
- [x] Key flows are documented — future field addition flow documented
- [x] Tradeoffs are explicitly documented — compile-time enforcement loss documented with mitigations
- [x] Integration points with existing code are identified — all files and line ranges listed
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Initial design draft (light mode) | Straightforward derive(Default) approach, matching existing StructuredDescription pattern |
| 2026-02-13 | Self-critique (7 agents) | Architecture: clean. Recurring theme across agents: compile-time enforcement loss for required fields. Classified as documented tradeoff per PRD scope (builder pattern explicitly excluded). Auto-fixed: added serde independence clarification, migration pattern example, unit test specification, future field flow details, strengthened tradeoff documentation. |
| 2026-02-13 | Finalized design | Ready for SPEC |
