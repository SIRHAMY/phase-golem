# Tech Research: Switch Worklog to Append-Only Writes

**ID:** WRK-023
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-023_switch-worklog-to-append-only_PRD.md
**Mode:** Light

## Overview

Research how to convert the orchestrator's worklog from prepend-and-rewrite to append-only writes. The key question is whether Rust's `std::fs::OpenOptions::append()` can directly replace the current `read_to_string` + `format!("{}{}", new, old)` + `fs::write()` cycle while preserving the existing function signatures, error handling, and entry formatting.

## Research Questions

- [x] What is the correct Rust API for append-only file writes?
- [x] Are there any gotchas with `OpenOptions::append()` + `.create(true)`?
- [x] What does the current implementation look like and what exactly needs to change?

---

## External Research

### Landscape Overview

Append-only file writing in Rust is well-supported through `std::fs::OpenOptions`. The pattern uses `.append(true).create(true).open(path)` to get a file handle that always writes at the end of the file. This is cross-platform consistent — Rust abstracts the underlying `O_APPEND` (Unix) and Windows append flags.

### Common Patterns & Approaches

#### Pattern: OpenOptions Append

**How it works:** Open file with `OpenOptions::new().append(true).create(true)`, write with `write!` or `write_all`.

**When to use:** Log-like data where entries are immutable once written.

**Tradeoffs:**
- Pro: O(1) per write regardless of file size
- Pro: No read required — eliminates `fs::read_to_string()`
- Pro: Cross-platform consistent
- Con: No atomicity guarantee across multiple `write!` calls (not relevant for single-process orchestrator)

**References:**
- [OpenOptions in std::fs - Rust](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html) — official docs
- [Append to a file](https://rust.code-maven.com/append-to-a-file) — practical tutorial

### Standards & Best Practices

- Always pair `.append(true)` with `.create(true)` — append alone won't create the file
- Use `write_all()` instead of `write()` to avoid partial writes
- Import `std::io::Write` trait for `write!`/`writeln!` macros
- Format the entire entry as a single string before writing to minimize system calls

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Missing `.create(true)` | File open fails if file doesn't exist | Always use `.append(true).create(true)` together |
| Missing `use std::io::Write` | Compiler error on `write!` macro | Include the import |
| Multiple `write!` calls per entry | Interleaving risk in multi-process scenarios | Format full entry first, write once (not a concern here — single process) |
| Not creating parent dirs | `OpenOptions` doesn't create directories | Keep existing `create_dir_all()` call |

### Key Learnings

- The migration is mechanically simple: replace 3 lines (read + format + write) with 2 lines (open + write_all)
- No external dependencies needed — `std::fs::OpenOptions` is sufficient
- The approach is well-established and requires no novel patterns

---

## Internal Research

### Existing Codebase State

Two functions implement the prepend-and-rewrite pattern:

**`write_entry()` in `worklog.rs` (lines 21-49):**
```rust
pub fn write_entry(
    worklog_dir: &Path,
    item: &BacklogItem,
    phase: &str,
    outcome: &str,
    result_summary: &str,
) -> Result<(), String>
```
- Takes directory path, constructs `YYYY-MM.md` filename internally
- Creates directory with `fs::create_dir_all(worklog_dir)`
- Formats entry, reads existing file, prepends, rewrites entire file
- Called from `coordinator.rs` (`handle_write_worklog`) and indirectly from 6 scheduler sites

**`write_archive_worklog_entry()` in `backlog.rs` (lines 421-447):**
```rust
fn write_archive_worklog_entry(worklog_path: &Path, item: &BacklogItem) -> Result<(), String>
```
- Takes full file path (different from `write_entry` which takes directory)
- Creates parent directory with `fs::create_dir_all(parent)`
- Same prepend-and-rewrite pattern
- Private function, called only from `archive_item()`

**Relevant files/modules:**
- `worklog.rs` — target function #1 (49 lines total)
- `backlog.rs` — target function #2 (lines 421-447)
- `tests/worklog_test.rs` — 4 tests including ordering assertion
- `coordinator.rs` — caller (`handle_write_worklog` at line 408)
- `scheduler.rs` — 6 call sites triggering worklog writes

### Existing Patterns

- **Error handling:** Both functions use `Result<(), String>` with `.map_err(|e| format!(...))` — this pattern must be preserved
- **Directory creation:** Both use `fs::create_dir_all()` before file operations — this stays
- **Entry formatting:** Both use `format!()` to build the full entry string — this stays

### Reusable Components

- Entry format strings are already correct and remain unchanged
- `fs::create_dir_all()` calls remain as-is
- Error mapping pattern reusable for the new `OpenOptions` call

### Constraints

- `write_entry()` signature is public API — must not change
- `write_archive_worklog_entry()` is private — signature can change if needed (but no reason to)
- Return type `Result<(), String>` must be preserved
- Entry format (markdown headers, fields, separators) must be preserved

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| No concerns — PRD is well-aligned with implementation reality | Both functions use the exact pattern described (read + prepend + rewrite) | Straightforward mechanical replacement |

The PRD accurately describes the current code. No conflicts or complications found.

---

## Critical Areas

### Correct `use` Import

**Why it's critical:** Using `write!` or `writeln!` on a `File` requires `use std::io::Write` in scope.

**Why it's easy to miss:** The current code uses `fs::write()` (a free function) which doesn't need the `Write` trait import. Switching to `OpenOptions` + `write_all()` or `write!()` requires adding this import.

**What to watch for:** Ensure `std::io::Write` is imported in `worklog.rs`. In `backlog.rs`, check if it's already imported for other uses.

---

## Deep Dives

No deep dives needed — this is a well-understood pattern with clear implementation path.

---

## Synthesis

### Open Questions

None. The implementation path is clear.

### Recommended Approaches

#### Write Mechanism

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `OpenOptions::append(true).create(true)` + `file.write_all(entry.as_bytes())` | Simple, one system call, no extra trait needed if using `write_all` | Slightly more verbose than `fs::write` | Single entry per open (our case) |
| `OpenOptions::append(true).create(true)` + `write!(&mut file, ...)` | Familiar macro syntax | Requires `use std::io::Write` | Multiple formatted writes per open |

**Initial recommendation:** Use `OpenOptions::append(true).create(true).open(path)` followed by `file.write_all(entry.as_bytes())`. This avoids needing the `Write` trait import for `write!` and is the simplest mechanical change. The entry is already formatted as a complete string.

#### Implementation Change Summary

For both functions, replace these 3 lines:
```rust
let existing = fs::read_to_string(&path).unwrap_or_default();
let contents = format!("{}{}", entry, existing);
fs::write(&path, contents).map_err(...)?;
```

With these 2 lines:
```rust
let mut file = fs::OpenOptions::new()
    .append(true)
    .create(true)
    .open(&path)
    .map_err(...)?;
file.write_all(entry.as_bytes())
    .map_err(...)?;
```

Plus add `use std::io::Write;` at the top of each file (needed for `write_all` on `File`).

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [OpenOptions docs](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html) | Official docs | API reference for append mode |
| [Append to a file](https://rust.code-maven.com/append-to-a-file) | Tutorial | Practical example of the exact pattern |
| [OpenOptions append PR](https://github.com/rust-lang/rust/pull/120781/files) | GitHub PR | Clarifies atomicity behavior |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Light external + internal research | Confirmed straightforward `OpenOptions::append` approach; no surprises in codebase |
