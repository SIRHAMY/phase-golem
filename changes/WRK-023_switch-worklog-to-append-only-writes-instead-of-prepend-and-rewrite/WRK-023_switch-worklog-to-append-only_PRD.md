# Change: Switch Worklog to Append-Only Writes

**Status:** Proposed
**Created:** 2026-02-13
**Author:** Orchestrator

## Problem Statement

The orchestrator's worklog module uses a **prepend-and-rewrite pattern** (read entire file, prepend new entry, rewrite whole file) for every entry. This pattern has two problems:

1. **I/O overhead grows linearly with file size.** Each write reads and rewrites the full file. The current month's worklog (`_worklog/2026-02.md`) is already 1,200+ lines and ~80 KB. Every phase completion, failure, or triage event (a "phase transition" — any scheduler event that writes to the worklog, such as Complete, Failed, Blocked, Cancelled, or triage outcomes) triggers a full file rewrite.

2. **Prepend-and-rewrite is non-standard for append-only data.** Worklog entries are immutable once written — they are never modified, deleted, or read back programmatically by the orchestrator. This is append-only data, yet the implementation uses a read-modify-write cycle that is unnecessary and violates the principle of least surprise for log-like data.

The same pattern exists in `write_archive_worklog_entry()` in `backlog.rs`, which writes archival entries to the worklog when items are marked Done.

## User Stories / Personas

- **Orchestrator system** — Writes worklog entries at each phase transition (6 call sites in `scheduler.rs`: Complete, Subphase Complete, Failed, Blocked, Cancelled, and triage). Benefits from reduced I/O per write.

- **Developer/operator** — Reads worklog files to understand orchestrator execution history. Entry ordering changes from newest-at-top to oldest-at-top (chronological). Can use `tail` to see recent entries, matching standard log file conventions.

## Desired Outcome

After this change, worklog writes use `OpenOptions` with append mode instead of read-modify-write. New entries are appended to the end of the file. The entry format, content, and file naming (`_worklog/YYYY-MM.md`) remain identical. The only observable difference is that entries appear in chronological order (oldest first) instead of reverse-chronological order (newest first).

## Success Criteria

### Must Have

- [ ] `write_entry()` in `worklog.rs` appends to the file instead of prepending and rewriting
- [ ] `write_archive_worklog_entry()` in `backlog.rs` appends to the file instead of prepending and rewriting
- [ ] New entries are written at the end of the file (chronological order)
- [ ] Entry format is unchanged: `## {datetime} — {id} ({title})` header, phase/outcome/summary fields, `---` separator
- [ ] Archive entry format is unchanged: `## {datetime} — {id} ({title})` header, status/phase fields, `---` separator
- [ ] File creation still works when the worklog file does not yet exist
- [ ] Parent directory creation still works when `_worklog/` does not exist
- [ ] All existing tests pass (updated to reflect new ordering — specifically `write_entry_prepends_newest_at_top` renamed/rewritten to assert oldest-first order)
- [ ] `write_entry()` function signature is unchanged — callers in `coordinator.rs` and `scheduler.rs` require zero code changes
- [ ] Error handling behavior is unchanged — `write_entry()` and `write_archive_worklog_entry()` continue to return `Result<(), String>` with descriptive error messages on I/O failure, same as before

### Should Have

- [ ] Doc comments on `write_entry()` and `write_archive_worklog_entry()` updated to reflect append behavior (e.g., "Appends entry to `_worklog/YYYY-MM.md`" instead of "Prepends")
- [ ] `write_archive_worklog_entry()` ordering behavior verified by an existing or new test in `backlog_test.rs`

### Nice to Have

- [ ] No read of the existing file is required — the implementation uses `OpenOptions::append(true).create(true)` or equivalent, eliminating the explicit `fs::read_to_string()` call entirely

## Scope

### In Scope

- `write_entry()` in `worklog.rs` — switch from prepend-and-rewrite to append
- `write_archive_worklog_entry()` in `backlog.rs` — switch from prepend-and-rewrite to append
- Test updates in `worklog_test.rs` — update ordering assertion to expect chronological (oldest-first) order
- Doc comment updates on modified functions

### Out of Scope

- Reversing the order of entries in existing `_worklog/*.md` files (existing files retain their current order)
- Adding worklog reading/parsing/querying functionality
- Changing the worklog file naming scheme (`YYYY-MM.md`)
- Changing the worklog entry format or fields
- Adding log rotation, size limits, or cleanup logic
- Changing how the coordinator or scheduler calls worklog functions (callers are unchanged)
- Concurrent write safety (orchestrator is single-process; concurrent appends are not a current concern)
- Performance benchmarking
- Adding `sync_all()` / fsync durability guarantees (pre-existing gap, not introduced by this change)
- Unifying the API difference between `write_entry()` (takes directory path) and `write_archive_worklog_entry()` (takes full file path) — pre-existing inconsistency

## Non-Functional Requirements

- **Performance:** Append-only writes should be O(1) with respect to file size, eliminating the current O(n) read-modify-write cycle. This assumes local filesystem access.

## Constraints

- The `write_entry()` function signature must not change — callers in `coordinator.rs` and `scheduler.rs` must not require modification.
- The `write_archive_worklog_entry()` function is private to `backlog.rs` — its signature may change if needed, but the change should be minimal.

## Dependencies

- **Depends On:** None
- **Blocks:** None

## Risks

- [ ] **Human readability of chronological order:** Operators accustomed to newest-at-top may need to adjust to using `tail` instead of reading from the top. Mitigation: This matches standard log file conventions (`syslog`, nginx access logs, etc.) and is a well-understood pattern.
- [ ] **Mixed-order transition month:** The worklog file active at deployment time will have older entries in reverse-chronological order (from prepend era) at the top, and newer entries in chronological order (from append era) at the bottom. Mitigation: Worklog files are informational and not parsed programmatically. The mixed state resolves naturally at the next month boundary when a fresh file is created.

## Open Questions

(None — all questions resolved during discovery.)

## Assumptions

Decisions made without human input:

1. **Chronological ordering is acceptable.** Switched from newest-at-top to oldest-at-top (append order). This matches industry-standard log file conventions and eliminates the need for a read-modify-write cycle. Operators can use `tail` for recent entries.

2. **Both functions should be updated.** `write_archive_worklog_entry()` in `backlog.rs` uses the same prepend pattern and should be updated for consistency, even though it writes less frequently.

3. **Existing worklog files are not migrated.** The current `_worklog/2026-02.md` retains its reverse-chronological order. New entries appended after this change will appear at the bottom in chronological order. This creates a mixed-order file during the transition month, which is acceptable for informational data that is not parsed programmatically. The mixed state resolves naturally when a new month's file is created.

4. **Error handling is unchanged.** Both functions already return `Result<(), String>` with descriptive error messages. The append implementation preserves this contract — callers see no difference in error behavior.

## References

- `worklog.rs` — `write_entry()` function
- `backlog.rs` — `write_archive_worklog_entry()` function
- `worklog_test.rs` — Test suite including ordering test (`write_entry_prepends_newest_at_top`)
- `coordinator.rs` — `handle_write_worklog()` caller
- `scheduler.rs` — 6 call sites: Complete, Subphase Complete, Failed, Blocked, Cancelled, triage
- Triage summary: "Direct promotion. Small, low-risk change: swap prepend-and-rewrite to append-only in worklog.rs write_entry(), plus update tests in worklog_test.rs."
