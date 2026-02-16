# Tech Research: Add BacklogItem Default impl

**ID:** WRK-048
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-048_add-backlogitem-constructor-or-default-impl-to-reduce-manual-field-initialization_PRD.md
**Mode:** Light

## Overview

Researching the use of Rust's `Default` trait and struct update syntax (`..Default::default()`) to reduce boilerplate in BacklogItem construction sites. The change is well-understood Rust idiom — research focuses on confirming codebase compatibility and identifying any gotchas.

## Research Questions

- [x] Can `BacklogItem` use `#[derive(Default)]` or does it need a manual impl?
- [x] Can `ItemStatus` use `#[derive(Default)]` with `#[default]` attribute?
- [x] Are there any interactions between `#[serde(default)]` and `Default` trait?
- [x] What existing patterns in the codebase should we follow?

---

## External Research

### Landscape Overview

Rust's `Default` trait is a core standard library trait for providing zero-argument construction with sensible defaults. Two patterns exist: `#[derive(Default)]` for automatic implementation (when all fields implement `Default`), and manual `impl Default` for custom logic. Enum default support via `#[derive(Default)]` with `#[default]` attribute has been stable since Rust 1.62 (RFC 3107). Struct update syntax (`..Default::default()`) is the idiomatic way to override specific fields while defaulting the rest.

### Common Patterns & Approaches

#### Pattern: Derive Default + Struct Update Syntax

**How it works:** Apply `#[derive(Default)]` to a struct where all fields implement `Default`. Construction sites then use `SomeStruct { field: value, ..Default::default() }` to override specific fields.

**When to use:** Structs with many fields where most have natural defaults (Option → None, Vec → empty, bool → false, String → empty).

**Tradeoffs:**
- Pro: Zero-cost abstraction, no runtime overhead
- Pro: Adding new fields with Default-compatible types requires no changes at construction sites using `..Default::default()`
- Con: No compile-time enforcement that "required" fields are set — relies on construction sites to override them

**References:**
- [Rust Book — Struct Update Syntax](https://doc.rust-lang.org/book/ch05-01-defining-structs.html)
- [Rust Design Patterns — Default Trait](https://rust-unofficial.github.io/patterns/idioms/default.html)

#### Pattern: Enum Default with #[default] Attribute

**How it works:** Apply `#[derive(Default)]` to an enum, mark exactly one unit variant with `#[default]`. Compiler generates `Default` returning that variant.

**When to use:** Enums with a clear "initial" or "zero" state.

**Tradeoffs:**
- Pro: Clean, explicit, idiomatic
- Con: Only works on unit variants (no associated data)

**References:**
- [RFC 3107 — Derive Default for Enums](https://rust-lang.github.io/rfcs/3107-derive-default-enum.html)
- [Rust Std Docs — Default](https://doc.rust-lang.org/std/default/trait.Default.html)

### Standards & Best Practices

- RFC 3107 is the official standard for enum Default derive
- Rust API Guidelines recommend `Default` over multiple constructors when sensible
- Combining `#[derive(Default)]` with struct update syntax is the idiomatic pattern for configuration-like types with many optional fields

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Forgetting to set "required" fields when using `..Default::default()` | Empty string defaults for id/title/timestamps could propagate if a construction site forgets to set them | All existing sites already set these fields; unit test for Default values catches regressions |
| Multiple `#[default]` attributes on enum | Compile error | Only one variant gets `#[default]` |
| Assuming `#[serde(default)]` and `Default` trait are the same | They're independent mechanisms | This change only adds Rust `Default`, doesn't touch serde attributes |

---

## Internal Research

### Existing Codebase State

The orchestrator is a Rust project using `serde_yaml_ng` for YAML serialization. `BacklogItem` is a 22-field struct in `orchestrator/src/types.rs` (lines 186-227). `ItemStatus` is an enum with 6 unit variants: `New`, `Scoping`, `Ready`, `InProgress`, `Done`, `Blocked`.

**Relevant files/modules:**
- `orchestrator/src/types.rs` — BacklogItem struct definition (L186-227), ItemStatus enum (L5-14)
- `orchestrator/src/backlog.rs` — Production construction sites: `add_item`, `ingest_follow_ups`, `ingest_inbox_items`
- `orchestrator/src/migration.rs` — Migration construction sites: `map_v1_item`, `map_v2_item`
- `orchestrator/tests/common/mod.rs` — Test helpers: `make_item()`, `make_in_progress_item()`
- `orchestrator/tests/types_test.rs` — YAML round-trip tests
- `orchestrator/tests/scheduler_test.rs` — Scheduler tests with local `make_item()`

**Existing patterns in use:**
- `StructuredDescription` already derives `Default` — establishes the pattern in the codebase
- All BacklogItem construction sites manually specify all 22 fields
- Optional fields use `#[serde(default)]` and `#[serde(skip_serializing_if)]` for YAML (independent from Default trait)
- ItemStatus currently derives: `Serialize, Deserialize, Clone, Debug, PartialEq, Eq`
- BacklogItem currently derives: `Serialize, Deserialize, Clone, Debug`

### Reusable Components

- `StructuredDescription::default()` — already implemented, validates the pattern works in this codebase
- All field types already implement `Default`: `String`, `Option<T>`, `Vec<T>`, `bool`
- All enum types (`ItemStatus`, `SizeLevel`, `DimensionLevel`, `PhasePool`) are unit-variant-only — eligible for `#[derive(Default)]`

### Constraints from Existing Code

- `ItemStatus` is used in two BacklogItem fields: `status` (direct) and `blocked_from_status` (Option). The `Option` wrapper handles the latter case.
- `ItemStatus` has `is_valid_transition()` method — unaffected by adding `Default`
- Existing `#[serde(default)]` on 11 BacklogItem fields is orthogonal to Rust `Default` trait — no interaction

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| `#[derive(Default)]` can be used for BacklogItem | Confirmed: all 22 field types implement `Default`. `String::default()` returns `""`, which matches PRD's "empty strings" for id/title/created/updated | No issue — derive works as expected |
| 20 construction sites across the codebase | Internal research found ~11 in production/migration/test helpers, plus additional inline test constructions | Count may vary slightly but approach is the same |
| serde behavior is unchanged | Confirmed: `#[serde(default)]` and Rust `Default` are independent mechanisms | No risk to serialization |

No conflicts between PRD and research findings.

---

## Critical Areas

### Default Values Must Match Current Explicit Values

**Why it's critical:** If `Default` produces different values than what construction sites currently set explicitly, behavior would silently change.

**Why it's easy to miss:** With 22 fields, easy to overlook one where the explicit value doesn't match the derived default.

**What to watch for:** The unit test for `BacklogItem::default()` (specified in PRD as a must-have) is the primary guard. Every field's derived default must be verified: `String` → `""`, `Option<T>` → `None`, `Vec<T>` → `Vec::new()`, `bool` → `false`, `ItemStatus` → `New`.

---

## Synthesis

### Open Questions

| Question | Why It Matters | Resolution |
|----------|----------------|------------|
| `#[derive(Default)]` vs manual `impl Default` for BacklogItem? | Derive is simpler; manual allows custom values | **Resolved:** `#[derive(Default)]` works because all fields have the correct derived defaults. No custom logic needed. |
| `#[derive(Default)]` + `#[default]` vs manual impl for ItemStatus? | Both work; derive is more idiomatic | **Resolved:** Use `#[derive(Default)]` with `#[default]` on `New` variant. Stable since Rust 1.62, already used for StructuredDescription in codebase. |

### Recommended Approaches

#### Default Implementation Strategy

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `#[derive(Default)]` for both | Simple, zero boilerplate, follows existing `StructuredDescription` pattern | All defaults must match `Default` trait behavior | All field defaults align with Rust's Default — **this is our case** |
| Manual `impl Default` | Can set custom defaults (e.g., timestamps) | More code, must update manually when fields change | Some fields need non-standard defaults — **not needed here** |

**Initial recommendation:** Use `#[derive(Default)]` for both `ItemStatus` and `BacklogItem`. This is the simplest approach, follows existing codebase patterns (`StructuredDescription`), and all field types produce the correct defaults automatically.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Rust Book — Struct Update Syntax](https://doc.rust-lang.org/book/ch05-01-defining-structs.html) | Docs | Core syntax reference |
| [RFC 3107 — Enum Default](https://rust-lang.github.io/rfcs/3107-derive-default-enum.html) | RFC | Authority for `#[default]` on enum variants |
| [Rust Design Patterns — Default](https://rust-unofficial.github.io/patterns/idioms/default.html) | Guide | Pattern reference and best practices |

---

## Assumptions

Decisions made without human input (autonomous mode):

1. **Mode: Light** — This is a well-understood Rust pattern with low complexity; deep investigation not needed.
2. **`#[derive(Default)]` over manual impl** — All field types produce correct derived defaults; no custom logic needed.
3. **No additional Default impls needed** — PRD explicitly scopes out adding Default to other types (BacklogFile, PhaseResult, FollowUp). Research confirms this is fine for the current change.

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | External research: Rust Default trait patterns | Confirmed derive(Default) + struct update syntax is idiomatic; enum #[default] stable since 1.62 |
| 2026-02-13 | Internal research: BacklogItem codebase analysis | Confirmed all field types support derive(Default); StructuredDescription sets precedent; no serde conflicts |
| 2026-02-13 | Analysis against PRD | No conflicts found; all PRD assumptions validated by research |
