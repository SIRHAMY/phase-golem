# Tech Research: Add Structured Description Format for Backlog Items

**ID:** WRK-028
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-028_add-structured-description-format-for-backlog-items_PRD.md
**Mode:** Medium

## Overview

This research investigates the patterns, tools, and codebase constraints relevant to converting `BacklogItem::description` from `Option<String>` to `Option<StructuredDescription>` with five typed fields. The key questions are: what's the best migration approach for YAML schema evolution, how to parse semi-structured freeform text into typed sections in Rust, how to implement flexible serde deserialization, and how to render structured descriptions into agent prompts effectively.

## Research Questions

- [x] What YAML schema migration pattern should we follow for v2-to-v3?
- [x] How should we parse freeform descriptions with section headers into structured types?
- [x] What serde pattern enables "string or struct" deserialization for StructuredDescription?
- [x] How should structured descriptions be rendered in production prompts?
- [x] What does the existing codebase already provide that we can reuse?
- [x] What integration points need updating and what's the blast radius?

---

## External Research

### Landscape Overview

The problem space spans four areas: schema migration for file-based data, freeform-to-structured text parsing, flexible Rust deserialization, and structured prompt rendering for LLMs. All four areas have well-established patterns with clear "right answers" for this use case.

### Common Patterns & Approaches

#### Pattern: Version Field + Eager Migration

**How it works:** Each data file contains a `schema_version` field. On load, the application checks the version and runs migration functions if older than expected. All data is migrated immediately (eager).

**When to use:** When you own the file exclusively and the dataset is small. This is the PRD's existing v1-to-v2 pattern.

**Tradeoffs:**
- Pro: Simple mental model — after migration, all data is in the new format
- Pro: Application code only needs to handle the current version
- Pro: Migration is testable and deterministic
- Con: Migration must handle all edge cases upfront
- Con: If migration has a bug, the file becomes unreadable (mitigated by git history)

**References:**
- [MongoDB Schema Versioning Pattern](https://www.mongodb.com/blog/post/building-with-patterns-the-schema-versioning-pattern) — Best conceptual overview
- [MongoDB Schema Versioning Docs](https://www.mongodb.com/docs/manual/data-modeling/design-patterns/data-versioning/schema-versioning/) — Implementation details

#### Pattern: Regex Split for Section Parsing

**How it works:** Use a case-insensitive, multi-line regex to match section headers at line starts, then extract content between headers. In Rust, the `regex` crate supports `(?im)` flags.

**When to use:** When section headers follow a predictable pattern with minor variations (case, punctuation). Ideal for 5 known headers.

**Tradeoffs:**
- Pro: Handles case variations naturally with `(?i)`
- Pro: Well-tested, performant regex engine in Rust
- Pro: Simple to understand and maintain
- Con: Greedy matching risks — must anchor to line starts with `(?m)` flag

**Suggested pattern:** `(?im)^(context|problem|solution|impact|sizing\s+rationale)\s*:\s*`

**References:**
- [Rust regex crate documentation](https://docs.rs/regex) — Case-insensitive matching, split, find_iter

#### Pattern: Official Serde `string_or_struct`

**How it works:** Implement `FromStr` for the type, then write a `string_or_struct` helper function with a custom `Visitor` that delegates `visit_str` to `FromStr` and `visit_map` to `Deserialize`. Apply with `#[serde(deserialize_with = "string_or_struct")]`.

**When to use:** When a field should accept either a string or a struct. The canonical example is Docker Compose's `build` key.

**Tradeoffs:**
- Pro: Official serde documentation with complete working example
- Pro: Clean separation — `FromStr` handles strings, `Deserialize` handles structs
- Con: ~30 lines of Visitor boilerplate

**References:**
- [Either string or struct — Official Serde Documentation](https://serde.rs/string-or-struct.html) — Complete working example
- [serde-rs/serde Issue #515](https://github.com/serde-rs/serde/issues/515) — Discussion and patterns

#### Pattern: Untagged Enum for Flexible Deserialization (Existing Codebase)

**How it works:** Define a `#[serde(untagged)]` enum with String and Struct variants inside a custom `Deserialize` impl. Match on the deserialized enum and map both variants to the target type.

**When to use:** When the codebase already uses this pattern and consistency matters.

**Tradeoffs:**
- Pro: Already proven in the codebase (`FollowUp` at `types.rs:281-321`)
- Pro: Simple implementation, familiar to contributors
- Con: Error messages from `#[serde(untagged)]` are poor ("data did not match any variant")
- Con: Tries each variant by buffering input

**References:**
- [Serde Enum Representations](https://serde.rs/enum-representations.html) — `#[serde(untagged)]` documentation
- [Serde Untagged Enum Error Analysis](https://www.gustavwengel.dk/serde-untagged-enum-errors-are-bad) — Why errors are poor

#### Pattern: Markdown Bold Headers for Prompt Rendering

**How it works:** Render each non-empty field as a labeled section: `**Context:** {text}\n**Problem:** {text}\n...`. Skip empty fields to minimize token waste.

**When to use:** When prompts target Claude/GPT-4 class models alongside other Markdown content.

**Tradeoffs:**
- Pro: Token-efficient (Markdown more compact than JSON/XML)
- Pro: Claude performs well with Markdown formatting
- Pro: Human-readable in logs
- Con: Less precise boundaries than XML tags

**References:**
- [Effective Context Engineering for AI Agents (Anthropic)](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents) — First-party guidance
- [XML Tags for Prompt Structure (Anthropic)](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/use-xml-tags) — Alternative approach
- [Prompt Format Impact on LLM Performance (arXiv)](https://arxiv.org/html/2411.10541v1) — Markdown vs JSON vs XML comparison

### Technologies & Tools

| Crate | Purpose | Status | Notes |
|-------|---------|--------|-------|
| `serde` + `serde_yaml_ng` | Serialization/deserialization | Already in use | All needed features available |
| `regex` | Section header parsing | **Not in Cargo.toml** | Would be a new dependency; alternative is pure string ops |
| `tempfile` | Atomic writes in migration | Already in use | Reuse existing pattern |
| `log` (via `log_info!`/`log_warn!`) | Migration logging | Already in use | Reuse existing macros |

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Case-sensitive header matching | "Sizing Rationale" vs "Sizing rationale" causes parse failure | Use case-insensitive matching (regex `(?i)` or `to_lowercase()`) |
| Greedy section header matching | Word "context" inside prose matches header pattern | Anchor to line start with `(?m)^` or require header to start a new line |
| Migration atomicity failure | Version bump in code but not data (or vice versa) causes load failure | Commit code + BACKLOG.yaml changes atomically |
| Empty string vs None confusion | `Some(StructuredDescription{all empty})` vs `None` have different semantics | Document clearly: `None` = no description, `Some` with empties = needs expansion |
| Prompt rendering empty fields | `**Solution:** ` with nothing wastes tokens and confuses agents | Only render non-empty fields |
| Untagged enum error messages | "data did not match any variant" is unhelpful for debugging | Accept this tradeoff (matches existing `FollowUp` pattern) or use `string_or_struct` |

### Key Learnings

- The existing v1-to-v2 migration pattern is well-aligned with industry best practices (MongoDB schema versioning). No need to change the approach.
- The `FollowUp` deserialization pattern in the codebase is the simplest path to flexible deserialization — consistency with existing code outweighs the minor error message downside.
- Prompt rendering should skip empty fields. Anthropic's context engineering guidance emphasizes "smallest possible set of high-signal tokens."
- The `regex` crate would be a new dependency. String-based parsing with `to_lowercase()` + `find()` is viable for 5 known headers and avoids the dependency.

---

## Internal Research

### Existing Codebase State

The orchestrator is a Rust project at `.claude/skills/changes/orchestrator/` using `serde` + `serde_yaml_ng` + `clap`. The data model centers on `BacklogItem` stored in `BACKLOG.yaml` at schema version 2. An actor-based coordinator manages state mutations through a channel-based command pattern using `tokio`.

**Relevant files/modules:**

| File | Relevance | Key Details |
|------|-----------|-------------|
| `src/types.rs` | Type definitions | `BacklogItem::description: Option<String>` (line 222), `ItemUpdate::SetDescription(String)` (line 158), `FollowUp` flexible deser (lines 281-321) |
| `src/migration.rs` | Migration pattern | v1-to-v2 pattern: `V1BacklogFile`/`V1BacklogItem` structs, `map_v1_item()`, `migrate_v1_to_v2()`. Uses atomic writes via tempfile. |
| `src/prompt.rs` | Prompt rendering | `build_preamble()` (line 157) does NOT render descriptions. `build_context_preamble()` (line 313, dead code) renders as `**Description:** {desc}`. |
| `src/backlog.rs` | Backlog management | `EXPECTED_SCHEMA_VERSION = 2` (line 13). `load()` dispatches migration. `add_item()` takes `Option<String>` description. `ingest_inbox_items()` copies inbox description directly. |
| `src/coordinator.rs` | State mutations | `SetDescription` handler at line 388 sets `item.description = Some(description)`. |
| `src/main.rs` | CLI | `--description` flag at line 73 on `Commands::Add`. |
| `BACKLOG.yaml` | Live data | 45 items, 9 with descriptions (7 convention-formatted, 2 freeform: WRK-050, WRK-051), 36 without. |

**Existing patterns in use:**
- Migration: Define old types → write mapping function → migrate eagerly on load → atomic write
- Flexible deser: `#[serde(untagged)]` enum inside custom `Deserialize` impl (`FollowUp`)
- Serde: `#[serde(rename_all = "snake_case")]`, `#[serde(default, skip_serializing_if = "...")]`
- Testing: `common::make_item()` helpers, YAML round-trip tests, temp dir isolation for migration tests

### Reusable Components

1. **`FollowUp` deserialization pattern** (`types.rs:281-321`) — Directly reusable for `StructuredDescription` flexible deser. The `#[serde(untagged)]` enum with String/Struct variants is proven.
2. **Migration infrastructure** (`migration.rs`) — The `migrate_v1_to_v2()` function pattern is directly clonable for `migrate_v2_to_v3()`. Atomic write, version check, idempotency guard all reusable.
3. **Test helpers** (`tests/common/mod.rs`) — `make_item()` sets `description: None` which still works since field remains `Option<_>`. `make_backlog()` needs schema_version bump.
4. **`Display` impl pattern** (`types.rs:72-110`) — `SizeLevel` and `DimensionLevel` have `Display` impls to follow for `StructuredDescription`.

### Constraints from Existing Code

1. **Migration chain in `load()`** — Currently `load()` at line 33 checks `schema_version < EXPECTED_SCHEMA_VERSION` and calls v1-to-v2. The v2-to-v3 migration must chain correctly: if a file is at v1, it should migrate to v2 first, then v2-to-v3. The current `load()` dispatches with a simple `if < expected` which only calls v1-to-v2. **This must be redesigned to handle multi-step migration (v1→v2→v3).**
2. **`ingest_inbox_items()` copies description** — At `backlog.rs:373`, it does `inbox_item.description.clone()`. PRD says ingested items should get `description: None`. This is a behavior change, not just a type change.
3. **`backlog_full.yaml` fixture** — Uses `schema_version: 2`. Must be updated to 3 or preserved as a v2 migration test input.
4. **`regex` is not currently a dependency** — The section header parsing could use `regex` (new dep) or pure string ops (no new dep). Need a decision.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Migration is v2→v3 (linear chain) | `load()` currently only dispatches v1→v2. A v1 file would need to go v1→v2→v3 in sequence. | `load()` needs a migration chain redesign — either sequential dispatch (v1→v2 then v2→v3) or a single v1→v3 shortcut. Sequential dispatch is simpler and more maintainable. |
| SCOPE.md says non-conforming descriptions get `description: None` | PRD says non-conforming descriptions are placed in `context` field with other fields as empty strings | **SCOPE.md and PRD disagree.** PRD's approach (preserve in `context`) is better — it's information-preserving. Design should follow the PRD. |
| `build_context_preamble` rendering is a Must Have | `build_context_preamble` is dead code (`#[allow(dead_code)]`) — not called anywhere in production | Updating dead code adds no value. Consider: (a) update it anyway for future use, (b) remove it entirely, or (c) defer until it's actually called. PRD lists it as Must Have, so update it, but note it has zero production impact. |
| `regex` crate implied for parsing | `regex` is not in `Cargo.toml` | Decision needed: add `regex` dependency or use pure string operations. For 5 known headers, pure string ops are sufficient and avoid a new dep. |
| Schema version bump + BACKLOG.yaml committed atomically | The migration function writes the file automatically on load | The migration transforms BACKLOG.yaml on first load. For the "atomic commit" requirement, the approach should be: (1) commit code changes with bumped `EXPECTED_SCHEMA_VERSION`, (2) run the orchestrator once to trigger migration, (3) commit the migrated BACKLOG.yaml. Or: pre-migrate BACKLOG.yaml manually and commit everything together. The PRD prefers the latter. |

---

## Critical Areas

### Section Header Parsing Robustness

**Why it's critical:** The migration parser must correctly handle all 9 existing descriptions (7 convention-formatted, 2 freeform). A parsing bug could silently discard description content or assign text to the wrong field.

**Why it's easy to miss:** The "happy path" (well-formatted descriptions) is easy. Edge cases include: varying case ("Sizing Rationale" vs "Sizing rationale"), section headers appearing inside prose content, descriptions with only some sections present, trailing whitespace, and empty sections.

**What to watch for:** Test against every existing description in BACKLOG.yaml. The parser should be tested with the exact text from the 7 convention-formatted items. For the 2 freeform items (WRK-050, WRK-051), verify the full text is placed in the `context` field.

### Migration Chain Dispatch

**Why it's critical:** The `load()` function currently only dispatches v1→v2. After this change, it must handle v1→v2→v3 correctly. A v1 file loaded after the version bump would try the v1→v2 migration but then still be at v2, which is less than the new expected version 3.

**Why it's easy to miss:** The PRD focuses on v2→v3 migration but doesn't explicitly address what happens to v1 files. The current `load()` has a simple `if < expected` guard that only calls one migration function.

**What to watch for:** The `load()` function should either: (a) chain migrations sequentially (call v1→v2, then v2→v3), or (b) bump the version check to dispatch the right migration. Option (a) is more maintainable.

### `ingest_inbox_items()` Behavior Change

**Why it's critical:** Currently copies inbox `description: Option<String>` directly to `BacklogItem::description`. The PRD requires ingested items to get `description: None`. This is a behavioral change that affects how triage works — the freeform inbox description will only be available to triage via the prompt, not stored on the BacklogItem.

**Why it's easy to miss:** It looks like a simple type mismatch fix, but it's actually a deliberate behavior change with downstream implications for how triage agents access inbox descriptions.

**What to watch for:** Ensure the triage prompt still passes the inbox item's freeform description to the triage agent even though the BacklogItem stores `description: None`. The freeform text must not be lost.

---

## Deep Dives

### Dependency Decision: `regex` vs Pure String Operations

**Question:** Should we add the `regex` crate for section header parsing, or use pure string operations?

**Summary:** For 5 known section headers with case-insensitive matching, pure string operations (`str::to_lowercase()` + line-by-line scanning) are sufficient. The regex approach (`(?im)^(context|problem|solution|impact|sizing\s+rationale)\s*:\s*`) is more elegant but introduces a new dependency.

**Approach — Pure string operations:**
1. Split description into lines
2. For each line, check if `line.trim().to_lowercase()` starts with any of the 5 header prefixes (e.g., `"context:"`, `"problem:"`, etc.)
3. When a header is found, start accumulating content for that section
4. When the next header is found or text ends, finalize the previous section
5. If no headers found, treat entire text as `context` field (freeform fallback)

**Implications:** Pure string ops avoid a new dependency, are simple to understand, and handle the 5 known headers well. The regex approach would be better if headers were dynamic or numerous. Given we have exactly 5 fixed headers, pure string ops are recommended.

### Migration Chain Architecture

**Question:** How should `load()` handle sequential migrations (v1→v2→v3)?

**Summary:** The simplest approach is a loop or sequential dispatch:

```
let mut version = detected_version;
if version == 1 {
    migrate_v1_to_v2(path, pipeline)?;
    version = 2;
}
if version == 2 {
    migrate_v2_to_v3(path)?;
    version = 3;
}
if version != EXPECTED_SCHEMA_VERSION {
    return Err(...)
}
```

Each migration reads the file, transforms it, and writes it back. The next migration reads the updated file. This is simple, correct, and follows the existing pattern where each migration function is self-contained.

**Implications:** The v2→v3 migration function should follow the same pattern as v1→v2: read file, parse as V2 types, map to V3 types, write. The `load()` function just needs sequential dispatch instead of a single branch.

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| Should `regex` be added as a dependency for section parsing? | Affects implementation complexity and dependency count | (a) Use pure string ops — simpler, no new dep; (b) Add regex — more elegant, handles edge cases better. **Recommendation: (a) pure string ops** |
| Should `build_context_preamble` (dead code) be updated or removed? | PRD says update it, but it's unused | (a) Update per PRD; (b) Remove dead code; (c) Update and remove `#[allow(dead_code)]`. **Recommendation: (a) follow PRD** |
| Should the migration pre-generate BACKLOG.yaml or rely on load-time migration? | Affects how the atomic commit works | (a) Pre-generate migrated YAML and commit with code; (b) Let load() migrate on first run. **Recommendation: (a) per PRD — commit atomically** |

### Recommended Approaches

#### Schema Migration (v2→v3)

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Eager migration following v1→v2 pattern | Consistent with codebase, proven, testable | Must handle all edge cases upfront | Always — this is the established pattern |
| Sequential dispatch in `load()` | Simple, maintainable, each step isolated | Slightly more disk I/O (read-write-read-write) | When migration chain grows beyond 2 steps |

**Initial recommendation:** Follow the v1→v2 pattern exactly. Define `V2BacklogFile`/`V2BacklogItem` structs, write `map_v2_item()` and `parse_description()` functions, implement `migrate_v2_to_v3()`. Update `load()` with sequential dispatch.

#### Section Header Parsing

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Pure string ops (line scan) | No new dep, readable, simple | Slightly more code | Known, fixed set of headers (our case) |
| Regex with `(?im)` flags | Elegant, handles variations | New dependency | Many/dynamic headers |
| Parser combinators (nom) | Most robust | Massive overkill | Complex grammar |

**Initial recommendation:** Pure string operations. Scan lines, match headers case-insensitively, accumulate section content. Fallback: if no headers found, put everything in `context`.

#### Flexible Deserialization (Nice to Have)

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `FollowUp`-style untagged enum | Consistent with codebase, simple | Poor error messages | Codebase already uses this pattern (our case) |
| Official `string_or_struct` pattern | Better documented, generic | More boilerplate (~30 lines) | Greenfield project |
| `#[serde(untagged)]` on field | Least code | Worst error messages | Prototype/quick hack |

**Initial recommendation:** Follow the `FollowUp` pattern from `types.rs:281-321`. Consistency with the existing codebase is more valuable than marginally better error messages.

#### Prompt Rendering

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Markdown bold headers, skip empties | Token-efficient, readable, consistent with `build_preamble` style | Less precise boundaries | Standard case (our case) |
| XML tags wrapping Markdown | Clear boundaries, Claude-optimized | More verbose | When description boundaries must be unambiguous |
| YAML/JSON block in prompt | Self-documenting schema | Worse reasoning performance | When agents need to output matching format |

**Initial recommendation:** Markdown bold headers with selective rendering (skip empty fields). Format: `**Context:** {text}\n**Problem:** {text}\n...`. Add as a `## Description` section in `build_preamble()` after the item info block.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Official Serde "string or struct"](https://serde.rs/string-or-struct.html) | Docs | Complete pattern for flexible deserialization |
| [MongoDB Schema Versioning](https://www.mongodb.com/blog/post/building-with-patterns-the-schema-versioning-pattern) | Article | Validates the version-field + eager-migration approach |
| [Effective Context Engineering (Anthropic)](https://www.anthropic.com/engineering/effective-context-engineering-for-ai-agents) | Article | Guidance on presenting structured data in prompts |
| [XML Tags for Prompts (Anthropic)](https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/use-xml-tags) | Docs | Alternative prompt structure approach |
| [Rust regex crate](https://docs.rs/regex) | Docs | Case-insensitive matching reference (if regex approach chosen) |
| `types.rs:281-321` (FollowUp deser) | Codebase | Proven flexible deserialization pattern to follow |
| `migration.rs:177-332` (v1→v2) | Codebase | Proven migration pattern to follow |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Parallel external + internal research (medium mode) | Comprehensive findings on migration patterns, parsing approaches, serde flexibility, prompt rendering, and full codebase analysis |
| 2026-02-13 | Deep dive: regex vs string ops | Recommended pure string ops for 5 known headers |
| 2026-02-13 | Deep dive: migration chain architecture | Recommended sequential dispatch in load() |
| 2026-02-13 | PRD concern analysis | Identified 5 PRD concerns including SCOPE/PRD contradiction on freeform fallback and migration chain gap |

## Assumptions

Decisions made without human input during autonomous tech research:

1. **Mode set to medium.** The PRD is well-scoped and the codebase is familiar from the internal research. Heavy mode not warranted.
2. **Pure string ops recommended over regex.** For exactly 5 known headers, adding a `regex` dependency is unnecessary. String operations are sufficient and simpler.
3. **Follow `FollowUp` deserialization pattern.** Codebase consistency outweighs the marginal error-message benefit of the official `string_or_struct` pattern.
4. **SCOPE.md contradiction resolved in favor of PRD.** SCOPE.md says non-conforming descriptions get `None`; PRD says they're preserved in `context` field. PRD takes precedence as the more recent, more detailed artifact.
5. **Sequential migration dispatch recommended.** `load()` should call v1→v2 then v2→v3 in sequence rather than requiring a single v1→v3 jump.
