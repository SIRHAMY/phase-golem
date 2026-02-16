# SPEC: Optimize BACKLOG.yaml to Single YAML Parse Pass

**ID:** WRK-002
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-002_optimize-migration-to-single-yaml-parse-pass-instead-of-double-parsing_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

The `backlog::load()` function in `orchestrator/src/backlog.rs` parses BACKLOG.yaml twice for the common v3 case: once to `Value` for version extraction (line 28), then again to `BacklogFile` via `from_str` (line 65). This is structurally redundant — the second parse repeats all YAML lexing and parsing on the same input. The design identified a one-line fix: replace `from_str::<BacklogFile>(&contents)` with `from_value::<BacklogFile>(version_check)`, reusing the already-parsed `Value` tree.

## Approach

Replace the second `serde_yaml_ng::from_str()` call on line 65 of `backlog.rs` with `serde_yaml_ng::from_value()`, which converts the already-parsed `Value` tree into the typed `BacklogFile` struct without re-parsing the YAML text. The function signature, behavior, and callers remain unchanged.

The PRD requires a pre-implementation verification test proving that `from_value()` produces identical results to `from_str()` for `BacklogFile` deserialization, including `#[serde(default)]` fields and the custom `FollowUp` deserializer. This test must pass before the production code is modified.

**Patterns to follow:**

- `orchestrator/tests/backlog_test.rs` — existing test patterns (fixture loading, assertion style, imports)
- `orchestrator/src/backlog.rs:28-34` — existing `from_str::<Value>()` and version extraction pattern

**Implementation boundaries:**

- Do not modify: `orchestrator/src/migration.rs` (migration functions remain unchanged per PRD scope)
- Do not modify: `orchestrator/src/types.rs` (no type changes needed)
- Do not modify: any existing tests (all 43+ backlog tests and 16+ migration tests must pass unchanged)
- Do not add: new dependencies to `Cargo.toml`

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Pre-implementation verification | Low | Write test proving `from_value()` and `from_str()` produce identical `BacklogFile` results |
| 2 | Single-parse implementation | Low | Replace `from_str` with `from_value` on line 65 of `backlog.rs` and verify all tests pass |

**Ordering rationale:** Phase 1 must complete first because the PRD requires pre-implementation verification that `from_value()` is behaviorally equivalent before touching production code.

---

## Phases

Each phase should leave the codebase in a functional, stable state. Complete and verify each phase before moving to the next.

---

### Phase 1: Pre-implementation verification

> Write test proving `from_value()` and `from_str()` produce identical `BacklogFile` results

**Phase Status:** complete

**Complexity:** Low

**Goal:** Verify that `serde_yaml_ng::from_value::<BacklogFile>()` produces identical results to `serde_yaml_ng::from_str::<BacklogFile>()` for representative inputs, satisfying the PRD's pre-implementation verification requirement.

**Files:**

- `orchestrator/tests/backlog_test.rs` — modify — add new test function `test_from_value_matches_from_str_for_backlog_file`

**Patterns:**

- Follow existing test structure in `backlog_test.rs` (imports, assertion style)
- Use inline YAML string rather than fixture file, since this test verifies serde behavior rather than load path behavior

**Tasks:**

- [x] Add `test_from_value_matches_from_str_for_backlog_file` test to `backlog_test.rs`
  - Construct an inline YAML string containing `schema_version: 3` with at least 3 items:
    - **Item 1 (full):** All fields populated — `id`, `title`, `status: in_progress`, `phase`, `size`, `complexity`, `risk`, `impact`, `tags`, `dependencies`, `blocked_type`, `blocked_reason`, `blocked_from_status`, `requires_human_review: true`, `follow_ups` (structured format with `title` + `context`), `updated_assessments`
    - **Item 2 (minimal):** Only required fields (`id`, `title`, `status: new`, `created`, `updated`) — exercises `#[serde(default)]` on all optional fields
    - **Item 3 (mixed):** Some optional fields populated, FollowUp as string-only format (exercises custom `FollowUp` deserializer union handling), `status: blocked`
  - Include an unknown top-level field (e.g., `future_field: true`) to verify forward compatibility (unknown fields silently ignored, no `deny_unknown_fields`)
  - Parse the string via `serde_yaml_ng::from_str::<BacklogFile>()` → `direct`
  - Parse the string via `serde_yaml_ng::from_str::<serde_yaml_ng::Value>()` then `serde_yaml_ng::from_value::<BacklogFile>()` → `via_value`
  - Assert `direct == via_value` (uses `PartialEq` derive on `BacklogFile`)
- [x] Add `test_from_value_error_preserves_field_context` test to `backlog_test.rs`
  - Construct malformed YAML with an invalid enum value (e.g., `status: invalid_status`)
  - Parse via `from_str::<Value>()` then `from_value::<BacklogFile>()` and capture the error
  - Assert the error message contains the field name or description (e.g., `"unknown variant"`)
  - This verifies error quality is acceptable even without line/column info
- [x] Run the new tests and confirm they pass

**Verification:**

- [x] Equivalence test passes: `cargo test -p orchestrate test_from_value_matches_from_str_for_backlog_file`
- [x] Error quality test passes: `cargo test -p orchestrate test_from_value_error_preserves_field_context`
- [x] All existing tests still pass: `cargo test -p orchestrate`
- [x] Codebase builds without errors: `cargo build -p orchestrate`

**Commit:** `[WRK-002][P1] Feature: Add pre-implementation verification test for from_value equivalence`

**Notes:**

The `BacklogFile` struct already derives `PartialEq`, so direct comparison works. The test YAML covers three key edge case categories: (1) missing optional fields exercising `#[serde(default)]` on `Option`, `bool`, and `Vec` fields, (2) FollowUp in both string and structured formats exercising the custom `Deserialize` impl in `types.rs`, and (3) an unknown top-level field exercising forward compatibility (no `deny_unknown_fields` on `BacklogFile`). The error test verifies that `from_value()` error messages contain useful field/variant info even without line/column positions.

**Followups:**

---

### Phase 2: Single-parse implementation

> Replace `from_str` with `from_value` on line 65 of `backlog.rs`

**Phase Status:** complete

**Complexity:** Low

**Goal:** Eliminate the redundant second YAML parse in `backlog::load()` for v3 files by replacing `serde_yaml_ng::from_str::<BacklogFile>(&contents)` with `serde_yaml_ng::from_value::<BacklogFile>(version_check)`.

**Files:**

- `orchestrator/src/backlog.rs` — modify — replace `from_str(&contents)` with `from_value(version_check)` on line 65

**Tasks:**

- [x] Replace line 65 of `backlog.rs`: change `serde_yaml_ng::from_str(&contents)` to `serde_yaml_ng::from_value(version_check)`
  - The `.map_err()` wrapper on line 66 remains compatible (`from_value()` returns the same `serde_yaml_ng::Error` type)
  - The `version_check` variable is moved (consumed by value) into `from_value()` — version extraction on lines 31-34 must remain before this call (already the case)
- [x] Rename `version_check` to `parsed_yaml` — the variable now serves dual purpose (version extraction and typed conversion), so the old name is misleading per the project's "explicit over implicit" style principle
- [x] Run all tests to confirm no regressions

**Verification:**

- [x] All backlog tests pass: `cargo test -p orchestrate` (43+ tests)
- [x] All migration tests pass: `cargo test -p orchestrate` (16+ tests)
- [x] v3 files load correctly (covered by existing `load_full_backlog_with_all_field_variations` and `load_minimal_backlog` tests)
- [x] v1/v2 migration paths still work (covered by existing migration tests)
- [x] Error messages include file path context (the `.map_err()` wrapper is unchanged)
- [x] No redundant `from_str::<BacklogFile>` calls remain in `backlog.rs`: `grep 'from_str.*BacklogFile' orchestrator/src/backlog.rs` returns zero matches
- [x] Codebase builds without errors or warnings: `cargo build -p orchestrate`
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-002][P2] Feature: Replace double YAML parse with single parse in backlog::load()`

**Notes:**

After this change, the `contents` String is only used as input to `from_str::<Value>()` on line 28 — it is not referenced again in the v3 path. The `parsed_yaml` Value (formerly `version_check`) is consumed (moved) by `from_value()`, so version extraction on lines 31-34 must remain before the `from_value()` call (already the case). The error message from `from_value()` loses line/column position info but preserves error descriptions and the file path context added by `.map_err()`. This is acceptable since BACKLOG.yaml is machine-managed.

**Followups:**

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met:
  - [x] v3 files parsed exactly once (no redundant `from_str` calls on same content)
  - [x] All existing tests pass without modification
  - [x] Migration paths (v1 and v2) continue to work
  - [x] Error messages include file path context
  - [x] Pre-implementation verification test confirms `from_value()` equivalence
- [x] Tests pass
- [x] No regressions introduced
- [x] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | complete | `[WRK-002][P1] Feature: Add pre-implementation verification test for from_value equivalence` | Both equivalence and error quality tests pass. All 83+ existing backlog tests pass unchanged. |
| 2 | complete | `[WRK-002][P2] Feature: Replace double YAML parse with single parse in backlog::load()` | Replaced `from_str` with `from_value`, renamed `version_check` to `parsed_yaml`. All 350+ tests pass. Code review clean. |

## Followups Summary

### Critical

### High

### Medium

- [ ] [Medium] Optimize migration functions (`migrate_v1_to_v2`, `migrate_v2_to_v3`) to accept pre-parsed `Value` instead of re-reading from disk — same double-parse pattern exists in `migration.rs` at lines 184/205 and 451/467. Deferred per PRD scope; migration crash-safety must be carefully considered.
- [ ] [Medium] Consider adding `serde_path_to_error` for better error paths in `from_value()` errors — would add structural field paths (e.g., `items[3].status`) to compensate for lost line/column info. Requires a new dependency.

### Low

- [ ] [Low] Extract version extraction helper (`fn extract_schema_version(value: &Value) -> u32`) — the same pattern appears in 3 locations (`backlog.rs`, `migration.rs` x2). Minor DRY improvement.
- [ ] [Low] Add optional performance benchmark comparing single-parse vs double-parse — PRD "Nice to Have" item. Structural improvement is obvious so benchmark is not blocking.

## Design Details

### Key Types

No new types introduced. Existing types used:

- `serde_yaml_ng::Value` — intermediate parsed representation (already used at line 28)
- `serde_yaml_ng::from_value::<T>(Value) -> Result<T, Error>` — converts `Value` to typed struct without re-parsing YAML text (new usage, existing API)
- `BacklogFile` — target struct with `#[derive(PartialEq)]` (enables verification test)

### Architecture Details

No architectural changes. Single function modification within existing module boundary.

**Data flow change:**

```
Before: String → from_str::<Value>() → extract version → from_str::<BacklogFile>()
                   [parse #1]                               [parse #2]

After:  String → from_str::<Value>() → extract version → from_value::<BacklogFile>()
                   [parse #1]                               [convert, no re-parse]
```

### Design Rationale

- **`from_value()` over tagged enum:** Integer tags (`schema_version: 3`) may not work reliably with `serde_yaml_ng`'s YAML type coercion. Tagged enum approach would also require maintaining V1/V2 type definitions.
- **Migration functions unchanged:** Crash-safety requires each migration step to persist to disk before the next runs. Passing pre-parsed `Value` would require changing migration function signatures and carefully preserving the write-then-read guarantee.
- **Line/column loss accepted:** `from_value()` errors lose positional info, but BACKLOG.yaml is machine-managed and error descriptions (field names, type mismatches) are preserved.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
