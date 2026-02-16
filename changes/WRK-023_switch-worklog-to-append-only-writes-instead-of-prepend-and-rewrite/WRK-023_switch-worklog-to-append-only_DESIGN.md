# Design: Switch Worklog to Append-Only Writes

**ID:** WRK-023
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-023_switch-worklog-to-append-only_PRD.md
**Tech Research:** ./WRK-023_switch-worklog-to-append-only_TECH_RESEARCH.md
**Mode:** Light

## Overview

Replace the prepend-and-rewrite pattern in `write_entry()` (worklog.rs) and `write_archive_worklog_entry()` (backlog.rs) with `OpenOptions::append(true).create(true)` followed by `write_all()`. This eliminates the O(n) read-modify-write cycle, replacing it with O(1) appends. The change is a mechanical 3-line-to-2-line replacement in each function, with entry format, function signatures, and error handling unchanged. Entries shift from newest-first to chronological (oldest-first) order, matching standard log conventions.

---

## System Design

### High-Level Architecture

No architectural changes. The worklog subsystem retains its current shape:

```
scheduler.rs / coordinator.rs
        │
        ▼
  worklog::write_entry()          backlog::write_archive_worklog_entry()
        │                                    │
        ▼                                    ▼
  _worklog/YYYY-MM.md             _worklog/YYYY-MM.md
```

The only change is internal to the two write functions — callers and file layout are unaffected.

### Component Breakdown

#### `write_entry()` in worklog.rs

**Purpose:** Append a phase-transition worklog entry to the current month's file.

**Responsibilities:**
- Ensure `_worklog/` directory exists
- Format the entry (datetime, item ID, title, phase, outcome, summary)
- Append entry to `_worklog/YYYY-MM.md`, creating the file if needed

**Interfaces:**
- Input: `worklog_dir: &Path`, `item: &BacklogItem`, `phase: &str`, `outcome: &str`, `result_summary: &str`
- Output: `Result<(), String>`

**Dependencies:** `std::fs`, `chrono`, `crate::types::BacklogItem`

#### `write_archive_worklog_entry()` in backlog.rs

**Purpose:** Append an archival worklog entry when an item is marked Done.

**Responsibilities:**
- Ensure parent directory of `worklog_path` exists
- Format the entry (datetime, item ID, title, status, phase)
- Append entry to the provided file path, creating the file if needed

**Interfaces:**
- Input: `worklog_path: &Path`, `item: &BacklogItem`
- Output: `Result<(), String>`

**Dependencies:** `std::fs`, `chrono`, `crate::types::BacklogItem`

### Data Flow

1. Caller invokes `write_entry()` or `write_archive_worklog_entry()`
2. Function ensures directory exists via `fs::create_dir_all()` — errors mapped to `Err(String)` with directory path context
3. Function formats the entry string (unchanged from current)
4. Function opens the file with `OpenOptions::new().append(true).create(true).open(path)` — errors mapped to `Err(String)` with file path context
5. Function writes the entry with `file.write_all(entry.as_bytes())` — errors mapped to `Err(String)` with file path context
6. Returns `Ok(())`

### Key Flows

#### Flow: Write Worklog Entry (append)

> Phase-transition event triggers a worklog write.

1. **Directory creation** — `fs::create_dir_all(worklog_dir)` ensures `_worklog/` exists
2. **Entry formatting** — `format!()` builds the full markdown entry (identical to current)
3. **File open** — `OpenOptions::new().append(true).create(true).open(&worklog_path)` opens or creates the file in append mode
4. **Write** — `file.write_all(entry.as_bytes())` appends the entry at the end of the file
5. **Return** — `Ok(())` on success, `Err(String)` on I/O failure

**Edge cases:**
- File does not exist — `.create(true)` creates it; first entry is written to an empty file
- Directory does not exist — `create_dir_all` handles arbitrarily deep nesting
- I/O error on open or write — mapped to descriptive `Err(String)`, same as current behavior

#### Flow: Write Archive Worklog Entry (append)

> Item archival triggers an archival worklog write.

Identical flow to above, except:
- Takes a full file path instead of a directory
- Uses `worklog_path.parent()` + `create_dir_all` for directory creation
- Entry format includes Status and Phase fields (no Outcome or Summary)

---

## Technical Decisions

### Key Decisions

#### Decision: Use `write_all()` instead of `write!()` macro

**Context:** Both `write_all()` and `write!()` can write to a file handle. The entry is already formatted as a complete `String`.

**Decision:** Use `file.write_all(entry.as_bytes())`.

**Rationale:** The entry is pre-formatted via `format!()`. Both `write_all()` and `write!()` require `use std::io::Write;` in scope. The advantage of `write_all` is simplicity — it's a direct method call that writes a pre-built byte buffer, with no macro expansion or format-string parsing. This matches the "write a complete, pre-formatted entry" use case exactly.

**Consequences:** Requires `use std::io::Write;` import in both files. This is the only new import needed.

#### Decision: Preserve entry format exactly

**Context:** Entry format could theoretically be adjusted during this change.

**Decision:** Keep format strings identical. Only the write mechanism changes.

**Rationale:** Minimizes blast radius. Format changes are out of scope per the PRD. The current format strings already include correct trailing whitespace (`\n\n---\n\n`), so appended entries are properly separated without any additional newline management.

**Consequences:** Existing tooling or human readers see the same entry content, just in chronological rather than reverse-chronological order.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Order change | Entries shift from newest-first to oldest-first | O(1) writes, no file reads, simpler code | Matches standard log conventions; `tail` works for recent entries |
| Mixed-order transition | Active month file will have old entries (reverse-chrono) above new entries (chrono) | Zero-migration simplicity | Resolves naturally at next month boundary; worklog is informational |

---

## Alternatives Considered

### Alternative: Append with explicit newline management

**Summary:** Instead of including trailing newlines in the entry format string, manage newlines at the write boundary (e.g., check if file is empty, conditionally add leading newline).

**How it would work:**
- Read file size or check existence before writing
- Conditionally prepend a newline if file is non-empty

**Pros:**
- More control over whitespace between entries

**Cons:**
- Adds complexity (file size check or stat call)
- Current format string already includes proper spacing (`\n\n---\n\n` at the end)
- Introduces a read/stat where none is needed

**Why not chosen:** The current entry format already includes correct leading/trailing whitespace. No conditional logic is needed — just append the pre-formatted string.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Missing `std::io::Write` import causes compile error | Build failure, caught immediately | Low (easily caught by compiler) | Tech research flagged this; add import as part of the change |
| Mixed-order during transition month confuses operator | Low — informational data, not parsed programmatically | Low | Natural resolution at month boundary; operators can use `tail` for recent entries; document in PR |

### Constraints and Assumptions

- **Single-process assumption:** The orchestrator runs as a single process. Append-mode writes use the OS-level `O_APPEND` flag, which provides per-call atomicity, but no cross-call synchronization is used. If the system evolves to support concurrent writers, file locking would be needed.
- **Entry format includes trailing whitespace:** Both format strings end with `\n\n---\n\n`, ensuring proper separation between appended entries. No conditional newline logic is needed.

---

## Integration Points

### Existing Code Touchpoints

- `worklog.rs` — Replace 3 lines (read + prepend + rewrite) with 2 lines (open-append + write_all); add `use std::io::Write;`; update doc comment
- `backlog.rs` — Same mechanical replacement in `write_archive_worklog_entry()`; add `use std::io::Write;` if not already present
- `tests/worklog_test.rs` — Update `write_entry_prepends_newest_at_top` test: rename and reverse the ordering assertion to expect oldest-first
- `tests/backlog_test.rs` — Verify or add test for `write_archive_worklog_entry()` append ordering (PRD "Should Have")

### External Dependencies

None. Uses only `std::fs::OpenOptions` from the standard library.

---

## Open Questions

None. The implementation path is clear and all decisions are resolved.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Initial design draft | Straightforward mechanical replacement; light mode appropriate |
| 2026-02-13 | Self-critique (7 agents) | Fixed contradictory import rationale, added error mapping detail to data flow, added backlog_test.rs to integration points, documented single-process assumption and format whitespace guarantee, added operator guidance for transition month |
