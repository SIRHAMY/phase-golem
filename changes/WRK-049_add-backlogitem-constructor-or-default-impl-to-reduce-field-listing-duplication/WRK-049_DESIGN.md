# Design: Add BacklogItem Default Impl

**ID:** WRK-049
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-049_PRD.md
**Tech Research:** ./WRK-049_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add a manual `impl Default for BacklogItem` with explicit field defaults, then update all construction sites to use struct update syntax (`..Default::default()`). This eliminates 15-17 lines of boilerplate per construction site by letting callers specify only the fields that differ from defaults. The approach follows the existing manual `Default` pattern established by `ProjectConfig`, `GuardrailsConfig`, and `ExecutionConfig` in `config.rs`.

---

## System Design

### High-Level Architecture

No new components or modules. This is a pure refactor within existing code:

1. **types.rs** — Add `impl Default for BacklogItem` with explicit field defaults
2. **Construction sites** — Replace explicit default-valued fields with `..Default::default()`
3. **New test** — Verify `BacklogItem::default()` produces expected values for all 22 fields (see Test Specification below)

### Component Breakdown

#### BacklogItem Default (types.rs)

**Purpose:** Provide sensible defaults for all 22 fields so construction sites can use struct update syntax.

**Changes:**
- Add `impl Default for BacklogItem` block after the struct definition (follows existing `config.rs` pattern)

**Default values:**

| Field | Type | Default Value | Rationale |
|-------|------|---------------|-----------|
| `id` | `String` | `""` | Placeholder; must be overridden at call sites |
| `title` | `String` | `""` | Placeholder; must be overridden at call sites |
| `status` | `ItemStatus` | `ItemStatus::New` | Most common initial status; reason manual impl is needed |
| `phase` | `Option<String>` | `None` | No phase assigned initially |
| `size` | `Option<SizeLevel>` | `None` | Unassessed |
| `complexity` | `Option<DimensionLevel>` | `None` | Unassessed |
| `risk` | `Option<DimensionLevel>` | `None` | Unassessed |
| `impact` | `Option<DimensionLevel>` | `None` | Unassessed |
| `requires_human_review` | `bool` | `false` | Default behavior |
| `origin` | `Option<String>` | `None` | No origin specified |
| `blocked_from_status` | `Option<ItemStatus>` | `None` | Not blocked |
| `blocked_reason` | `Option<String>` | `None` | Not blocked |
| `blocked_type` | `Option<BlockType>` | `None` | Not blocked |
| `unblock_context` | `Option<String>` | `None` | Not blocked |
| `tags` | `Vec<String>` | `vec![]` | No tags |
| `dependencies` | `Vec<String>` | `vec![]` | No dependencies |
| `created` | `String` | `""` | Placeholder; production callers override with `Utc::now()` |
| `updated` | `String` | `""` | Placeholder; production callers override with `Utc::now()` |
| `pipeline_type` | `Option<String>` | `None` | No pipeline specified |
| `description` | `Option<StructuredDescription>` | `None` | No description |
| `phase_pool` | `Option<PhasePool>` | `None` | No pool assigned |
| `last_phase_commit` | `Option<String>` | `None` | No commit recorded |

**Why manual impl, not derive:** `status` must default to `ItemStatus::New`. Using `#[derive(Default)]` would require adding `Default` to `ItemStatus` (with `#[default]` on `New`). While valid, the PRD explicitly chose manual impl because `ItemStatus` has no universally correct default — `New` is only appropriate for construction contexts, not as a general-purpose default. Manual impl also makes default values explicit and documentable, matching the `config.rs` pattern already established in the codebase.

**Serde interaction note:** This `impl Default` is used only for programmatic struct construction in Rust code (via `..Default::default()`). Serde deserialization uses field-level `#[serde(default)]` attributes independently — it does not invoke this impl. The two mechanisms are independent but agree on values (`None`, `false`, `Vec::new()`, `String::new()`) for all fields that have `#[serde(default)]`.

#### Test Specification

A unit test `test_backlogitem_default_all_fields` must verify the `Default` impl by constructing `BacklogItem::default()` and asserting every field's value against the defaults table above. This ensures:
- All 22 fields produce expected defaults
- Future changes to default values are caught by test failure
- The `Default` impl stays in sync with the struct definition (any new field missing from `Default` is a compile error; any incorrect default is a test failure)

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
    origin: Some("inbox".to_string()),
    created: now.clone(),
    updated: now,
    ..Default::default()
}
```

Note: fields that match the default (like `status: ItemStatus::New`) can be omitted since `Default` provides `ItemStatus::New`.

**Sites to update (3 categories):**

1. **Production** (`backlog.rs`):
   - `add_item` (L146-169): Sets `id`, `title`, `size`, `risk`, `created`, `updated` — rest default
   - `ingest_follow_ups` (L277-300): Sets `id`, `title`, `size` (from `fu.suggested_size`), `risk` (from `fu.suggested_risk`), `origin`, `created`, `updated` — rest default
   - `ingest_inbox_items` (L364-387): Sets `id`, `title`, `size`, `risk`, `impact`, `origin`, `dependencies`, `pipeline_type`, `created`, `updated` — rest default

2. **Migration** (`migration.rs`):
   - `map_v1_item` (L145-168): Maps 20 fields explicitly from v1 source (including transformations like `blocked_from_status` via `map_v1_status`). Only `description: None` and `last_phase_commit: None` are defaulted. Benefit: 2 fields saved.
   - `map_v2_item` (L417-440): Maps all 22 fields from v2 source; `description` is computed via `parse_description`. Benefit: 0 fields saved.

   **Decision:** Leave both migration sites as-is. Migration code maps nearly all fields from source structs, so `..Default::default()` saves at most 2 lines while making it less obvious which fields are explicitly mapped vs. defaulted. Migration functions prioritize auditability over brevity.

3. **Tests**:
   - `make_item` in `tests/common/mod.rs` (L22-46): Sets `id`, `title`, `status`, `created`, `updated` — rest default. Simplifies from 24 lines to ~7 lines.
   - `make_in_progress_item`: Already delegates to `make_item`, no direct change needed.
   - Inline test constructions across test files: Optional, can simplify gradually.

### Data Flow

No change to data flow. `BacklogItem` instances are constructed identically at runtime — the only difference is syntactic (fewer lines of source code).

### Key Flows

#### Flow: Adding a new optional field to BacklogItem (future maintenance)

> The primary maintenance benefit of this change.

1. **Add field** — Developer adds new `Option<T>` field to `BacklogItem` struct in `types.rs` with `#[serde(default, skip_serializing_if = "Option::is_none")]`
2. **Add to Default impl** — Developer adds `field_name: None` to the `impl Default` block (compiler enforces this via exhaustive struct literal)
3. **Compile** — Sites using `..Default::default()` compile without changes (new field defaults to `None`)
4. **Update specific sites** — Only sites that need a non-default value for the new field need updating

**Before this change:** Step 3 would fail — every construction site must be updated to include the new field, even when it defaults to `None`.

**Note:** Adding a field to the `Default` impl (step 2) is an extra step compared to `#[derive(Default)]`, but it's a single-line addition in one file, and the compiler error is clear about what's missing. This tradeoff is acceptable because it keeps default values explicit.

---

## Technical Decisions

### Key Decisions

#### Decision: Use manual `impl Default` rather than `#[derive(Default)]`

**Context:** Both approaches provide the same functionality. `#[derive(Default)]` requires adding `Default` to `ItemStatus`.

**Decision:** Use manual `impl Default for BacklogItem`.

**Rationale:**
- `ItemStatus` has no universally correct default — `New` is only appropriate for programmatic construction of fresh items, not as a general-purpose default for the enum
- Manual impl makes all default values explicit and documentable (matches `config.rs` precedent)
- The codebase already has three manual `Default` impls in `config.rs`; this follows the established pattern
- Tech research confirmed this as the idiomatic Rust approach

**Consequences:** Adding a new field to `BacklogItem` requires adding one line to the `Default` impl. The compiler enforces this (missing field = compile error). This is an acceptable tradeoff for explicit control.

#### Decision: Leave migration sites unchanged

**Context:** `map_v1_item` maps 20 fields from v1 source (including transformations); only `description` and `last_phase_commit` are defaulted. `map_v2_item` maps all fields from v2 source.

**Decision:** Leave both migration sites with explicit field listings. Do not use `..Default::default()`.

**Rationale:** Migration code maps nearly every field from a source struct, often with transformations (e.g., `blocked_from_status` via `map_v1_status`). Using `..Default::default()` for 0-2 remaining fields would obscure which fields are intentionally mapped vs. silently defaulted. Migration functions are one-time code that prioritizes auditability over brevity.

**Consequences:** Both sites will still need updating if a new field is added, but migrations are rarely written and each is a one-time operation. The explicit listing makes the field mapping auditable.

#### Decision: Empty string `""` for timestamp defaults

**Context:** `created` and `updated` could default to `""`, a placeholder date, or `Utc::now()`.

**Decision:** Default to `""` (empty string).

**Rationale:**
- `Default` should be pure and deterministic — no side effects like clock reads
- Empty strings are obviously invalid if accidentally persisted, making bugs easy to spot
- All production callers explicitly set timestamps; tests can use fixed values or leave empty
- Matches the approach specified in the PRD

**Consequences:** Any production code path that forgets to set timestamps would produce items with empty timestamps. This risk is low — all current call sites set timestamps explicitly, and the same mistake is equally possible today.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why Acceptable |
|----------|-----------------|-----------------|----------------|
| Loss of compile-time enforcement | Construction sites using `..Default::default()` won't get a compiler error if they forget to set `id`, `title`, `created`, or `updated` | Elimination of 15-17 lines of boilerplate per construction site and easier future field additions | All existing sites set these. A unit test for `BacklogItem::default()` guards defaults. Risk is low. |
| Manual `Default` maintenance | Adding a field requires updating the `Default` impl (one line) | Explicit, documentable defaults; `ItemStatus` stays without a `Default` impl | Compiler enforces the update; single line in one file. |

---

## Alternatives Considered

### Alternative: `#[derive(Default)]` on both `ItemStatus` and `BacklogItem`

**Summary:** Add `#[derive(Default)]` + `#[default]` attribute on `ItemStatus::New`. Zero custom code.

**How it would work:**
- Add `Default` to `ItemStatus` derive list, add `#[default]` on `New` variant
- Add `Default` to `BacklogItem` derive list
- All defaults are derived from type defaults

**Pros:**
- Zero boilerplate — no manual impl to maintain
- New fields automatically get defaults without updating `Default` impl

**Cons:**
- Implies `ItemStatus::New` is the universally correct default for `ItemStatus`, which is semantically misleading
- Less explicit about what defaults are — need to mentally derive from types
- Doesn't follow the existing manual `Default` pattern in `config.rs`

**Why not chosen:** The manual approach better communicates intent and doesn't require giving `ItemStatus` a potentially misleading `Default` impl. The marginal maintenance cost (one line per new field) is negligible.

### Alternative: `BacklogItem::new(id, title)` constructor

**Summary:** Add a constructor function that enforces required fields at the type level, returning a `BacklogItem` with defaults for everything else.

**How it would work:**
```rust
impl BacklogItem {
    pub fn new(id: String, title: String) -> Self {
        Self { id, title, ..Default::default() }
    }
}
```
Callers use: `BacklogItem { origin: Some("inbox".into()), ..BacklogItem::new(id, title) }`

**Pros:**
- Enforces `id` and `title` at the type level
- Makes required fields explicit at every call site

**Cons:**
- Still requires `impl Default` underneath (doesn't replace this change)
- Doesn't enforce `created`/`updated` timestamps (callers still override these manually)
- Choosing which fields are "required" constructor params is arbitrary — `status` is always `New` so shouldn't be a param, but `created`/`updated` are always set and aren't params either
- PRD explicitly scopes this out as unnecessary layering

**Why not chosen:** The constructor adds a layer on top of `Default` without meaningfully improving safety. The fields it would enforce (`id`, `title`) are already set at every call site. If a constructor is desired later, it can be added on top of this change without modifying it.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Forgotten required field at construction site | Item created with empty id/title/timestamps | Low — all sites already set these | Unit test for defaults + code review |
| Incorrect default value in manual impl | Silent behavioral change | Very Low — values match current explicit usage at every site | Unit test verifying all 22 defaults |
| New field not added to Default impl | Compile error (caught immediately) | N/A | Compiler enforces this |

---

## Integration Points

### Existing Code Touchpoints

- `types.rs` (after L227) — Add `impl Default for BacklogItem` block (~25 lines)
- `backlog.rs` (L146-169) — Simplify `add_item` construction
- `backlog.rs` (L277-300) — Simplify `ingest_follow_ups` construction
- `backlog.rs` (L364-387) — Simplify `ingest_inbox_items` construction
- `migration.rs` (L145-168, L417-440) — Leave as-is (explicit field mapping for auditability)
- `tests/common/mod.rs` (L22-46) — Simplify `make_item` helper
- Test files — Optional simplification of inline constructions

### External Dependencies

None.

---

## Open Questions

None — all decisions resolved by the PRD and tech research.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD must-have requirements — manual Default impl, construction site updates, unit test, no serde changes
- [x] Key flows are documented — future field addition flow documented
- [x] Tradeoffs are explicitly documented — compile-time enforcement loss and manual maintenance cost
- [x] Integration points with existing code are identified — all files and line ranges listed
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Initial design draft (light mode) | Manual `impl Default` approach, following `config.rs` precedent |
| 2026-02-13 | Self-critique (7 agents) | No critical issues. Auto-fixed: clarified migration decision (leave as-is), added test specification, added constructor alternative, added serde interaction note. No directional items requiring input. |
| 2026-02-13 | Finalized design | Ready for SPEC |
