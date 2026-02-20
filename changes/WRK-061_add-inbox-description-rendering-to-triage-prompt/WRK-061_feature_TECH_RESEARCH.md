# Tech Research: Add Inbox Description Rendering to Triage Prompt

**ID:** WRK-061
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-061_feature_PRD.md
**Mode:** Light

## Overview

Researching the simplest correct approach for mapping `InboxItem.description: Option<String>` to `BacklogItem.description: Option<StructuredDescription>` during `ingest_inbox_items()`. The key questions are: (1) which mapping pattern to use, (2) whether to reuse existing `parse_description()` or do direct struct initialization, and (3) how to handle empty/whitespace-only inputs.

## Research Questions

- [x] What pattern should we use for mapping Option<String> to Option<StructuredDescription>?
- [x] Should we reuse `parse_description()` from migration.rs or do direct mapping?
- [x] How does the existing codebase handle similar optional field transformations?

---

## External Research

### Landscape Overview

Mapping an optional simple type to an optional structured type is a standard Rust data transformation. The idiomatic approach uses `Option::map()` combined with struct initialization or conversion traits.

### Common Patterns & Approaches

#### Pattern: Direct Mapping with Option::map()

**How it works:** Use `.filter()` to discard empty values, then `.map()` to construct the target struct inline.

**When to use:** One-off mappings where the transformation is straightforward and localized.

**Tradeoffs:**
- Pro: Clear, explicit, easy to understand at the call site
- Pro: No extra functions or trait implementations needed
- Con: Verbose if mapping occurs in multiple places

**References:**
- [Rust Option Documentation](https://doc.rust-lang.org/std/option/enum.Option.html)

#### Pattern: From/Into Trait Implementation

**How it works:** Implement `From<String> for StructuredDescription` to enable `.map(StructuredDescription::from)`.

**When to use:** When the same conversion is needed in multiple places.

**Tradeoffs:**
- Pro: Reusable, composable with Rust's type system
- Con: Over-engineered for a single call site
- Con: Semantics of "String → StructuredDescription" may be unclear

**References:**
- [Effective Rust - Type Conversions](https://www.lurklurk.org/effective-rust/casts.html)

### Standards & Best Practices

- Use `.map()` combinator for transforming Option values (idiomatic Rust)
- Use `..Default::default()` for struct initialization with partial fields
- Prefer explicit over implicit (aligns with project style guide)

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Not filtering empty strings before mapping | Creates a StructuredDescription with all-empty fields, wastes prompt space | Use `.filter()` before `.map()` |
| Using `parse_description()` for simple strings | Introduces unnecessary header-parsing complexity | Direct struct init for inbox descriptions |

### Key Learnings

- The simplest correct approach is `Option::filter` + `Option::map` with inline struct construction
- No external libraries or complex patterns needed for this change

---

## Internal Research

### Existing Codebase State

The ingestion pipeline is in `src/backlog.rs:306-343`. The `ingest_inbox_items()` function maps most `InboxItem` fields to `BacklogItem` but currently skips `description`, relying on `..Default::default()` (line 336) which sets it to `None`.

The prompt rendering pipeline is already complete — `build_preamble()` (prompt.rs:249) renders `StructuredDescription` when present, and `render_structured_description()` (prompt.rs:464) handles formatting individual fields, filtering out empty ones.

**Relevant files/modules:**
- `src/backlog.rs:306-343` — `ingest_inbox_items()` function; the modification site
- `src/types.rs:343-360` — `InboxItem` struct with `description: Option<String>` at line 349
- `src/types.rs:263-285` — `StructuredDescription` struct with fields: context, problem, solution, impact, sizing_rationale
- `src/prompt.rs:249-254` — `build_preamble()` renders description when present
- `src/prompt.rs:464-480` — `render_structured_description()` formats fields as `**{label}:** {value}`
- `src/migration.rs:541-614` — `parse_description()` converts text to StructuredDescription
- `tests/backlog_test.rs:1030-1061` — Existing test asserting `description == None` (must be updated)

**Existing patterns in use:**
- `ingest_inbox_items()` uses `..Default::default()` for unset fields (backlog.rs:336)
- Title trimming + empty check pattern at backlog.rs:316 (same pattern needed for description)
- `parse_description()` in migration.rs:541 converts text → StructuredDescription, placing plain text in `context` field when no headers found

### Reusable Components

- **`migration::parse_description()`** — Public function that converts text to StructuredDescription. When no section headers are found, it places `text.trim()` into the `context` field (line 588-595). Could technically be reused, but see PRD Concerns below.
- **`StructuredDescription` struct** — Already defined, derives Default. No modifications needed.
- **`render_structured_description()`** — Already handles rendering. No changes needed.

### Constraints from Existing Code

- `BacklogItem.description` is `Option<StructuredDescription>` — must wrap in Some() or leave as None
- `StructuredDescription` does not derive/implement `From<String>` — would need to be added if desired
- The existing `..Default::default()` pattern on line 336 means description defaults to None; we need to set it explicitly

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Direct struct init, not `parse_description()` | `parse_description()` handles the plain-text case correctly (line 588-595) and is already public | Either approach works; direct init is simpler and avoids coupling to migration module. PRD's choice is sound. |
| Empty/whitespace → None | `parse_description()` would return a StructuredDescription with empty context for whitespace-only input, not None | Filtering must happen before any mapping, regardless of approach chosen |

---

## Critical Areas

### Empty/Whitespace Handling

**Why it's critical:** If not handled, whitespace-only descriptions would create StructuredDescription objects that render as empty "## Description" sections in triage prompts — confusing and noisy.

**Why it's easy to miss:** The Option<String> could be `Some("")` or `Some("  ")`, which would pass a naive `.is_some()` check.

**What to watch for:** Must trim and check emptiness before constructing StructuredDescription. Follow the same pattern as title validation at backlog.rs:316.

---

## Deep Dives

(None needed — light mode research)

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| (None) | All questions resolved during research | N/A |

### Recommended Approaches

#### Description Mapping Strategy

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Direct struct init with `.filter().map()` | Explicit, simple, no new dependencies, matches PRD | Slightly verbose inline | Single call site (our case) |
| Reuse `parse_description()` | Handles structured headers too, already tested | Couples to migration module, over-parses simple strings, still needs filter step | Multiple call sites or complex text |
| `From<String>` trait impl | Reusable, idiomatic | Over-engineered for one site, still needs filter | Many conversion sites |

**Initial recommendation:** Direct struct init with `.filter().map()`. This is a single call site doing a simple mapping. The code will look like:

```rust
description: inbox_item.description
    .as_ref()
    .filter(|d| !d.trim().is_empty())
    .map(|d| StructuredDescription {
        context: d.trim().to_string(),
        ..Default::default()
    }),
```

This is explicit, handles all edge cases (None, empty, whitespace-only, normal text), and follows existing patterns in the function.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Rust Option docs](https://doc.rust-lang.org/std/option/enum.Option.html) | Docs | `.filter()` and `.map()` API reference |
| `src/backlog.rs:316` | Code | Existing trim + empty check pattern to follow |
| `src/migration.rs:588-595` | Code | Shows how parse_description handles plain text (for reference, not reuse) |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Light external + internal research | Confirmed direct mapping approach; identified parse_description as viable but unnecessary alternative; documented empty-string handling concern |
