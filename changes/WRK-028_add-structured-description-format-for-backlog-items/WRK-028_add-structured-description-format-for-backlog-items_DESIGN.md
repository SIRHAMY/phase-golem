# Design: Add Structured Description Format for Backlog Items

**ID:** WRK-028
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-028_add-structured-description-format-for-backlog-items_PRD.md
**Tech Research:** ./WRK-028_add-structured-description-format-for-backlog-items_TECH_RESEARCH.md
**Mode:** Medium

## Overview

Replace `BacklogItem::description` from `Option<String>` to `Option<StructuredDescription>` — a struct with five `String` fields (context, problem, solution, impact, sizing_rationale). The change follows the existing v1-to-v2 migration pattern to create a v2-to-v3 migration, uses pure string operations for section header parsing, follows the existing `FollowUp` deserialization pattern for flexible string-or-struct support, and wires structured descriptions into the production prompt path (`build_preamble`) so phase agents can actually see them.

---

## System Design

### High-Level Architecture

The change is a type-level evolution with four interacting concerns:

1. **Type definition** — New `StructuredDescription` struct in `types.rs`
2. **Schema migration** — v2-to-v3 migration in `migration.rs` with sequential dispatch in `load()`
3. **Prompt rendering** — Structured description rendering in `prompt.rs` for both `build_preamble` and `build_context_preamble`
4. **Integration updates** — Coordinator, backlog management, and CLI updated for the new type

```
BACKLOG.yaml (v2, Option<String>)
       │
       ▼ load()
  ┌─────────────┐
  │  migration   │ ── v2→v3 ── parse section headers ── write v3 YAML
  └─────────────┘
       │
       ▼
  BacklogItem { description: Option<StructuredDescription> }
       │
       ├──► prompt.rs::build_preamble()     ── render for phase agents
       ├──► prompt.rs::build_context_preamble() ── render for context
       ├──► coordinator.rs::SetDescription() ── update via StructuredDescription
       └──► backlog.rs::add_item()           ── new items start with None
```

### Component Breakdown

#### StructuredDescription (types.rs)

**Purpose:** Typed representation of a backlog item description with five required sections.

**Responsibilities:**
- Hold the five description fields: `context`, `problem`, `solution`, `impact`, `sizing_rationale`
- Serialize to/from YAML as a mapping with five keys
- Optionally deserialize from a plain string (flexible deser, Nice to Have)

**Interfaces:**
- Input: Constructed by migration parser, coordinator SetDescription handler, or YAML deserialization
- Output: Consumed by prompt rendering and YAML serialization

**Data structure:**
```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredDescription {
    pub context: String,
    pub problem: String,
    pub solution: String,
    pub impact: String,
    pub sizing_rationale: String,
}
```

All fields are `String` (not `Option<String>`), per PRD constraint. If the struct exists, all five fields are present. Empty strings indicate "not yet populated." This eliminates null-checking in consuming code and is consistent with the existing convention where all five sections are always included together.

**Semantics:** `None` = no description ever set. `Some(StructuredDescription { ... })` = description exists (even if some fields are empty strings, meaning "not yet populated"). This distinction is important: `None` items have never been described, while `Some` with empty fields indicates a description was created but is incomplete.

#### Section Header Parser (migration.rs)

**Purpose:** Parse freeform description strings with section headers into `StructuredDescription` during v2-to-v3 migration.

**Responsibilities:**
- Detect section headers (case-insensitive) in freeform text: `Context:`, `Problem:`, `Solution:`, `Impact:`, `Sizing rationale:`/`Sizing Rationale:`
- Extract content between headers, trimming leading/trailing whitespace of each section
- Preserve blank lines and indentation within sections
- Fall back to placing entire text in `context` field if no headers are found
- Log warnings for non-conforming descriptions using `log_warn!` macro with item ID and first 80 chars of text

**Interfaces:**
- Input: `&str` (freeform description text)
- Output: `StructuredDescription`

**Algorithm:**
```
fn parse_description(text: &str) -> StructuredDescription:
    1. Split text into lines
    2. Track current_section (initially None) and accumulated lines per section
    3. For each line:
       a. Trim the line and lowercase it
       b. Check if it starts with any known header prefix:
          - "context:", "problem:", "solution:", "impact:", "sizing rationale:"
       c. If header match:
          - Extract content after the colon on this line (include as first content line)
          - Set current_section to the matched field
       d. If no match:
          - Append line to current_section's accumulated content
    4. After all lines, finalize each section: join accumulated lines with '\n', trim
       leading/trailing whitespace
    5. If no headers were matched (current_section was never set):
       - Return StructuredDescription { context: text.trim().to_string(), problem: "".into(), ... }
    6. For any section that had no matching header, set to empty string
    7. Return the populated StructuredDescription
```

The function is infallible — it always returns a valid `StructuredDescription`. Parsing errors are handled via the freeform fallback.

**Header matching details:**
- Case-insensitive prefix check on trimmed lines. Line-by-line scanning naturally prevents false positives from header words appearing mid-sentence in prose (only line-start matches).
- Known prefixes: `"context:"`, `"problem:"`, `"solution:"`, `"impact:"`, `"sizing rationale:"`
- Content after the colon on the header line is included as the first line of that section's content.
- **Duplicate headers:** If a header appears more than once, the later occurrence wins (overwrites the earlier section). This is acceptable since existing BACKLOG.yaml descriptions don't contain duplicate headers.
- **Whitespace-only descriptions:** A description containing only whitespace is treated as freeform fallback — full text placed in `context` field (which will be an empty string after trimming).

**Dependencies:** No new dependencies. Uses `str::to_lowercase()` and `str::starts_with()`.

#### v2-to-v3 Migration (migration.rs)

**Purpose:** Transform v2 `BacklogFile` (description as `Option<String>`) to v3 (description as `Option<StructuredDescription>`).

**Responsibilities:**
- Define `V2BacklogFile` and `V2BacklogItem` structs mirroring the current schema
- Read v2 YAML, map each item's description through `parse_description()`
- Write v3 YAML with schema_version 3
- Atomic write via tempfile (following existing pattern): write to temp file in same directory, flush, rename over original. Rename is atomic on POSIX. Failure at any step leaves original file unchanged.
- Log migration progress using `log_info!` (start, completion, item count)
- Log warnings for non-conforming descriptions using `log_warn!` with item ID and text preview

**Interfaces:**
- Input: File path to BACKLOG.yaml at schema version 2
- Output: Migrated BACKLOG.yaml at schema version 3

**Error handling:**
- Deserialization failure (malformed v2 YAML): propagate error with context "Failed to deserialize BACKLOG.yaml as v2 schema"
- Write failure: propagate error, original file unchanged due to atomic write pattern
- The function validates that the input file is at schema_version 2 before proceeding; returns error if not

**Pattern:** Follows `migrate_v1_to_v2()` exactly:
1. Read file and deserialize as `V2BacklogFile`
2. Map each `V2BacklogItem` to a `BacklogItem` via `map_v2_item()`
3. Create `BacklogFile { schema_version: 3, items, next_item_id }`
4. Serialize and write atomically via tempfile

#### Sequential Migration Dispatch (backlog.rs::load())

**Purpose:** Chain migrations so any schema version (1, 2, or 3) reaches the current expected version.

**Current behavior:**
```rust
if file.schema_version < EXPECTED_SCHEMA_VERSION {
    migrate_v1_to_v2(path, pipeline)?;
    // re-read
}
```

**New behavior:**
```rust
// Read schema_version from file before full deserialization
let version = read_schema_version(path)?;

if version == 1 {
    migrate_v1_to_v2(path, pipeline)?;
    // version is now 2 on disk
}
if version <= 2 {
    migrate_v2_to_v3(path)?;
    // version is now 3 on disk
}
if version > EXPECTED_SCHEMA_VERSION {
    return Err("unsupported schema version {version} (expected {EXPECTED_SCHEMA_VERSION}) — please upgrade orchestrator")
}
// re-read and return
```

The `EXPECTED_SCHEMA_VERSION` constant bumps from 2 to 3.

**Error handling for migration chain:**
- If v1→v2 succeeds but v2→v3 fails: file is at v2 on disk. On next load, v1→v2 is skipped (already v2), v2→v3 retries. Each migration is independently idempotent, making the chain retry-safe.
- If v2→v3 deserialization fails: error propagated to caller with context. Original v2 file is unchanged (migration validates version before writing).
- Forward compatibility: If schema_version > EXPECTED_SCHEMA_VERSION, return a clear error asking the user to upgrade the orchestrator.

#### Prompt Rendering (prompt.rs)

**Purpose:** Render structured descriptions into agent prompts so phase agents can see item descriptions.

**Non-empty field predicate:** A field is considered non-empty when `!field.is_empty()`. Whitespace-only fields (e.g., `"  "`) are rendered as-is — the parser trims whitespace during migration, so this case shouldn't arise in practice.

**For `build_preamble` (production path):**

Add a `## Description` section after the existing item info block. Render only non-empty fields with Markdown bold headers. If all fields are empty, omit the `## Description` section entirely.

```
## Description

**Context:** {text}
**Problem:** {text}
**Solution:** {text}
**Impact:** {text}
**Sizing Rationale:** {text}
```

Fields are always rendered in this fixed order: Context, Problem, Solution, Impact, Sizing Rationale. Empty fields are skipped (entire line omitted).

**For `build_context_preamble` (currently dead code):**

Update the existing `**Description:** {desc}` rendering to iterate over structured fields, rendering each non-empty field with a labeled header. Same format and field order as `build_preamble`.

#### Integration Updates

**Coordinator (coordinator.rs):**
- `ItemUpdate::SetDescription(String)` → `ItemUpdate::SetDescription(StructuredDescription)`
- Handler at line 389 remains the same logic: `item.description = Some(description)`

**Backlog management (backlog.rs):**
- `add_item()` signature: `description: Option<String>` → remove the parameter entirely. All new items start with `description: None`. The CLI `--description` flag is removed.
- `ingest_inbox_items()`: Change from `description: inbox_item.description.clone()` to `description: None`. The inbox freeform description flows to triage via the prompt, not via the BacklogItem field. **Important:** Verify that `build_triage_prompt()` renders the inbox item's freeform description text so triage agents can still see it. The freeform text must not be lost.
- `ingest_follow_ups()`: Already sets `description: None` — no change needed.

**CLI (main.rs):**
- Remove `--description` flag from `Commands::Add`
- Remove `description` parameter from `handle_add()`

### Data Flow

1. **On load:** `load()` reads BACKLOG.yaml → detects schema_version → chains migrations (v1→v2→v3 as needed) → re-reads as current types → returns `BacklogFile`
2. **Migration (one-time):** For each item with `Some(description_string)`, `parse_description()` converts to `StructuredDescription`. Items without descriptions stay `None`.
3. **Prompt rendering:** `build_preamble()` checks `item.description`, renders non-empty fields as Markdown sections
4. **Item creation:** New items via CLI or inbox ingestion start with `description: None`
5. **Description updates:** Coordinator accepts `SetDescription(StructuredDescription)` — triage or other agents populate descriptions (triage prompt changes are out of scope)

### Key Flows

#### Flow: First Load After Migration (v2→v3)

> Existing BACKLOG.yaml at v2 is migrated to v3 on first load after code deployment.

1. **load() reads schema version** — Detects schema_version 2
2. **Sequential dispatch** — version <= 2, calls `migrate_v2_to_v3(path)`
3. **migrate_v2_to_v3 reads file** — Deserializes as `V2BacklogFile`. If deserialization fails, returns error with context; original file unchanged.
4. **Map each item** — For items with `Some(description)`:
   - Call `parse_description(text)` which scans for section headers
   - Convention-formatted descriptions (7 items): parsed into five fields
   - Freeform descriptions (2 items: WRK-050, WRK-051): full text placed in `context`, other fields empty; `log_warn!` with item ID and text preview
   - Items with `None`: remain `None`
5. **Write v3 file** — Atomic write via tempfile with schema_version 3. If write fails, original v2 file is unchanged.
6. **load() re-reads from disk** — Deserializes as current `BacklogFile` with `Option<StructuredDescription>` fields. If re-read fails after successful migration, error propagated with context.
7. **Returns** — Fully typed backlog ready for use

**Edge cases:**
- Already at v3 → migration skipped (idempotent via version check)
- At v1 → v1→v2 migration runs first, then v2→v3
- Schema version > expected → error "please upgrade orchestrator"
- Description with some headers missing → present headers populate their fields, missing headers get empty strings
- Description with section headers inside prose → only matches headers at start of line (after trimming), preventing false positives
- Duplicate section headers in same description → later occurrence overwrites earlier
- Whitespace-only description → freeform fallback, context field is empty after trimming
- Migration fails mid-way → atomic write ensures original file is preserved; retry-safe on next load

#### Flow: Phase Agent Receives Description in Prompt

> A phase agent (PRD, design, spec, build, review) receives the item's structured description.

1. **Scheduler selects item** — Item has `description: Some(StructuredDescription { ... })`
2. **build_preamble() called** — Constructs prompt preamble with item metadata
3. **Description rendering** — For each non-empty field (in order: Context, Problem, Solution, Impact, Sizing Rationale), appends `**{Label}:** {content}` line under `## Description` header
4. **Agent receives prompt** — Sees structured description with clear section labels, uses it to inform phase work

**Edge cases:**
- All fields empty → `## Description` section omitted entirely (no empty header)
- Some fields empty → only non-empty fields rendered (empty ones skipped)
- Description is `None` → no description section in prompt

#### Flow: Inbox Item Ingestion

> An inbox item with a freeform description is ingested into the backlog.

1. **Inbox loaded** — `InboxItem { description: Some("freeform text") }`
2. **ingest_inbox_items()** — Creates `BacklogItem` with `description: None`
3. **Triage prompt** — `build_triage_prompt()` renders the inbox item including its freeform description text so triage agent can see it
4. **Triage processes** — Agent reads freeform text, eventually calls `SetDescription(StructuredDescription { ... })` to populate (out of scope for this change — requires triage prompt updates)

**Edge cases:**
- Inbox item with no description → `description: None` on BacklogItem (same behavior)

---

## Technical Decisions

### Key Decisions

#### Decision: Pure String Operations for Section Parsing

**Context:** Need to parse 5 known section headers case-insensitively from freeform text. `regex` crate is available but not currently a dependency.

**Decision:** Use `str::to_lowercase()` + `str::starts_with()` line-by-line scanning instead of regex.

**Rationale:** For exactly 5 fixed, known headers, pure string ops are sufficient and avoid adding a new dependency. The regex approach would be better if headers were dynamic or numerous. Tech research recommends this approach. The O(n) line-scanning approach meets the PRD's performance requirement of completing migration within 1 second for 100+ items with typical description lengths (100-500 words).

**Consequences:** Slightly more code than a single regex pattern, but zero new dependencies and easier to understand for contributors.

#### Decision: Sequential Migration Dispatch in load()

**Context:** `load()` currently only dispatches v1→v2. Adding v2→v3 requires handling the migration chain.

**Decision:** Use sequential `if version == N` / `if version <= N` checks to chain migrations, where each migration reads/writes the file independently.

**Rationale:** Simple, maintainable, and each migration is self-contained. Follows the existing pattern. Tech research recommends this. Each migration is independently retry-safe — if one step fails, the file is left at a valid intermediate version and the next load retries from that point.

**Consequences:** Slightly more disk I/O for files starting at v1 (read-write-read-write), but this is a one-time migration and the performance impact is negligible for small YAML files.

#### Decision: Remove CLI --description Flag

**Context:** The `--description` flag on `orchestrate add` accepts a freeform string, which can't produce a `StructuredDescription`.

**Decision:** Remove the flag entirely. Items added via CLI start with `description: None`.

**Rationale:** Aligns with the inbox model where human input is freeform and agents expand it during triage. There's no ergonomic way to specify five separate fields via CLI flags, and adding a mini-parser for freeform CLI input is unnecessary complexity.

**Consequences:** Users who currently use `--description` on `orchestrate add` will need to omit it. This is acceptable since `SetDescription` is never called in production today and descriptions come from manual YAML editing.

#### Decision: Inbox Ingestion Sets description: None

**Context:** `ingest_inbox_items()` currently copies `inbox_item.description.clone()` (an `Option<String>`) to the BacklogItem's description field.

**Decision:** Change to always set `description: None` on ingested items.

**Rationale:** The inbox description is freeform text. It can't be directly assigned to `Option<StructuredDescription>`. The freeform text still flows to the triage agent via the prompt rendering of the inbox item — it's not lost. Triage is responsible for expanding freeform text into a structured description via `SetDescription`.

**Consequences:** Ingested items will not carry their inbox description on the BacklogItem until triage processes them and calls `SetDescription`. The freeform description is still available in the triage prompt. **Prerequisite verification required during spec/build:** confirm that `build_triage_prompt()` renders `InboxItem::description` text to triage agents.

#### Decision: Follow FollowUp Deserialization Pattern for Flexible Deser (Nice to Have)

**Context:** YAML could contain either a structured map or a plain string for the description field (e.g., from agent output variations or manual YAML editing).

**Decision:** Use the `#[serde(untagged)]` enum pattern from `FollowUp`'s custom `Deserialize` impl. A plain string maps to `context` field with other fields as empty strings.

**Rationale:** Codebase consistency. The `FollowUp` pattern at `types.rs:281-321` is proven and familiar. The error message downside is acceptable given the existing precedent.

**Consequences:** Poor error messages from `#[serde(untagged)]` ("data did not match any variant"), same as existing `FollowUp` behavior.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| No new dependency | Slightly more code for section parsing (~30 lines) | Zero dependency growth, simpler build | 5 fixed headers don't warrant a regex dependency |
| Freeform descriptions in `context` field | 2 items (WRK-050, WRK-051) have unstructured descriptions in `context` | Information preservation — no data loss during migration | Freeform items signal "needs expansion" during future triage |
| Inbox description dropped on ingestion | Ingested items lose direct description until triage processes them | Clean type boundaries — `Option<StructuredDescription>` everywhere | Freeform inbox text still flows via triage prompt |
| CLI --description removed | One less CLI flag | Clean API — no mismatch between freeform input and structured type | Flag was rarely used; items get descriptions via triage |
| Empty strings for unpopulated fields | `Some(StructuredDescription { all empties })` is valid | Eliminates null-checking per field in consuming code | Consistent with PRD constraint; `None` vs `Some` distinction is sufficient |
| Triage can't produce SetDescription yet | Type infrastructure ships but primary producer can't use it until triage prompts updated | Clean incremental delivery — type system is ready when triage support lands | SetDescription is never called in production today; triage prompt updates are a follow-up |

---

## Alternatives Considered

### Alternative: Regex-Based Section Parsing

**Summary:** Use the `regex` crate with pattern `(?im)^(context|problem|solution|impact|sizing\s+rationale)\s*:\s*` to split descriptions into sections.

**How it would work:**
- Add `regex` to `Cargo.toml`
- Use `Regex::split()` or `Regex::find_iter()` to locate section boundaries
- Extract content between matches

**Pros:**
- More concise parsing code (~10 lines vs ~30)
- Natural case-insensitive handling via `(?i)` flag
- Multi-line anchoring via `(?m)` flag

**Cons:**
- New dependency in `Cargo.toml`
- Regex compilation cost (minor, one-time)
- Slightly harder to debug than line-by-line scanning

**Why not chosen:** For exactly 5 fixed headers, the added dependency isn't justified. Pure string ops are sufficient and the codebase has no existing regex dependency to piggyback on.

### Alternative: Keep description as Option<String> and Add Parsing at Consumption

**Summary:** Leave the storage type as `Option<String>` and parse section headers at the point of consumption (prompt rendering, phase extraction).

**How it would work:**
- No migration needed
- `parse_description()` called in `build_preamble()` on every prompt render
- Description text stays freeform in YAML

**Pros:**
- No schema migration
- No type system changes
- Backwards compatible

**Cons:**
- Parsing on every render (performance overhead, minor)
- No type-level guarantee of structure
- Consuming code must handle both structured and unstructured formats
- Convention erosion continues — nothing enforces the format

**Why not chosen:** Doesn't solve the core problem. The PRD explicitly requires type-level enforcement of the five-section structure. Parse-at-consumption means every consumer must handle the unstructured case, which is exactly the problem we're trying to eliminate.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Section parser misassigns content to wrong field | Corrupted descriptions post-migration | Low | Test against all 9 existing descriptions in BACKLOG.yaml with exact text assertions. Line-start matching prevents false positives from headers inside prose. |
| Migration writes invalid YAML | BACKLOG.yaml becomes unreadable | Low | Atomic write via tempfile (existing pattern) — failure leaves original file unchanged. Git history as additional backup. |
| Triage agents can't produce SetDescription after type change | SetDescription variant exists but is unusable until triage prompts are updated | Low | Acceptable — SetDescription is never called in production today. Triage prompt updates are explicitly out of scope and tracked as a follow-up. |
| Test update volume causes merge conflicts | Multiple test files modified | Low | Most tests use `description: None` which requires no change. Only ~3-5 tests construct `Some(description)` values. |
| `backlog_full.yaml` test fixture requires update | Fixture at schema_version 2 would fail to load | Medium | Create a dedicated v2 fixture for migration tests and update `backlog_full.yaml` to v3 for production tests. |
| Inbox description lost if triage prompt doesn't render it | Freeform inbox text invisible to triage agents | Medium | Verify during spec/build that `build_triage_prompt()` renders `InboxItem::description`. If not, add rendering before shipping the ingestion behavior change. |
| v1→v2 succeeds but v2→v3 fails | File left at v2, migration incomplete | Low | Each migration is independently idempotent and retry-safe. On next load, v1→v2 is skipped (already v2), v2→v3 retries. |

---

## Integration Points

### Existing Code Touchpoints

- `orchestrator/src/types.rs` — New `StructuredDescription` struct; change `BacklogItem::description` type; change `ItemUpdate::SetDescription` payload type; optional flexible deser impl
- `orchestrator/src/migration.rs` — New `V2BacklogFile`/`V2BacklogItem` structs; new `parse_description()` function; new `migrate_v2_to_v3()` function with version validation and logging
- `orchestrator/src/backlog.rs` — Bump `EXPECTED_SCHEMA_VERSION` to 3; update `load()` dispatch to sequential chain with forward-compatibility error; update `add_item()` (remove description param); update `ingest_inbox_items()` (set `description: None`)
- `orchestrator/src/prompt.rs` — Update `build_preamble()` to render structured descriptions; update `build_context_preamble()` similarly
- `orchestrator/src/coordinator.rs` — Update `SetDescription` handler (type change, logic unchanged)
- `orchestrator/src/main.rs` — Remove `--description` CLI flag from `Commands::Add`; remove from `handle_add()`
- `BACKLOG.yaml` — Pre-migrated to v3 format and committed atomically with code changes
- `orchestrator/tests/common/mod.rs` — Update `make_backlog()` schema_version to 3
- `orchestrator/tests/` — Update tests constructing `Some(description)` values; add migration tests

### Test Requirements

- **Migration tests:** Must validate `parse_description()` against all 9 current BACKLOG.yaml descriptions by exact text match: 7 convention-formatted items parsed into five populated fields (WRK-022, WRK-023, WRK-024, WRK-025, WRK-028, WRK-032, WRK-033), 2 freeform items (WRK-050, WRK-051) placed in `context` field with other fields empty.
- **Migration idempotency test:** Load a v3 file — verify migration is not re-triggered.
- **Migration chain test:** Load a v1 file — verify v1→v2→v3 chain completes correctly.
- **Prompt rendering tests:** Verify non-empty fields rendered, empty fields skipped, all-empty omits section.
- **Round-trip test:** Serialize `StructuredDescription` to YAML and back.

### Atomic Commit Process

To satisfy the PRD requirement that schema version bump and BACKLOG.yaml migration are committed atomically:

1. Implement all code changes including `EXPECTED_SCHEMA_VERSION = 3`
2. Run the orchestrator once (e.g., `orchestrate status`) to trigger migration on load
3. Review the migrated BACKLOG.yaml diff to verify correctness
4. Commit code changes + migrated BACKLOG.yaml together in a single commit
5. Verify tests pass with the migrated data

### External Dependencies

None. No new crate dependencies. The change is entirely internal to the orchestrator.

---

## Open Questions

- [ ] **Should `orchestrate status` display structured descriptions?** Currently shows nothing. Not blocking for this change but worth noting for follow-up.
- [ ] **Should there be a minimum quality threshold?** A `StructuredDescription` with all empty strings is valid per the PRD constraint. Worth revisiting after triage agents start producing descriptions.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain (or they're flagged for spec phase)

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Initial design draft | Full design covering type changes, migration, prompt rendering, and integration updates |
| 2026-02-13 | Self-critique (7 agents) | 11 auto-fixes applied: error handling details, edge cases, parser specifics, migration chain retry safety, test requirements, atomic commit process, inbox description verification, forward compatibility, performance note |
