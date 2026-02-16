# Change: Add BacklogItem constructor to reduce manual field initialization

**Status:** Proposed
**Created:** 2026-02-13
**Author:** Orchestrator (autonomous)

## Problem Statement

The `BacklogItem` struct has 22 fields. Of these, 5 are "required" (must be set at every construction site: `id`, `title`, `status`, `created`, `updated`) and 17 have natural defaults (`None` for Option fields, `Vec::new()` for Vec fields, `false` for `requires_human_review`). Every construction site must manually specify all 22 fields, even though only 5-8 fields vary between call sites (the 5 required fields plus a handful of optional fields like `origin`, `size`, `risk`).

There are 20 construction sites across the codebase:
- **3 in production code** (`backlog.rs`: `add_item`, `ingest_follow_ups`, `ingest_inbox_items`)
- **2 in migration code** (`migration.rs`: `map_v1_item`, `map_v2_item`)
- **15 in test code** (across `types_test.rs`, `prompt_test.rs`, `scheduler_test.rs`, `worklog_test.rs`, and `common/mod.rs`)

Each construction site repeats 15-17 lines of boilerplate setting optional fields to `None`, vectors to `Vec::new()`, and booleans to `false`. This creates two problems:

1. **Maintenance burden** — Adding a new field to `BacklogItem` requires updating all 20 construction sites, even when the new field has a natural default. The WRK-028 change (which added a `description` field) and subsequent additions of `phase_pool` and `last_phase_commit` each required touching every construction site.

2. **Error surface** — When a new optional field is added, the compiler forces all 20 sites to be updated (good), but each site must remember to set the correct default (redundant). The "correct" value is always `None`/`Vec::new()`/`false` for new optional fields, making the explicit specification pure noise.

## User Stories / Personas

- **Orchestrator developer** — Adds new fields to `BacklogItem` as the schema evolves. Wants to add a field with a default in one place, not update 20 construction sites with identical boilerplate. Currently every new optional field requires mechanical edits across production, migration, and test code.

## Desired Outcome

Construction sites specify only the fields that differ from defaults. Adding a new optional field to `BacklogItem` requires updating only the struct definition and any sites that need a non-default value — not every construction site.

## Success Criteria

### Must Have

- [ ] `BacklogItem` has a `Default` impl where all Option fields are `None`, all Vec fields are empty, `requires_human_review` is `false`, `status` is `ItemStatus::New`, and `id`/`title`/`created`/`updated` are empty strings
- [ ] `ItemStatus` implements `Default` with `New` as the default variant
- [ ] Production construction sites in `backlog.rs` (`add_item`, `ingest_follow_ups`, `ingest_inbox_items`) use `..Default::default()` struct update syntax (the Rust pattern that fills unspecified fields with their default values), specifying only fields that differ from defaults
- [ ] All existing tests pass without modification to assertions (behavior is unchanged)
- [ ] No change to serialization/deserialization behavior — YAML round-trip tests continue to pass. The `Default` impl is only used in Rust struct literal construction; it does not affect serde deserialization. No `#[serde(default)]` attributes are added or removed on any field.
- [ ] A unit test verifies that `BacklogItem::default()` produces the expected values for all 22 fields

### Should Have

- [ ] Migration construction sites in `migration.rs` (`map_v1_item`, `map_v2_item`) use `..Default::default()` for fields that didn't exist in earlier schema versions, while explicitly setting fields mapped from the old struct
- [ ] Test helper functions (`make_item` in `common/mod.rs`, `prompt_test.rs`, `scheduler_test.rs`, `worklog_test.rs`) use `..Default::default()` to reduce test boilerplate
- [ ] Test construction sites in `types_test.rs` use `..Default::default()` where they currently specify `None`/empty defaults, except in tests that intentionally assert on default values (where explicit specification aids readability)

### Nice to Have

- [ ] None currently identified

## Scope

### In Scope

- Adding `impl Default for BacklogItem` in `types.rs`
- Updating all `BacklogItem { ... }` construction sites to use struct update syntax (`..Default::default()`)
- Verifying all tests pass

### Out of Scope

- Adding `Default` impls for other types (`BacklogFile`, `PhaseResult`, `FollowUp`, etc.) — can be done as separate follow-ups if valuable
- Changing the `BacklogItem` struct's field layout, types, or serde attributes
- Adding builder pattern or constructor functions beyond `Default` — `Default` + struct update syntax is sufficient for current needs
- Changing any runtime behavior — this is a pure refactor

## Constraints

- The `Default` impl must derive or implement values that are safe sentinels. For `id`, `title`, `created`, and `updated`, empty strings are acceptable defaults since every construction site overwrites them. These fields are never used at their default values. No `#[serde(default)]` attributes are added to these fields — they remain required in YAML.
- The Rust `Default` trait and serde's `#[serde(default)]` attribute are independent mechanisms. This change only adds a Rust `Default` impl for use in struct literal construction (`..Default::default()`). It does not change how YAML deserialization handles missing fields.
- Migration code maps fields from older struct versions — migration construction sites should use `..Default::default()` for fields that didn't exist in older versions, but must explicitly set fields that are mapped from the old struct.
- The `#[derive(Default)]` approach requires `ItemStatus` to also impl `Default`. Since `ItemStatus::New` is the natural default (all items start as New), this is safe. All other field types already support derivation: `Option<T>` defaults to `None`, `Vec<T>` to empty, `String` to empty, `bool` to `false`.

## Dependencies

- **Depends On:** None
- **Blocks:** None — but every future field addition to `BacklogItem` benefits: adding a new optional field requires only (1) adding the field to the struct definition and (2) updating construction sites that need a non-default value. Sites using `..Default::default()` need no changes.

## Risks

- [ ] Low risk: Incorrect default for a field could silently produce wrong behavior. Mitigation: all defaulted fields are already `None`/empty/`false` at every construction site, so the defaults exactly match current explicit values. Tests verify behavior is unchanged.

## Assumptions

- Empty string is an acceptable default for `id`, `title`, `created`, and `updated` since all construction sites overwrite these immediately. No code path creates a BacklogItem and uses the default values for these fields.
- `ItemStatus::New` is the correct default status — all three production construction sites create items with `New` status.
- `#[derive(Default)]` on `ItemStatus` (defaulting to `New`) is preferred over a manual `Default` impl for simplicity, but a manual impl is also acceptable.
- The migration construction sites benefit from `Default` for fields that didn't exist in earlier schema versions (e.g., `description`, `phase_pool`, `last_phase_commit` didn't exist in v1/v2).

## Open Questions

- [ ] Should `ItemStatus` use `#[derive(Default)]` with `#[default]` attribute on `New`, or a manual `impl Default`? Both work; `#[derive(Default)]` with `#[default]` is more idiomatic in modern Rust (stable since Rust 1.62). Recommendation: use `#[derive(Default)]` with `#[default]` on `New` for both `ItemStatus` and `BacklogItem`.

## References

- `orchestrator/src/types.rs` — `BacklogItem` struct definition (lines 186-227)
- `orchestrator/src/backlog.rs` — Production construction sites: `add_item` (L146-169), `ingest_follow_ups` (L277-299), `ingest_inbox_items` (L364-387)
- `orchestrator/src/migration.rs` — Migration construction sites: `map_v1_item`, `map_v2_item`
- `orchestrator/tests/` — 15 test construction sites across multiple test files
