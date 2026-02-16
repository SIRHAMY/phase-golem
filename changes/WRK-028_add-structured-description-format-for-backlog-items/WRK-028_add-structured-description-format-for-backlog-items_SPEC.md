# SPEC: Add Structured Description Format for Backlog Items

**ID:** WRK-028
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-028_add-structured-description-format-for-backlog-items_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

Backlog items use `description: Option<String>` — an unstructured free-text field. A five-section convention (Context, Problem, Solution, Impact, Sizing Rationale) is followed by 5 of the 7 items with descriptions, but nothing enforces this. The production prompt path (`build_preamble`) doesn't render descriptions at all, making them invisible to phase agents. This change replaces the freeform string with a typed `StructuredDescription` struct, migrates existing data via v2→v3 schema migration, and wires descriptions into the production prompt path.

## Approach

The change is a type-level evolution with four interacting concerns:

1. **Type definition** — New `StructuredDescription` struct in `types.rs` with five `String` fields
2. **Schema migration** — v2→v3 migration in `migration.rs` using pure string ops for section header parsing, with sequential dispatch in `load()`
3. **Prompt rendering** — Structured description rendering in `prompt.rs` for both `build_preamble` and `build_context_preamble`
4. **Integration updates** — Coordinator, backlog management, and CLI updated for the new type

The migration parses existing freeform descriptions by scanning for section headers (`Context:`, `Problem:`, etc.) at line starts, case-insensitively. Items with non-conforming descriptions (no recognizable headers) get their full text placed in the `context` field with other fields as empty strings. The parser is infallible — it always produces a valid `StructuredDescription`.

**Patterns to follow:**

- `.claude/skills/changes/orchestrator/src/types.rs:267-321` — `FollowUp` flexible deserialization pattern (for Nice to Have flexible deser)
- `.claude/skills/changes/orchestrator/src/migration.rs:171-332` — `migrate_v1_to_v2()` function (atomic write, version check, logging, V1 struct definitions)
- `.claude/skills/changes/orchestrator/src/prompt.rs:157-208` — `build_preamble()` optional section rendering pattern
- `.claude/skills/changes/orchestrator/tests/migration_test.rs` — Migration test structure (fixture copy, assertions, idempotency)
- `.claude/skills/changes/orchestrator/tests/common/mod.rs` — Test helper patterns

**Implementation boundaries:**

- Do not modify: `InboxItem` type (stays `Option<String>`)
- Do not modify: Triage prompt schema or output format (out of scope — follow-up item)
- Do not modify: `write_archive_worklog_entry` (no description rendering in worklog)
- Do not refactor: `build_context_preamble` beyond the description rendering change (keep `#[allow(dead_code)]`)

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Type & Parser Foundation | Med | Define `StructuredDescription`, implement `parse_description()` parser, write parser tests |
| 2 | Migration, Type Propagation & Integration | High | Change `BacklogItem::description` type, v2→v3 migration, prompt rendering, update all consumers and tests |
| 3 | Data Migration & Final Verification | Low | Run migration on BACKLOG.yaml, verify all descriptions migrated correctly |

**Ordering rationale:** Phase 1 creates the foundation type and parser in isolation — `StructuredDescription` exists but `BacklogItem` still uses `Option<String>`, so the codebase compiles cleanly. Phase 2 is a single atomic change: `BacklogItem::description` type change forces all consumers (migration, coordinator, backlog, CLI, prompt, tests) to update together since Rust's type system won't allow partial compilation. Phase 3 runs the migration on real data after all code is in place.

---

## Phases

Each phase should leave the codebase in a functional, stable state. Complete and verify each phase before moving to the next.

---

### Phase 1: Type & Parser Foundation

> Define StructuredDescription type and implement the section header parser with comprehensive tests

**Phase Status:** complete

**Complexity:** Med

**Goal:** Create the `StructuredDescription` struct and a thoroughly-tested `parse_description()` function that can correctly handle all 7 existing BACKLOG.yaml descriptions.

**Files:**

- `.claude/skills/changes/orchestrator/src/types.rs` — modify — Add `StructuredDescription` struct with serde derives
- `.claude/skills/changes/orchestrator/src/migration.rs` — modify — Add `parse_description()` function (standalone, not connected to migration yet)
- `.claude/skills/changes/orchestrator/tests/migration_test.rs` — modify — Add parser tests

**Patterns:**

- Follow `types.rs:267-276` (`FollowUp` struct) for struct definition with serde derives and `skip_serializing_if`
- The parser is a pure function — no side effects, no dependencies on other migration code

**Tasks:**

- [x] Define `StructuredDescription` struct in `types.rs` with five `String` fields: `context`, `problem`, `solution`, `impact`, `sizing_rationale`. Add `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]`. All fields are `String` (not `Option<String>`).
- [x] Implement `pub fn parse_description(text: &str) -> StructuredDescription` in `migration.rs`. Algorithm: split text into lines, scan each line for known headers (case-insensitive, line-start only after trim), accumulate content per section, trim final content. If no headers found, place entire text in `context` with other fields as empty strings. Known headers: `context:`, `problem:`, `solution:`, `impact:`, `sizing rationale:`.
- [x] Write parser tests in `migration_test.rs`:
  - Test convention-formatted descriptions: extract each field correctly with exact content match. Use inline strings that follow the `Context:\n...\nProblem:\n...` pattern with multi-line content, colons in content, and "Sizing rationale:" (lowercase 'r').
  - Test freeform descriptions: full text in `context`, other fields are empty strings. Use inline strings matching the pattern of WRK-050 and WRK-051 style descriptions (no section headers).
  - Test edge cases: empty string input (all fields empty), whitespace-only input (all fields empty), partial headers (some sections missing → missing sections get empty strings), duplicate headers (later occurrence wins), header with content on same line (`Context: some text here`)
- [x] Write `StructuredDescription` round-trip serialization test: create struct, serialize to YAML, deserialize, assert equality

**Verification:**

- [x] `cargo test` passes — all new parser tests pass
- [x] `cargo build` succeeds (StructuredDescription exists but is not yet used by BacklogItem)
- [x] Parser correctly extracts all 5 fields from convention-formatted descriptions
- [x] Parser places full text in `context` for freeform descriptions
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-028][P1] Feature: Add StructuredDescription type and section header parser`

**Notes:**

The `StructuredDescription` struct exists but is not yet used by `BacklogItem`. This keeps the codebase compiling at every step. The parser is tested in isolation before being wired into the migration.

**Followups:**

- Code review findings addressed: added `Default` derive and `#[serde(default)]` to StructuredDescription for resilient deserialization, added debug assertion for ASCII header safety, added doc comments clarifying duplicate header behavior, added test for pre-header text landing in context field.

---

### Phase 2: Migration, Type Propagation & Integration

> Change BacklogItem::description type, implement v2→v3 migration, update all consumers and tests

**Phase Status:** complete

**Complexity:** High

**Goal:** Perform the atomic type change from `Option<String>` to `Option<StructuredDescription>` on `BacklogItem::description`, implement v2→v3 migration with sequential dispatch, add prompt rendering, update all consumers (coordinator, backlog, CLI), and update all test files and fixtures. After this phase, the orchestrator handles v3 schema natively.

**Files:**

- `.claude/skills/changes/orchestrator/src/types.rs` — modify — Change `BacklogItem::description` type, change `ItemUpdate::SetDescription` payload type
- `.claude/skills/changes/orchestrator/src/migration.rs` — modify — Add V2 schema structs, `map_v2_item()`, `migrate_v2_to_v3()`
- `.claude/skills/changes/orchestrator/src/backlog.rs` — modify — Bump `EXPECTED_SCHEMA_VERSION`, update `load()` dispatch, update `add_item()`, update `ingest_inbox_items()`
- `.claude/skills/changes/orchestrator/src/prompt.rs` — modify — Add structured description rendering to `build_preamble()` and `build_context_preamble()`
- `.claude/skills/changes/orchestrator/src/coordinator.rs` — modify — SetDescription handler type update (line 388)
- `.claude/skills/changes/orchestrator/src/main.rs` — modify — Remove `--description` CLI flag, update `handle_init()` schema version
- `.claude/skills/changes/orchestrator/tests/fixtures/backlog_v2_full.yaml` — create — Copy of current `backlog_full.yaml` for v2→v3 migration tests
- `.claude/skills/changes/orchestrator/tests/fixtures/backlog_full.yaml` — modify — Bump to schema_version 3
- `.claude/skills/changes/orchestrator/tests/common/mod.rs` — modify — Bump `make_backlog()` to schema_version 3
- `.claude/skills/changes/orchestrator/tests/migration_test.rs` — modify — Add v2→v3 migration tests
- `.claude/skills/changes/orchestrator/tests/prompt_test.rs` — modify — Add structured description rendering tests, update existing description tests
- `.claude/skills/changes/orchestrator/tests/backlog_test.rs` — modify — Update inbox ingestion tests, update `add_item` tests
- `.claude/skills/changes/orchestrator/tests/coordinator_test.rs` — modify — Update SetDescription tests
- `.claude/skills/changes/orchestrator/tests/types_test.rs` — modify — Update description serialization tests if any exist

**Patterns:**

- Follow `migration.rs:14-95` for V2 struct definitions (mirror current types but with `description: Option<String>`)
- Follow `migration.rs:171-332` for `migrate_v2_to_v3()` (atomic write via tempfile, version validation, logging)
- Follow `backlog.rs:21-59` for `load()` dispatch pattern
- Follow `prompt.rs:184-187` for optional section rendering pattern

**Tasks:**

- [x] **Copy v2 fixture:** Copy `tests/fixtures/backlog_full.yaml` to `tests/fixtures/backlog_v2_full.yaml` (preserve v2 fixture for migration tests)
- [x] **Type changes in `types.rs`:**
  - Change `BacklogItem::description` (line 222) from `Option<String>` to `Option<StructuredDescription>`
  - Change `ItemUpdate::SetDescription(String)` (line 158) to `SetDescription(StructuredDescription)`
- [x] **V2 schema structs in `migration.rs`:**
  - Define `V2BacklogFile` and `V2BacklogItem` structs mirroring current schema with `description: Option<String>`. Include all fields with proper serde attributes.
- [x] **Migration function in `migration.rs`:**
  - Implement `map_v2_item(v2: &V2BacklogItem) -> BacklogItem` — maps all fields directly, transforms `description` via `v2.description.as_deref().map(parse_description)`. Log a warning via `log_warn!` for non-conforming descriptions (no recognized headers) with item ID and first 80 chars.
  - Implement `pub fn migrate_v2_to_v3(path: &Path) -> Result<BacklogFile, String>` following the `migrate_v1_to_v2` pattern: read file, validate `schema_version == 2`, deserialize as `V2BacklogFile`, map items via `map_v2_item`, create `BacklogFile` with `schema_version: 3`, atomic write via tempfile, log migration start/completion with item counts.
- [x] **Backlog updates in `backlog.rs`:**
  - Bump `EXPECTED_SCHEMA_VERSION` from 2 to 3
  - Update `load()` to sequential migration dispatch: the existing `schema_version < EXPECTED_SCHEMA_VERSION` check already handles the "needs migration" case. Update the migration block to chain: if `schema_version == 1`, call `migrate_v1_to_v2()` first (writes v2 to disk), then fall through. If `schema_version <= 2` (covers both fresh v2 files and just-migrated-from-v1 files), call `migrate_v2_to_v3()`. Each migration reads and writes the file, so they chain naturally. The existing `schema_version != EXPECTED` check after migration handles forward-compat errors.
  - Remove `description` parameter from `add_item()` — all new items start with `description: None`
  - Change `ingest_inbox_items()` line 374: set `description: None` instead of `inbox_item.description.clone()`
- [x] **Prompt rendering in `prompt.rs`:**
  - Add helper function `render_structured_description(desc: &StructuredDescription) -> String` — renders non-empty fields in order (Context, Problem, Solution, Impact, Sizing Rationale) as `**{Label}:** {content}`, one per line, skipping empty-string fields. Returns empty string if all fields are empty strings.
  - In `build_preamble()`: after the `extra_item_field` block (after line 182), add description rendering: `if let Some(ref desc) = item.description { let rendered = render_structured_description(desc); if !rendered.is_empty() { push "## Description\n{rendered}" } }`
  - In `build_context_preamble()`: replace lines 333-338 with structured rendering using the same `render_structured_description` helper
- [x] **Coordinator in `coordinator.rs`:** Update `SetDescription` handler — line 388 currently does `item.description = Some(description)`. The compiler will enforce the type change. If the handler receives a `String` from somewhere, it may need to parse it via `parse_description()` — check the call site and update accordingly.
- [x] **CLI in `main.rs`:**
  - Remove `--description` arg from `Commands::Add` (line 72-73)
  - Remove `description` from `handle_add()` signature and its `backlog::add_item()` call
  - Update `handle_init()`: change empty backlog `schema_version` from 2 to 3
- [x] **Test fixture updates:**
  - Update `tests/fixtures/backlog_full.yaml`: bump `schema_version` to 3
  - Scan for any other v2 fixtures (e.g., `backlog_empty.yaml`, `backlog_minimal.yaml`, `backlog_unknown_fields.yaml`) and bump to 3 where appropriate. Leave `backlog_wrong_version.yaml` as-is if it tests wrong-version behavior.
- [x] **Test common updates:**
  - Update `tests/common/mod.rs`: change `make_backlog()` to `schema_version: 3`
- [x] **Migration tests in `migration_test.rs`:**
  - `migrate_v2_full_fixture`: Copy `backlog_v2_full.yaml` to tempdir, migrate, verify `schema_version == 3`
  - `migrate_v2_to_v3_with_descriptions`: Create inline v2 YAML with a convention-formatted description (5 sections), migrate, verify each `StructuredDescription` field has correct content
  - `migrate_v2_to_v3_freeform_description`: Create inline v2 YAML with freeform description (no headers), verify `context` contains full text and other fields are empty strings
  - `migrate_v2_to_v3_no_description`: Item with `description: ~` in v2 stays `None` after migration
  - `migrate_v3_is_noop`: V3 file loads without re-migration
  - `migrate_chain_v1_to_v3`: Start with v1 fixture, call `load()`, verify result is `schema_version == 3` with correct items
  - `migrate_forward_compat`: Create YAML with `schema_version: 99`, call `load()`, verify error message contains "Unsupported schema_version"
- [x] **Prompt tests in `prompt_test.rs`:**
  - `build_prompt_includes_structured_description`: Create item with populated `StructuredDescription` (all 5 fields non-empty), verify prompt output contains `## Description`, `**Context:**`, `**Problem:**`, `**Solution:**`, `**Impact:**`, `**Sizing Rationale:**`
  - `build_prompt_skips_empty_description_fields`: Create item with only `context` and `problem` populated (others empty string), verify prompt omits `**Solution:**`, `**Impact:**`, `**Sizing Rationale:**`
  - `build_prompt_omits_description_when_all_empty`: All fields empty strings → no `## Description` header in prompt
  - `build_prompt_excludes_description_when_none`: `description: None` → no `## Description` in prompt
  - Update `context_preamble_includes_description` test to use `StructuredDescription` instead of `String`
- [x] **Backlog tests in `backlog_test.rs`:**
  - Update inbox ingestion tests: verify ingested item has `description: None` regardless of inbox item description content
  - Update `add_item` tests: remove description parameter from calls
- [x] **Coordinator tests:** Update any `SetDescription` test to pass `StructuredDescription` instead of `String`
- [x] Run `cargo test` — all tests pass
- [x] Run `cargo clippy` — no new warnings (3 pre-existing)

**Verification:**

- [x] `cargo test` passes — all tests (existing and new) pass (487 passed, 0 failed)
- [x] `cargo build` succeeds with no warnings
- [x] `cargo clippy` passes (no new warnings; 3 pre-existing)
- [x] Loading a v2 BACKLOG.yaml produces v3 with structured descriptions (tested via `migrate_v2_full_fixture`, `migrate_v2_with_structured_descriptions`)
- [x] Loading a v3 BACKLOG.yaml returns as-is (no re-migration) (tested via `migrate_v2_persisted_file_is_valid_v3`)
- [x] v1→v2→v3 migration chain works end-to-end (tested via `migrate_chain_v1_to_v3_via_load`)
- [x] Prompt rendering includes structured descriptions with labeled fields (tested via `build_prompt_includes_structured_description`)
- [x] CLI `orchestrate add` works without `--description` flag (flag removed from clap struct)
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-028][P2] Feature: v2→v3 migration, structured descriptions in type system, prompt rendering, and integration`

**Notes:**

This is the largest phase because the type change (`Option<String>` → `Option<StructuredDescription>`) cascades across the entire codebase via Rust's type system. All changes must be made together for compilation. The approach: make the type change first, then fix all compilation errors systematically (migration, backlog, coordinator, CLI, prompt, tests), then add new tests.

**Inbox description handling:** The `ingest_inbox_items()` change from `inbox_item.description.clone()` to `description: None` means inbox freeform descriptions don't flow to the BacklogItem. This is acceptable because: (1) once `build_preamble()` renders structured descriptions, phase agents see descriptions on items that have them; (2) for inbox items, the title provides primary context for triage; (3) richer inbox description support is tracked as a follow-up.

**Migration dispatch pattern:** The current `load()` uses `schema_version < EXPECTED_SCHEMA_VERSION` to detect "needs migration" and `schema_version != EXPECTED` to detect "unsupported future version." The update chains migrations sequentially: check for v1 first, then check for v2, each writing to disk before the next runs. This is retry-safe — if v1→v2 succeeds but v2→v3 fails, the file is at v2 on disk and next `load()` skips v1→v2 and retries v2→v3.

**Followups:**

- [ ] [Medium] Add inbox description rendering to triage prompt — triage agents should see `InboxItem.description` freeform text even when `BacklogItem.description` is None. Requires `build_triage_prompt()` to accept optional inbox context.
- [ ] [Medium] Extract shared `atomic_write_yaml` helper — the temp-file + sync + rename pattern is duplicated in `backlog::save`, `migrate_v1_to_v2`, and `migrate_v2_to_v3`. Consolidating reduces risk of divergence.
- [ ] [Low] `map_v2_item` could take ownership instead of cloning — change signature to `fn map_v2_item(v2: V2BacklogItem) -> BacklogItem` and use `.into_iter()` for zero-cost field moves. Low priority since migration runs once.
- Fixed: Stale doc comments on `load()` and `migrate_v1_to_v2()` updated to reflect v3 migration chain.
- Fixed: `Box<BacklogItem>` in `CoordinatorCommand::WriteWorklog` to resolve clippy `large_enum_variant` warning caused by larger `BacklogItem` with `StructuredDescription`.
- Fixed: `migrate_v1_to_v2` idempotency path updated to parse as `V2BacklogFile` (not `BacklogFile`) for forward-compatibility with v3 types.
- Fixed: `load()` dispatch returns migrated backlog directly from `migrate_v2_to_v3` instead of re-parsing stale file contents.

---

### Phase 3: Data Migration & Final Verification

> Migrate production BACKLOG.yaml and verify all descriptions migrated correctly

**Phase Status:** complete

**Complexity:** Low

**Goal:** Run the migration on the actual BACKLOG.yaml, verify correctness of all 7 migrated descriptions, and commit code + data atomically.

**Files:**

- `BACKLOG.yaml` — modify — Migrated from schema_version 2 to 3 (automated via `load()`)

**Tasks:**

- [x] Build the orchestrator: `cargo build` in the orchestrator directory
- [x] Run the orchestrator once to trigger migration: `cd /home/sirhamy/Code/ai-dotfiles && .claude/skills/changes/orchestrator/target/debug/orchestrate --root . status` (calls `load()` which triggers v2→v3 migration)
- [x] Verify the migrated BACKLOG.yaml programmatically:
  - Verify `schema_version: 3` at top of file
  - Verify WRK-024, WRK-025, WRK-028, WRK-032, WRK-033 have `description` as a structured map with `context`, `problem`, `solution`, `impact`, `sizing_rationale` keys (convention-formatted)
  - Verify WRK-050 has `description.context` containing the full freeform text and `description.problem`, `description.solution`, `description.impact`, `description.sizing_rationale` are empty strings
  - Verify WRK-051 has `description.context` containing the full freeform text and other fields are empty strings
  - Verify all 46 items without descriptions still have no `description` key (53 total - 7 with descriptions = 46)
- [x] Run `cargo test` one final time to confirm everything still passes with the migrated data (487 passed, 0 failed)
- [x] Stage all changes (code + migrated BACKLOG.yaml) for atomic commit

**Verification:**

- [x] BACKLOG.yaml has `schema_version: 3`
- [x] All 7 existing descriptions are preserved (no data loss)
- [x] 5 convention-formatted descriptions (WRK-024, WRK-025, WRK-028, WRK-032, WRK-033) have all 5 fields populated
- [x] 2 freeform descriptions (WRK-050, WRK-051) have text in `context` field, empty strings in other fields
- [x] 46 items without descriptions remain unchanged (53 total - 7 with descriptions)
- [x] `cargo test` passes (487 passed, 0 failed)
- [x] All PRD success criteria verified (checklist in Final Verification below)
- [x] Code review passes (data-only migration, clean diff, no issues)

**Commit:** `[WRK-028][P3] Feature: Migrate BACKLOG.yaml to v3 structured descriptions`

**Notes:**

This phase is the "point of no return" for the BACKLOG.yaml migration. The v2 data is always recoverable via git history. After this commit, the orchestrator expects schema_version 3.

**Followups:**

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met:
  - [x] `StructuredDescription` struct defined with five `String` fields and serde derives
  - [x] `BacklogItem::description` changed to `Option<StructuredDescription>`
  - [x] `ItemUpdate::SetDescription` payload changed to `StructuredDescription`
  - [x] v2→v3 migration parses convention-formatted descriptions by section headers
  - [x] v2→v3 migration places non-conforming descriptions in `context` field
  - [x] `EXPECTED_SCHEMA_VERSION` bumped to 3
  - [x] Schema version in BACKLOG.yaml bumped to 3 with descriptions migrated
  - [x] `build_context_preamble` renders structured descriptions with labeled sections
  - [x] `build_preamble` renders structured descriptions for phase agents
  - [x] All tests updated and passing (487 passed, 0 failed)
  - [x] `InboxItem::description` remains `Option<String>`
  - [x] Migration is idempotent (loading a v3 file returns it as-is)
  - [x] Migration handles edge cases for all 7 existing descriptions
  - [x] Coordinator `SetDescription` handler updated
  - [x] CLI `--description` flag removed
- [x] Tests pass (487 passed, 0 failed)
- [x] No regressions introduced
- [x] Code reviewed (if applicable)

## Execution Log

<!-- Updated automatically during autonomous execution via /implement-spec -->
<!-- Each phase agent appends an entry when it completes -->

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| Phase 1: Type & Parser Foundation | complete | `[WRK-028][P1] Feature: Add StructuredDescription type and section header parser` | 12 parser tests, added Default+serde(default), debug assertion for ASCII safety |
| Phase 2: Migration, Type Propagation & Integration | complete | `[WRK-028][P2] Feature: v2→v3 migration, structured descriptions in type system, prompt rendering, and integration` | 487 tests pass, 0 new clippy warnings. Fixed: load() dispatch bug, migrate_v1_to_v2 idempotency path, Box<BacklogItem> for large_enum_variant. 8 new migration tests, 4 new prompt tests. |
| Phase 3: Data Migration & Final Verification | complete | `[WRK-028][P3] Migrate BACKLOG.yaml to v3 structured descriptions` | 53 items migrated, 5 convention-formatted + 2 freeform descriptions parsed correctly, 46 items without descriptions unchanged. 487 tests pass. |

## Followups Summary

### Critical

<!-- Items that should be addressed before shipping -->

### High

- [ ] Add inbox description rendering to triage prompt — when inbox items with descriptions are ingested, the freeform text should flow to the triage agent via the prompt. Currently `build_triage_prompt()` doesn't render descriptions and `ingest_inbox_items()` sets `description: None`. Deferred because descriptions are still visible via item titles and this requires additional `build_triage_prompt()` refactoring.

### Medium

- [ ] Triage prompt schema changes — Make triage output structured descriptions via JSON output schema so triage agents can produce `SetDescription(StructuredDescription)` updates. Deferred because it involves prompt engineering beyond the type system change.
- [ ] `Display` impl for `StructuredDescription` — Consistent formatting in CLI output (e.g., `orchestrate status`). Not blocking for v1.
- [ ] Flexible deserialization for `StructuredDescription` — Accept plain string in YAML (maps to `context` field). Follows `FollowUp` pattern. Deferred because migration handles all known cases and this adds complexity.

### Low

- [ ] `orchestrate status` display — Show structured descriptions or a summary in status output. Currently shows nothing.
- [ ] Extract shared `atomic_write_yaml` helper — The temp-file + sync + rename pattern is duplicated in `backlog::save`, `migrate_v1_to_v2`, and `migrate_v2_to_v3`. Consolidating reduces risk of divergence. (From Phase 2)
- [ ] `map_v2_item` could take ownership instead of cloning — Change signature to `fn map_v2_item(v2: V2BacklogItem) -> BacklogItem` for zero-cost field moves. Low priority since migration runs once. (From Phase 2)

## Design Details

### Key Types

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

All fields are `String` (not `Option<String>`). If the struct exists, all fields are present. Empty strings indicate "not yet populated." `None` = no description ever set. `Some(StructuredDescription { ... })` = description exists.

### Architecture Details

**Migration chain dispatch in `load()`:**
```
Read schema_version from YAML
if schema_version < EXPECTED_SCHEMA_VERSION:
    if schema_version == 1 → migrate_v1_to_v2(path, pipeline)
    // File is now v2 on disk (or was already v2)
    if schema_version <= 2 → migrate_v2_to_v3(path)
    // File is now v3 on disk
if schema_version != EXPECTED → error "Unsupported schema_version"
Re-read file as BacklogFile and return
```

Each migration reads the file, transforms it, and writes atomically. The next migration reads the updated file. Retry-safe: if v1→v2 succeeds but v2→v3 fails, file is at v2 on disk. Next `load()` skips v1→v2 and retries v2→v3.

**Section header parser algorithm:**
```
fn parse_description(text: &str) -> StructuredDescription:
    Split text into lines
    Track current_section (initially None) and accumulated lines per section
    For each line:
        Trim the line
        Check if trimmed lowercase starts with known header prefix
        If match: start new section, include content after colon on the same line
        If no match: append to current section (or buffer if no section started)
    Finalize: join lines per section, trim whitespace
    If no headers ever matched: entire text → context field, others empty
    Return StructuredDescription
```

**Prompt rendering:**
```
fn render_structured_description(desc: &StructuredDescription) -> String
    For each field in order [Context, Problem, Solution, Impact, Sizing Rationale]:
        If field is non-empty: emit "**{Label}:** {content}"
    Join with newlines
    Return (empty string if all fields empty)
```

### Design Rationale

- **Pure string ops over regex:** 5 fixed headers don't warrant adding `regex` crate dependency. `str::to_lowercase()` + `str::starts_with()` is sufficient and zero-dependency.
- **Sequential migration dispatch:** Simple, maintainable, each step isolated and retry-safe. Slightly more disk I/O (read-write-read-write for v1 files) but negligible for small YAML files.
- **CLI --description removal:** No ergonomic way to specify five separate fields via CLI flags. Items get descriptions during triage.
- **Inbox ingestion sets None:** Freeform inbox text can't map directly to structured fields without parsing, and parsing during ingestion would be premature. Triage is the right place to set structured descriptions. Richer inbox description support is a follow-up.
- **Phases 2 merged (migration + type propagation):** The `migrate_v2_to_v3()` function must return a `BacklogFile` with `Option<StructuredDescription>` on `BacklogItem::description`. This creates a hard compilation dependency between the migration and the type change. Attempting to separate them into phases would leave intermediate phases that don't compile.

---

## Assumptions

Decisions made without human input during autonomous SPEC creation:

1. **Phase 2 and 3 merged into single phase.** The original plan had separate phases for migration infrastructure and type propagation, but `migrate_v2_to_v3()` must return a `BacklogFile` with the new `Option<StructuredDescription>` type, creating a hard compilation dependency. All consumers must update atomically with the type change.
2. **Inbox description loss accepted for now.** The `ingest_inbox_items()` behavior change (setting `description: None`) means inbox freeform descriptions don't flow to BacklogItem. This is acceptable because triage agents primarily work from item titles, and richer inbox description support is tracked as a follow-up.
3. **`build_context_preamble` updated despite being dead code.** PRD lists this as Must Have. Updated for consistency with `build_preamble` and future use.
4. **No flexible deserialization in v1.** The Nice to Have `FollowUp`-style flexible deser is deferred. The migration handles all known cases, and adding flexible deser adds complexity with poor error messages.
5. **7 descriptions, not 9.** BACKLOG.yaml has 7 items with descriptions (WRK-024, WRK-025, WRK-028, WRK-032, WRK-033, WRK-050, WRK-051), of which 5 follow the convention format and 2 are freeform.

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
