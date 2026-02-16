# Technical Research: WRK-002 — Optimize BACKLOG.yaml to Single Parse Pass

**Status:** Complete
**Mode:** Light
**Created:** 2026-02-13

## External Research

### Landscape Overview

The "parse once to `Value`, extract a discriminator field, then convert to a typed struct via `from_value()`" pattern is a well-established idiom in the Rust serde ecosystem. It works across `serde_json`, `serde_yaml`, `serde_yaml_ng`, and `serde_yml`. The core principle: serde's `Deserializer` trait provides a uniform abstraction, so all `#[serde(...)]` attributes work identically whether the deserializer is backed by raw text (`from_str`) or an in-memory `Value` (`from_value`).

The single meaningful difference is **error reporting**: `from_value()` loses source-file location information (line/column numbers) because the `Value` tree does not retain positional metadata from the original text.

### Common Patterns

| Pattern | How It Works | Tradeoffs |
|---------|-------------|-----------|
| **Parse-Once-Then-Convert** (recommended) | Parse to `Value`, extract discriminator, `from_value()` to typed struct | Eliminates redundant parsing; loses line/column in errors |
| **Direct Typed Deserialization** (current) | `from_str::<T>()` directly on raw string | Simplest; best error messages; requires double parse if version check needed |
| **Enum-Based Version Dispatch** | `#[serde(tag = "schema_version")]` on enum | Single parse, no manual extraction; poor errors with `untagged`; integer tags problematic in YAML |

### Key Finding: `#[serde(default)]` Behavior

`#[serde(default)]` operates at the serde `Deserialize` trait level, not at the deserializer level. When a field is absent from the `Value` map, serde calls `Default::default()` exactly as it would when the field is absent from the raw YAML text. **Behavior is identical between `from_value()` and `from_str()`.**

This is confirmed by serde documentation on default values (https://serde.rs/attr-default.html) and is architecturally guaranteed by serde's trait-based design.

### Error Message Differences

| Aspect | `from_str()` | `from_value()` |
|--------|-------------|----------------|
| Error description | "missing field 'title'" | "missing field 'title'" |
| Line/column info | Included (e.g., "at line 47 column 5") | **Not included** |
| Structural path | Not included | Not included |

**Mitigation options:**
- Accept the minor degradation (BACKLOG.yaml is machine-managed)
- Use `serde_path_to_error` to add structural paths like `items[3].status` (follow-up)

### Common Pitfalls

1. **`from_value()` consumes the Value** — Takes `Value` by value, not by reference. Must clone if needed after conversion (not needed in our case).
2. **Loss of line/column in error messages** — Acceptable tradeoff for a machine-managed file.
3. **Numeric type coercion in YAML** — `Value` preserves numeric types as-is; `from_value()` handles the same coercion as `from_str()`. Not a new concern.

## Internal Research

### Existing Codebase State

The double-parse pattern appears in three locations:

| Location | First Parse (to Value) | Second Parse (to typed) |
|----------|----------------------|------------------------|
| `backlog.rs` lines 28 + 65 | `from_str::<Value>(&contents)` for version check | `from_str::<BacklogFile>(&contents)` |
| `migration.rs` lines 184 + 205 | `from_str::<Value>(&contents)` for version check in v1→v2 | `from_str::<V1BacklogFile>(&contents)` |
| `migration.rs` lines 451 + 467 | `from_str::<Value>(&contents)` for version check in v2→v3 | `from_str::<V2BacklogFile>(&contents)` |

### Relevant Files

- **`orchestrator/src/backlog.rs`** (550 lines) — Primary optimization target. `load()` at lines 23-69 with double-parse at lines 28 and 65.
- **`orchestrator/src/migration.rs`** (611 lines) — Out of scope per PRD, but contains same pattern at lines 184-206 and 451-468.
- **`orchestrator/src/types.rs`** (400+ lines) — Defines `BacklogFile` (lines 229-239) and `BacklogItem` (lines 186-227). Extensive `#[serde(default)]` usage. No `#[serde(deny_unknown_fields)]`.
- **`orchestrator/Cargo.toml`** — Uses `serde_yaml_ng = "0.10"` which supports `from_value()`.
- **`orchestrator/tests/backlog_test.rs`** — 15+ load/save round-trip tests.
- **`orchestrator/tests/migration_test.rs`** — Migration path tests.

### Serde Attributes on Key Types

**BacklogFile:**
- `#[serde(default)]` on `items: Vec<BacklogItem>` and `next_item_id: Option<u32>`
- No `deny_unknown_fields`

**BacklogItem:**
- `#[serde(default)]` on ~15 optional fields
- `#[serde(skip_serializing_if = "Option::is_none")]` / `#[serde(skip_serializing_if = "Vec::is_empty")]` on optional fields
- Custom `Deserialize` impl on `FollowUp` for union handling

### Constraints

- No `deny_unknown_fields` anywhere — forward-compatible; `from_value()` will handle this correctly
- Custom `Deserialize` impl on `FollowUp` — `from_value()` works with custom deserializers
- Error handling uses `.map_err()` with context — must be preserved for `from_value()` path
- `serde_yaml_ng` version 0.10 supports `from_value()` — no dependency changes needed

### Version Extraction Pattern (Reusable)

```rust
let schema_version = version_value
    .get("schema_version")
    .and_then(|v| v.as_u64())
    .unwrap_or(1) as u32;
```

This pattern appears identically in three locations. Could be extracted but is out of scope per PRD.

## PRD Concerns

### Confirmed Safe

1. **`#[serde(default)]` works identically with `from_value()`** — The PRD's top risk is resolved. The attribute is trait-level, not deserializer-level. All 15+ `#[serde(default)]` annotations on BacklogItem will behave identically.

2. **No new dependencies needed** — `from_value()` is already in `serde_yaml_ng` 0.10.

3. **All serde attributes are format-agnostic** — `skip_serializing_if`, `rename`, `default`, `alias` all work through the `Deserialize` trait.

### Minor Concern: Error Messages

PRD says "Error messages should remain helpful and include file path context, though exact wording may differ." Research confirms error _descriptions_ are preserved but line/column numbers are lost. This is acceptable since:
- BACKLOG.yaml is machine-managed (not hand-edited)
- File path context is added by `.map_err()` wrapper, which is unaffected
- The PRD already anticipated this difference

### PRD Pre-Implementation Verification Requirement

PRD requires: "Pre-implementation verification that `serde_yaml_ng::from_value()` produces identical results to `from_str()` for structs using `#[serde(default)]`." Research strongly suggests this will pass, but the verification test is still good practice and should be done early in implementation.

## Critical Areas

1. **`from_value()` consumes the Value** — The Value variable used for version extraction is moved into `from_value()`. Ensure the version extraction happens before the Value is consumed. This is straightforward but easy to miss.

2. **Error wrapping must be updated** — The current `.map_err()` on line 65 wraps `from_str` errors. The same `.map_err()` must wrap `from_value` errors. Error types should be compatible (`serde_yaml_ng::Error` in both cases).

## Open Questions

None remaining. All key questions are resolved by research:
- `#[serde(default)]`: Confirmed identical behavior
- Error messages: Confirmed minor degradation (no line/column), acceptable
- API availability: Confirmed in `serde_yaml_ng` 0.10

## Recommended Approach

**Parse-Once-Then-Convert** — the approach outlined in the PRD is correct and well-supported.

Implementation steps for `backlog::load()`:
1. Keep existing `from_str::<Value>()` call (line 28)
2. Keep existing `schema_version` extraction (lines 31-34)
3. Replace `from_str::<BacklogFile>(&contents)` on line 65 with `from_value::<BacklogFile>(version_check)`
4. Update `.map_err()` wrapper for the new call site
5. Write pre-implementation verification test comparing `from_value()` vs `from_str()` output

No alternative approaches are recommended. The PRD's chosen approach is the standard, well-supported pattern.

## References

| Description | URL |
|---|---|
| serde_yaml_ng `from_value` API docs | https://docs.rs/serde_yaml_ng/latest/serde_yaml_ng/fn.from_value.html |
| serde_yaml_ng GitHub repository | https://github.com/acatton/serde-yaml-ng |
| serde_yaml_ng crates.io | https://crates.io/crates/serde_yaml_ng |
| Serde `#[serde(default)]` documentation | https://serde.rs/attr-default.html |
| Serde enum representations | https://serde.rs/enum-representations.html |
| serde-rs/serde#1811 — span info in deserialization | https://github.com/serde-rs/serde/issues/1811 |
| serde_path_to_error docs | https://docs.rs/serde_path_to_error |
| YAML Wrangling with Rust (patterns overview) | https://parsiya.net/blog/2022-10-16-yaml-wrangling-with-rust/ |

## Assumptions

Decisions made without human input:

1. **Light mode appropriate** — PRD specified light mode; the research confirmed the approach is well-understood and low-risk.
2. **Line/column loss acceptable** — BACKLOG.yaml is machine-managed, so losing line/column info in rare error cases is an acceptable tradeoff.
3. **No `serde_path_to_error` needed now** — Could improve error quality but adds a dependency; leave as potential follow-up.
4. **Migration functions remain out of scope** — Consistent with PRD; they use the same pattern but have crash-safety concerns.
