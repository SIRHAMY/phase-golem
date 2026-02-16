# Change: Add BacklogItem Constructor to Reduce Field-Listing Duplication

**Status:** Proposed
**Created:** 2026-02-13
**Author:** AI (autonomous)

## Problem Statement

`BacklogItem` has 22 fields, of which only 3 are truly required for creation (`id`, `title`, `status`). The remaining 19 fields have sensible defaults (`None`, `Vec::new()`, `false`, or a generated timestamp). Despite this, every construction site must explicitly list all 22 fields — even when most are set to their default values.

There are 5 construction sites in production code (`backlog.rs` × 3, `migration.rs` × 2) and 33+ in tests. The three `backlog.rs` sites (`add_item`, `ingest_follow_ups`, `ingest_inbox_items`) are nearly identical — they differ only in which 2-3 non-default fields are set (e.g., `origin`, `size`, `risk`, `impact`, `dependencies`). Adding a new field to `BacklogItem` requires updating every single construction site, even if the new field has a default value.

This creates unnecessary maintenance burden and makes it harder to evolve the struct over time.

## User Stories / Personas

- **Orchestrator developer** — Wants to add new optional fields to `BacklogItem` without updating 38+ construction sites. Wants to create items concisely by specifying only the fields that differ from defaults.

## Desired Outcome

Construction of `BacklogItem` instances requires specifying only the fields that differ from defaults. Adding a new optional field to the struct requires changes only at the definition site (and any call sites that actually use the new field), not at every construction site.

## Success Criteria

### Must Have

- [ ] `BacklogItem` implements the `Default` trait, with all `Option` fields defaulting to `None`, `Vec` fields to empty, `bool` fields to `false`, `String` fields to `""`, and `status` to `ItemStatus::New`
- [ ] The three production construction sites in `backlog.rs` (`add_item`, `ingest_follow_ups`, `ingest_inbox_items`) use `..Default::default()` and list only the fields that differ from defaults (reducing each from ~22 lines to ~5-8 lines)
- [ ] The two migration sites in `migration.rs` use `..Default::default()` for fields not present in older schema versions (fields that are mapped from the source struct remain explicit)
- [ ] All existing tests pass without behavior changes
- [ ] Adding a new `Option`-typed field to `BacklogItem` no longer requires touching construction sites that don't use that field

### Should Have

- [ ] Test helpers (`tests/common/mod.rs::make_item`, per-file `make_item` variants) are simplified using the new mechanism
- [ ] The `created` and `updated` timestamp fields default to `""` (empty string), allowing callers to override with real timestamps when needed

### Nice to Have

- [ ] Inline test constructions in `types_test.rs`, `prompt_test.rs`, etc. are simplified

## Scope

### In Scope

- Adding an `impl Default for BacklogItem` with explicit field defaults
- Updating the three `backlog.rs` construction sites to use `..Default::default()`
- Updating the two `migration.rs` construction sites to use `..Default::default()` for unmapped fields
- Updating test helpers to use `..Default::default()`
- The `created`/`updated` fields default to `""` (callers that need real timestamps override them)

### Out of Scope

- Changing the serialization format or YAML schema
- Adding builder pattern (over-engineering for this use case)
- Changing field types or semantics
- Modifying `BacklogFile` or other types

## Non-Functional Requirements

- **Performance:** No runtime cost — struct update syntax is zero-cost in Rust

## Constraints

- Must remain compatible with serde deserialization from existing YAML files. All optional fields already have `#[serde(default)]` attributes; `created` and `updated` do not (they are always present in YAML). The `Default` impl does not affect deserialization behavior — it only provides defaults for struct literal construction in Rust code
- Using a manual `impl Default` (not `#[derive(Default)]`) because `status` must default to `ItemStatus::New` rather than requiring `ItemStatus` to implement `Default`

## Dependencies

- **Depends On:** Nothing
- **Blocks:** Nothing directly, but unblocks easier future field additions

## Risks

- [ ] Low risk: If `Default` uses empty-string timestamps, any code path that forgets to set `created`/`updated` would silently produce items with empty timestamps. Mitigation: the existing call sites already set timestamps explicitly; we'd only be enabling the same mistake that's already possible today.

## Open Questions

None — all decisions resolved in Assumptions section.

## Assumptions

- **`impl Default` over `#[derive(Default)]`** — Manual impl needed because `status` must default to `ItemStatus::New`, and `ItemStatus` doesn't (and shouldn't) implement `Default`. A manual impl also makes the default values explicit and documentable.
- **`Default` impl over constructor function** — `Default` enables struct update syntax (`..Default::default()`), which is the standard Rust pattern for specifying only non-default fields. A constructor would require choosing parameters — any subset is arbitrary.
- **Empty string `""` for timestamp defaults** — Using `""` rather than `Utc::now()` because `Default` should be pure and deterministic. Empty strings are obviously invalid if accidentally persisted, making bugs easy to spot. Callers that need real timestamps (production code) set them explicitly; callers that don't care (tests) can leave them or use fixed values.
- **Migration sites use `..Default::default()` for unmapped fields only** — Migration functions map most fields explicitly from the source struct. `..Default::default()` handles fields that don't exist in older schema versions, replacing the current pattern of explicitly listing `None` for each new field.

## References

- `orchestrator/src/types.rs:186-227` — `BacklogItem` struct definition
- `orchestrator/src/backlog.rs:146-169` — `add_item` construction
- `orchestrator/src/backlog.rs:277-300` — `ingest_follow_ups` construction
- `orchestrator/src/backlog.rs:364-387` — `ingest_inbox_items` construction
- `orchestrator/src/migration.rs:145-168` — `map_v1_item` construction
- `orchestrator/src/migration.rs:417-440` — `map_v2_item` construction
- `orchestrator/tests/common/mod.rs:22-46` — `make_item` test helper
