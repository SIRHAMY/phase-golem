# SPEC: Switch Worklog to Append-Only Writes

**ID:** WRK-023
**Status:** Ready
**Created:** 2026-02-13
**PRD:** ./WRK-023_switch-worklog-to-append-only_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

The orchestrator's worklog module uses a prepend-and-rewrite pattern (`fs::read_to_string` + `format!("{}{}", new, old)` + `fs::write`) for every worklog entry. This is O(n) with file size for what is fundamentally append-only data. The current month's worklog file is already 1,200+ lines. Two functions use this pattern: `write_entry()` in `worklog.rs` and `write_archive_worklog_entry()` in `backlog.rs`. Both need the same mechanical replacement.

## Approach

Replace the 3-line prepend-and-rewrite pattern with a 2-line `OpenOptions::append(true).create(true)` + `write_all()` pattern in both functions. This is a mechanical replacement — function signatures, entry format strings, error handling contract (`Result<(), String>`), directory creation, and all callers remain unchanged. The only observable difference is that entries appear in chronological order (oldest-first) instead of reverse-chronological order (newest-first).

**Patterns to follow:**

- `orchestrator/src/worklog.rs:32-33` — existing `create_dir_all` + `map_err` error mapping pattern (preserved as-is)
- `orchestrator/src/backlog.rs:429-430` — same `create_dir_all` + `map_err` pattern

**Implementation boundaries:**

- Do not modify: `coordinator.rs`, `scheduler.rs`, `types.rs` — callers are unchanged
- Do not modify: entry format strings — the `format!()` calls remain identical
- Do not refactor: the API difference between `write_entry()` (takes directory) and `write_archive_worklog_entry()` (takes file path) — pre-existing inconsistency, out of scope

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Append-only writes + test updates | Low | Replace prepend-and-rewrite with append in both functions, update tests |

**Ordering rationale:** Single phase — the change is small enough that splitting would create artificially tiny phases. Both functions get the same mechanical replacement, and tests must be updated atomically with the implementation.

---

## Phases

### Phase 1: Append-only writes + test updates

> Replace prepend-and-rewrite with append in both functions, update all tests

**Phase Status:** complete

**Complexity:** Low

**Goal:** Switch `write_entry()` and `write_archive_worklog_entry()` from prepend-and-rewrite to append-only using `OpenOptions`, and update tests to assert chronological ordering.

**Files:**

- `orchestrator/src/worklog.rs` — modify — add `use std::io::Write;` import, replace lines 41-46 (prepend logic) with `OpenOptions::append` + `write_all`, update doc comment on line 8
- `orchestrator/src/backlog.rs` — modify — add `use std::io::Write;` import, replace lines 439-444 (prepend logic) with `OpenOptions::append` + `write_all`, update doc comment on line 420
- `orchestrator/tests/worklog_test.rs` — modify — rename `write_entry_prepends_newest_at_top` to `write_entry_appends_chronologically`, reverse ordering assertion (lines 141-144)
- `orchestrator/tests/backlog_test.rs` — modify — add test `archive_worklog_entry_appends_chronologically` verifying oldest-first ordering

**Tasks:**

- [x] Add `use std::io::Write;` import to `worklog.rs` (after line 1)
- [x] In `write_entry()`: replace lines 41-46 (comment + `read_to_string` + `format!` prepend + `fs::write`) with `OpenOptions::new().append(true).create(true).open(&worklog_path).map_err(...)? ` + `file.write_all(entry.as_bytes()).map_err(...)?`
- [x] Update `write_entry()` doc comment (line 8): change "Prepends (newest-at-top)" to "Appends an entry to"
- [x] Add `use std::io::Write;` import to `backlog.rs` (after line 1)
- [x] In `write_archive_worklog_entry()`: replace lines 439-444 (same prepend pattern) with identical `OpenOptions` + `write_all` pattern
- [x] Update `write_archive_worklog_entry()` doc comment (line 420): change "Write a simple worklog entry" to "Append a worklog entry" or similar
- [x] In `worklog_test.rs`: rename test `write_entry_prepends_newest_at_top` → `write_entry_appends_chronologically`
- [x] In `worklog_test.rs`: reverse assertion — change `pos_second < pos_first` to `pos_first < pos_second`, update message to "Expected WRK-001 (older) to appear before WRK-002 (newer)"
- [x] In `backlog_test.rs` (PRD "Should Have"): add test `archive_worklog_entry_appends_chronologically` — archive two items sequentially via `backlog::archive_item()` (public API, since `write_archive_worklog_entry()` is private), verify first-archived appears before second-archived in the worklog file
- [x] Run `cargo test` in the orchestrator directory — all tests must pass

**Verification:**

- [x] `cargo test` (full suite, unfiltered) passes with zero failures — confirms no regressions in unrelated tests
- [x] `cargo test write_entry_appends_chronologically` passes — confirms chronological ordering for `write_entry()`
- [x] `cargo test archive_worklog_entry_appends_chronologically` passes — confirms chronological ordering for `write_archive_worklog_entry()`
- [x] `cargo test write_entry_creates_file` passes — confirms file creation still works
- [x] `cargo test write_entry_creates_parent_dirs` passes — confirms directory creation still works
- [x] `cargo test write_entry_contains_expected_fields` passes — confirms entry format unchanged
- [x] `cargo build` succeeds with no warnings related to this change
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-023][P1] Fix: Replace prepend-and-rewrite with append-only writes in worklog`

**Notes:**

- The `write_all()` method requires `std::io::Write` trait in scope — this is the only new import needed in each file.
- Error messages in `map_err` should include the file path for debuggability, matching the existing pattern: `format!("Failed to open worklog at {}: {}", path.display(), e)` for the `OpenOptions::open()` call and `format!("Failed to write worklog at {}: {}", path.display(), e)` for the `write_all()` call.
- The entry format strings already end with `\n\n---\n\n`, so appended entries are properly separated without any additional newline management.
- The new `backlog_test.rs` test must use `backlog::archive_item()` (the public function) since `write_archive_worklog_entry()` is private. This tests the same code path through the public API.

**Followups:**

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met
- [x] Tests pass
- [x] No regressions introduced
- [x] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | complete | `[WRK-023][P1] Fix: Replace prepend-and-rewrite with append-only writes in worklog` | All 681 tests pass. 9-agent code review passed. Fixed one stale comment in worklog_test.rs. |

## Followups Summary

### Critical

### High

### Medium

### Low

## Design Details

### Key Types

No new types introduced. Existing types unchanged:

```rust
// write_entry signature (unchanged)
pub fn write_entry(
    worklog_dir: &Path,
    item: &BacklogItem,
    phase: &str,
    outcome: &str,
    result_summary: &str,
) -> Result<(), String>

// write_archive_worklog_entry signature (unchanged)
fn write_archive_worklog_entry(worklog_path: &Path, item: &BacklogItem) -> Result<(), String>
```

### Architecture Details

No architectural changes. The write mechanism changes from:

```
read_to_string() → format!(new + old) → fs::write(combined)
```

To:

```
OpenOptions::append().create().open() → write_all(new)
```

Callers, file layout, entry format, and error contract are all preserved.

### Design Rationale

See Design doc for full rationale. Key points:
- `OpenOptions::append(true).create(true)` eliminates the file read entirely
- `write_all()` is preferred over `write!()` because the entry is already a complete pre-formatted string
- Single-process assumption means no file locking needed — `O_APPEND` provides sufficient per-call atomicity

---

## SPEC Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Initial SPEC draft | Single-phase SPEC for mechanical append-only replacement |
| 2026-02-13 | Self-critique (6 agents) | Auto-fixed: error message pattern with path context, explicit assertion reversal details, full test suite verification, archive test PRD priority annotation. No directional or critical issues. |
| 2026-02-13 | Phase 1 build | Complete. Replaced prepend-and-rewrite with append-only in worklog.rs and backlog.rs. Updated tests. All 681 tests pass. 9-agent code review passed with no critical/high issues. |
