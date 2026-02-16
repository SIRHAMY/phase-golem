# Technical Research: Add BacklogItem Default Impl

**Status:** Complete
**Mode:** Light
**Date:** 2026-02-13
**PRD:** WRK-049_PRD.md

## External Research

### Landscape Overview

The pattern of implementing `Default` for a struct and using struct update syntax (`..Default::default()`) is a well-established, idiomatic Rust pattern. It is documented in The Rust Book, Google's Comprehensive Rust course, and the Rust Design Patterns book as the recommended approach for structs with many optional fields. This is precisely the right fit for `BacklogItem` — 22 fields where only 3 need explicit values in most constructions.

### Patterns Considered

| Pattern | Pros | Cons | Recommendation |
|---------|------|------|----------------|
| Manual `impl Default` + struct update syntax | Idiomatic, explicit defaults, zero runtime cost, works on stable Rust | Must keep impl in sync with struct; compiler error on missing field is a safety feature | **Use this** |
| `#[derive(Default)]` | Zero boilerplate, auto-syncs with new fields | Requires `ItemStatus` to implement `Default`; less explicit control over defaults | Not suitable — `ItemStatus` shouldn't implement `Default` |
| Builder pattern | Can enforce required fields at compile time | Overkill when all fields are `pub` and no validation needed | Over-engineered for this use case |
| RFC 3681 (default field values) | Most concise, inline defaults | Unstable/nightly-only as of 2026; not ready for production | Not available on stable Rust |

### Serde Interaction — No Conflicts

The current codebase uses **field-level** `#[serde(default)]` on optional fields. These annotations use the **type's own `Default`** (e.g., `Option::default()` -> `None`) — NOT the struct-level `impl Default`. Since our proposed defaults match the type defaults (`None`, `false`, `Vec::new()`, `String::new()`), there is no conflict between serde deserialization and programmatic construction.

Key distinction:
- **Field-level `#[serde(default)]`**: Used during YAML deserialization for missing fields
- **`impl Default for BacklogItem`**: Used during programmatic construction via `..Default::default()`
- These are independent mechanisms that happen to agree on values

The `created` and `updated` fields do NOT have `#[serde(default)]` (they are always present in YAML), so the `impl Default` only affects programmatic construction — no deserialization impact.

### Common Pitfalls

1. **Forgetting to override "required" fields**: `..Default::default()` won't warn if `id` or `title` are left as empty strings. Mitigation: all production call sites already set these explicitly; code review catches this.

2. **New field sync**: Adding a field to `BacklogItem` without adding it to the `Default` impl causes a compile error. This is a safety feature, not a bug — it forces explicit default decisions.

3. **Private fields limitation**: Struct update syntax requires all fields to be visible. Since all `BacklogItem` fields are `pub`, this is not an issue.

## Internal Research

### Current State

- **41 total construction sites**: 5 production (3 `backlog.rs`, 2 `migration.rs`) + 36 test
- **22 fields** on `BacklogItem`, of which only 3 (`id`, `title`, `status`) are truly required
- Every construction site explicitly lists all 22 fields, even when most are default values

### Existing Patterns Supporting This Change

1. **Manual `impl Default` already in use**: `ProjectConfig`, `GuardrailsConfig`, and `ExecutionConfig` in `config.rs` all have manual `Default` implementations — this is an established pattern in the codebase.

2. **Struct update syntax already in use**: `prompt_test.rs` has two test helpers using `..make_item()` struct update syntax. Replacing `make_item()` with `..Default::default()` is a natural evolution.

3. **`StructuredDescription` already derives `Default`**: Shows the codebase is comfortable with `Default` on data structs.

### Relevant Files

| File | Purpose | Impact |
|------|---------|--------|
| `orchestrator/src/types.rs:186-227` | BacklogItem definition | Add `impl Default` below struct |
| `orchestrator/src/backlog.rs` | 3 production construction sites | Simplify with `..Default::default()` |
| `orchestrator/src/migration.rs` | 2 migration construction sites | Partial simplification (most fields mapped from source) |
| `orchestrator/tests/common/mod.rs:21-46` | `make_item` test helper | Simplify with `..Default::default()` |
| `orchestrator/tests/` (8 test files) | 36 test construction sites | Optional simplification |

### Migration Site Details

The two `migration.rs` construction sites (`map_v1_item` at line 145, `map_v2_item` at line 417) map most fields explicitly from source structs. The benefit of `..Default::default()` here is limited to fields that don't exist in older schema versions (e.g., `pipeline_type`, `description`, `phase_pool`, `last_phase_commit` in `map_v1_item`). Both sites currently list these as explicit `None` or `Some(...)`.

## PRD Concerns

**No significant concerns.** The PRD's approach aligns perfectly with the standard Rust idiom:

1. **PRD correctly chose manual `impl Default` over `#[derive(Default)]`** — `ItemStatus` should not implement `Default` since there is no universally correct default status outside of construction contexts.

2. **PRD correctly chose empty string `""` for timestamp defaults** — `Default` should be pure/deterministic. Empty strings are obviously invalid if persisted, making bugs easy to spot.

3. **PRD correctly scoped migration sites** — `map_v1_item` and `map_v2_item` map most fields from source; `..Default::default()` only covers truly unmapped fields.

## Critical Areas

1. **Ensuring all production call sites still set `created`/`updated` explicitly** — With `..Default::default()` providing empty strings, any production code path that forgets to set timestamps would silently produce items with empty timestamps. However, this risk already exists (callers could set any field to empty string today), and all current call sites explicitly set timestamps.

## Open Questions

None. All decisions resolved in the PRD's Assumptions section. The research confirms the PRD's approach is idiomatic and correct.

## Recommendations for Design

1. **Use the approach exactly as specified in the PRD** — Manual `impl Default for BacklogItem` with struct update syntax. This is the standard Rust idiom and the codebase already has precedent for manual Default impls.

2. **Consider a `BacklogItem::new(id, title, status)` constructor** (optional enhancement): This would wrap `..Default::default()` internally and make required fields explicit at the type level. However, the PRD explicitly excludes this ("Out of Scope: Adding builder pattern"), and the simpler `Default` + struct update approach is sufficient.

3. **Migrate test helpers early** — The `make_item` helper in `tests/common/mod.rs` is the highest-leverage simplification target after production code.

## Key References

- [The Rust Book: Struct Update Syntax](https://doc.rust-lang.org/book/ch05-01-defining-structs.html)
- [std::default::Default](https://doc.rust-lang.org/std/default/trait.Default.html)
- [Comprehensive Rust: Default + Struct Update Syntax](https://google.github.io/comprehensive-rust/std-traits/default.html)
- [Rust Design Patterns: The Default Trait](https://rust-unofficial.github.io/patterns/idioms/default.html)
- [Serde: Default value for a field](https://serde.rs/attr-default.html)
- [Rust Forum: #[serde(default)] vs impl Default](https://users.rust-lang.org/t/serde-default-versus-impl-default/66773)
- [RFC 3681: Default Field Values (unstable)](https://rust-lang.github.io/rfcs/3681-default-field-values.html)

## Assumptions

- **No tech research template found** — Created this document following the structure described in the tech-research workflow. Documented all findings in a format consistent with the workflow's expectations.
