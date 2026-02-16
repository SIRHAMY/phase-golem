# Tech Research: Add Inbox File for Adding New Backlog Items While Orchestrator Is Running

**ID:** WRK-026
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-026_PRD.md
**Mode:** Medium

## Overview

Researching how to add a `BACKLOG_INBOX.yaml` file mechanism to the orchestrator so users can add new backlog items while the orchestrator is running. Key questions: what file-based IPC patterns exist for this, how to handle concurrent reads/writes safely, how to integrate with the existing coordinator actor pattern, and what codebase machinery can be reused.

## Research Questions

- [x] What file-based IPC / inbox patterns exist for polling-based consumption?
- [x] How to safely read a file that may be written to concurrently (partial writes, atomic operations)?
- [x] What does the existing codebase provide for follow-up ingestion that we can reuse?
- [x] Where exactly in the scheduler loop should the inbox check be inserted?
- [x] Should we use `tokio::fs` or `std::fs` for the file operations?
- [x] What `InboxItem` struct shape best serves the use case?

---

## External Research

### Landscape Overview

The problem of adding items to a running system via a file-based mechanism is well-established in Unix systems engineering, with decades of precedent in mail delivery (Maildir), print spooling (CUPS), and configuration hot-reloading. The core challenge is consistent: one process writes a file while another reads it, requiring safe handling of partial reads, data loss, and race conditions.

For WRK-026, the problem is simpler than the general case because:
- Single reader (orchestrator) and single writer (human with text editor)
- Infrequent polling (once per scheduler loop, seconds to minutes apart)
- Very low write frequency (human adding items occasionally)
- Small file size (typically 1-10 items)

This means heavy-duty solutions (file locking, inotify watchers, directory-based queues) are overkill. The simpler end of the pattern spectrum is the right fit.

### Common Patterns & Approaches

#### Pattern: Drop File / Inbox File

**How it works:** A producer creates a file in a known location. A consumer polls for the file's existence, reads it, processes the contents, and deletes it. The file's presence is the signal; its absence means "nothing to do."

**When to use:** Low-frequency, low-volume message passing between loosely coupled processes. Ideal when the producer is a human or a simple script.

**Tradeoffs:**
- Pro: Extremely simple to implement and understand
- Pro: No dependencies beyond the filesystem
- Pro: Human-editable (YAML, JSON, plain text)
- Pro: Crash recovery is straightforward (file persists if consumer crashes before deletion)
- Con: Partial write risk if consumer reads while producer is mid-write
- Con: Single file means all-or-nothing processing

**References:**
- [Peer-to-peer IPC (Eric Raymond, TAOUP)](http://www.catb.org/~esr/writings/taoup/html/ch07s07.html) — Classic Unix IPC patterns
- [IPC Topics - OS course materials](https://os.cs.luc.edu/ipc.html) — Overview of IPC mechanisms

#### Pattern: Spool Directory

**How it works:** Messages are individual files in a directory. A daemon monitors the directory, processes each file, and removes it. Each file represents one unit of work.

**When to use:** When independent processing of multiple items is needed, ordering matters, or partial processing is important.

**Tradeoffs:**
- Pro: Each message is independent — one malformed message doesn't block others
- Pro: Natural ordering via filenames
- Pro: Atomic delivery via write-to-tmp-then-rename
- Con: More complex (directory scanning, ordering, cleanup)

**References:**
- [CUPS Design Description](https://www.cups.org/doc/spec-design.html) — Spool directory in production
- [Maildir specification (DJB)](https://cr.yp.to/proto/maildir.html) — Gold standard for lock-free file-based message delivery
- [Maildir - Wikipedia](https://en.wikipedia.org/wiki/Maildir) — Three-phase delivery pattern overview

#### Pattern: Atomic Write-to-Temp-Then-Rename

**How it works:** Write to a temporary file in the same directory, then `rename()` atomically to the target path. Readers always see either the old complete file or the new complete file, never a partial state.

**When to use:** Any time you need to ensure readers never see a partial file. This is the fundamental building block for safe file updates.

**Tradeoffs:**
- Pro: `rename()` is atomic on POSIX systems within the same filesystem
- Pro: Well-understood, battle-tested pattern
- Con: Requires temp file and target on the same filesystem

**References:**
- [A way to do atomic writes (LWN.net)](https://lwn.net/Articles/789600/) — Deep dive on Linux atomic file patterns
- [Things UNIX can do atomically (Richard Crowley)](https://rcrowley.org/2010/01/06/things-unix-can-do-atomically.html) — POSIX atomic operations guide
- [HN discussion on rename atomicity](https://news.ycombinator.com/item?id=11512006) — Practical discussion of guarantees

#### Pattern: TOCTOU-Safe File Reading

**How it works:** Instead of checking if a file exists then reading it, directly attempt to read and handle `NotFound` error.

**When to use:** Always, when checking for a file's presence before reading it. The PRD already specifies this.

**References:**
- [Rust Book: Recoverable Errors with Result](https://doc.rust-lang.org/book/ch09-02-recoverable-errors-with-result.html) — Idiomatic Rust error handling
- [std::fs::exists docs (notes TOCTOU)](https://doc.rust-lang.org/std/fs/fn.exists.html) — Rust std library warning

### Technologies & Tools

#### Rust Crates

| Technology | Purpose | Pros | Cons | Relevance |
|------------|---------|------|------|-----------|
| [tempfile](https://docs.rs/tempfile/latest/tempfile/struct.NamedTempFile.html) | Atomic file writes via `NamedTempFile::persist()` | Well-maintained, widely used | Already a dependency | **Already used** in `backlog::save()` |
| [serde_yaml_ng](https://crates.io/crates/serde_yaml_ng) | YAML parsing | Already in use, integrates with serde | — | **Already used** throughout codebase |
| [tokio::fs](https://docs.rs/tokio/latest/tokio/fs/index.html) | Async filesystem operations | Won't block tokio runtime, uses `spawn_blocking` | Slight overhead vs std::fs | **Recommended** for async context |
| [notify](https://docs.rs/notify/) | Filesystem event watching | Real-time events, cross-platform | Overkill, PRD rules it out | **Not recommended** |

#### Key Rust Std Library Components

- `std::fs::read_to_string()` / `tokio::fs::read_to_string()` — read file in one call
- `std::fs::remove_file()` / `tokio::fs::remove_file()` — delete file after processing
- `std::io::ErrorKind::NotFound` — discriminate "file doesn't exist" from other errors

### Standards & Best Practices

1. **Canonical Atomic File Update (POSIX):** open tmpfile → write → fsync → rename → fsync dir. For WRK-026, fsync is optional since crash durability of the inbox file itself is not critical.

2. **Try-Read Pattern (TOCTOU avoidance):** Always attempt the operation directly and handle errors, rather than checking preconditions first.

3. **Operation Ordering for Durability:** read inbox → validate → ingest into memory → persist to durable store → delete inbox. Never delete the source before the destination is confirmed durable.

4. **Separate Schema for Ingestion vs Storage:** Use a simplified input schema (inbox) that maps to the full internal schema (backlog item) for forward compatibility and clear validation boundaries.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| TOCTOU race: check existence then read | File can be deleted between check and read | Directly attempt `read_to_string()`, match on `NotFound` |
| Reading a partially written file | Editor mid-write produces invalid YAML | Accept parse failure, log warning, skip — file complete on next poll |
| Deleting inbox before backlog is saved | If save fails, items are lost | Order: read → ingest → save → delete. Only delete after confirmed save |
| Blocking tokio runtime with sync file I/O | `std::fs` blocks the executor thread | Use `tokio::fs` (uses `spawn_blocking` internally) |
| Deserializing directly into final types | Fails on missing optional fields or extra fields | Separate `InboxItem` struct with lenient serde config |

### Key Learnings

- The drop-file/inbox pattern is the simplest and most appropriate for this use case — single reader, single human writer, low frequency
- Partial write handling via parse-failure-and-retry is the standard approach when file locking is impractical (human editor)
- The existing codebase already uses the atomic write pattern (`tempfile::NamedTempFile::persist()`) for backlog saves — this is best practice
- `tokio::fs` should be used over `std::fs` in async context to avoid blocking the runtime
- The Maildir lesson applies: never delete the source until the destination is durably written

---

## Internal Research

### Existing Codebase State

The orchestrator is a Rust-based actor system with clear separation of concerns:

1. **Coordinator Actor** (`src/coordinator.rs`, 631 lines) — Tokio-based message-passing actor that serializes all backlog state mutations through `CoordinatorCommand` enum variants (12 variants). Uses `CoordinatorHandle` for async public interface with oneshot channel replies.

2. **Scheduler Loop** (`src/scheduler.rs`, 1331 lines) — Main event loop in `run_scheduler()` that polls coordinator snapshots, selects actions, spawns executor tasks, and processes results.

3. **Backlog Persistence** (`src/backlog.rs`, 316 lines) — Pure data transformations for backlog state. File I/O uses atomic write-temp-rename pattern via `tempfile` crate.

4. **Type System** (`src/types.rs`, 293 lines) — Core structs with serde serialization. `BacklogItem` has 20 fields, `FollowUp` has flexible deserialization.

**Relevant files/modules:**

- `src/coordinator.rs` — Actor loop, command processing, `handle_ingest_follow_ups()` (lines 415-425)
- `src/scheduler.rs` — Main loop (lines 462-704), integration point after line 501
- `src/backlog.rs` — `ingest_follow_ups()` (lines 243-284), `generate_next_id()` (lines 99-116), `save()` (lines 63-91)
- `src/types.rs` — `BacklogItem`, `FollowUp`, `BacklogFile`, `ItemStatus` definitions
- `src/main.rs` — Entry point, coordinator initialization (lines 240-381)
- `tests/backlog_test.rs` — Comprehensive tests (825 lines)

**Existing patterns in use:**
- Coordinator actor pattern with `CoordinatorCommand` enum and oneshot reply channels
- Atomic file writes via `NamedTempFile::persist()`
- YAML serialization with `serde_yaml_ng` and `#[serde(default, skip_serializing_if)]`
- Logging macros: `log_info!`, `log_warn!`, `log_debug!`, `log_error!`
- Error handling: `Result<T, String>` with `map_err()` for context

### Reusable Components

1. **`backlog::generate_next_id(backlog, prefix)`** — Thread-safe ID generation using high-water mark. Returns `(String, u32)`. Reusable as-is for inbox items.

2. **`backlog::ingest_follow_ups(backlog, follow_ups, origin, prefix)`** — Creates items from `FollowUp` structs with status `New`, sets origin. Returns created item IDs. **This is the model function** — inbox ingestion follows the same pattern.

3. **`backlog::save(path, backlog)`** — Atomic backlog persistence. Already proven in tests.

4. **`CoordinatorHandle::ingest_follow_ups()`** — Async method that sends through actor channel. The inbox can either reuse this (converting `InboxItem` → `FollowUp`) or add a parallel `ingest_inbox_items()` method.

5. **`CoordinatorState` fields** — Already holds `project_root: PathBuf`, `backlog_path: PathBuf`, `prefix: String`. Can construct inbox path as `project_root.join("BACKLOG_INBOX.yaml")`.

6. **YAML deserialization patterns** — `#[serde(default)]` for optional fields, `skip_serializing_if = "Option::is_none"` for clean output. `FollowUp` struct shows flexible deserialization pattern.

### Constraints from Existing Code

1. **All backlog mutations through coordinator actor** — Must use `CoordinatorCommand`, not direct state modification. Thread safety guaranteed by actor serialization.

2. **ID generation in coordinator** — `generate_next_id()` reads `backlog.next_item_id` from coordinator state. Cannot pre-generate IDs outside the actor.

3. **Status transitions** — Inbox items must enter as `ItemStatus::New`. Only triage can promote them. Invalid transitions rejected by `transition_status()`.

4. **Schema version** — `BacklogFile` currently at `schema_version: 2`. Inbox items inherit this on ingestion.

5. **Origin field convention** — Follow-ups use `"WRK-001/prd"` format. Inbox items should use `"inbox"` for traceability.

6. **`Result<T, String>` error type** — The codebase uses `String` errors, not typed error enums. New code should follow this convention.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| "Reuse or extend `ingest_follow_ups`" (PRD Assumption 3) | The `FollowUp` struct has `title`, `context`, `suggested_size`, `suggested_risk` — closely matching inbox needs but missing `description`, `impact`, `pipeline_type`, `dependencies` | Either extend `FollowUp` with new optional fields (disruptive), convert `InboxItem` → `FollowUp` (loses some fields), or create a parallel ingestion path. **Recommendation: parallel path** — an `InboxItem` struct and `ingest_inbox_items()` function are cleaner |
| Direct read avoids TOCTOU (PRD must-have #1) | Confirmed best practice — `tokio::fs::read_to_string()` with `NotFound` match | PRD is correct. Use `tokio::fs` not `std::fs` to avoid blocking runtime |
| "Inbox items have `origin` set to `"inbox"`" (PRD must-have #5) | Internal research confirms origin field is `Option<String>` and already used for follow-ups with format `"WRK-XXX/phase"` | Simple to implement — just set `origin: Some("inbox".to_string())` |
| "Scheduler loop integration: check for inbox at top of each iteration" (PRD scope) | Scheduler loop at lines 462-704. After `get_snapshot()` (line 501), before `select_actions()` (line 517) is the right insertion point | Inbox check should be placed between snapshot and action selection. If items are ingested, the snapshot is already stale — but since new items have status `New`, they won't be selected by `select_actions()` until triage promotes them. **No re-snapshot needed.** |
| "Should the orchestrator commit inbox-ingested items to git immediately?" (Open question #1) | Backlog is committed on phase completion via existing patterns. `save()` writes to disk but doesn't commit to git | Deferring to next phase completion is simpler and consistent. WRK-032 (commit on halt) provides safety net |

---

## Critical Areas

### Correct Operation Ordering

**Why it's critical:** The sequence read → ingest → save → delete is essential for data safety. If any step is misordered, items can be lost (inbox deleted before save) or duplicated (saved but not deleted, crash between).

**Why it's easy to miss:** The implementation might naturally put the delete right after the read for "cleanup", or an error path might skip saving before deleting.

**What to watch for:** Ensure the delete is gated on save success. If save fails, the inbox file must remain. Test the error paths explicitly.

### tokio::fs vs std::fs in Async Context

**Why it's critical:** Using `std::fs::read_to_string()` in an async function blocks the tokio executor thread. For a small inbox file this is brief, but it violates async safety principles.

**Why it's easy to miss:** `std::fs` compiles and works, and the blocking window is tiny for small files. Easy to not think about it.

**What to watch for:** Use `tokio::fs::read_to_string()` and `tokio::fs::remove_file()` throughout. The coordinator handler runs inside the actor task, which is async — synchronous I/O is acceptable there since the actor is already a single sequential task, but `tokio::fs` is still cleaner.

### InboxItem Struct Design

**Why it's critical:** The struct must be lenient enough for forward compatibility (unknown fields ignored) while validating required fields (title non-empty). Getting the serde attributes wrong breaks user experience.

**Why it's easy to miss:** Defaulting to strict deserialization (`deny_unknown_fields`) would break forward compatibility. Missing the title validation means empty items enter the backlog.

**What to watch for:** Use `#[serde(default)]` on optional fields, do NOT use `#[serde(deny_unknown_fields)]`. Validate title is non-empty after deserialization, not during.

### Reuse vs. Parallel Path Decision

**Why it's critical:** The PRD assumes reusing `ingest_follow_ups`, but the `FollowUp` struct doesn't carry all the optional fields inbox items need (`description`, `impact`, `pipeline_type`, `dependencies`).

**Why it's easy to miss:** The structures look similar at first glance, but the field sets diverge enough that forcing inbox items through the follow-up path loses data.

**What to watch for:** Create a dedicated `InboxItem` struct and `ingest_inbox_items()` function. The code pattern is nearly identical to `ingest_follow_ups()` but works with a different input type. This is cleaner than extending `FollowUp` which would affect all existing follow-up callers.

---

## Deep Dives

### tokio::fs Behavior in Actor Context

**Question:** Should the coordinator actor handler use `tokio::fs` or `std::fs`? The actor is already a single sequential task processing one command at a time.

**Summary:** The coordinator actor runs as a single tokio task that processes commands sequentially. Using `std::fs` inside it would block the tokio thread pool for the duration of the file read. While the file is small and the blocking window is tiny, the existing codebase uses `std::fs` in `backlog::save()` and `backlog::load()` — these are called from within the coordinator handler. So the existing pattern is synchronous I/O within the actor. Following this existing pattern is more consistent than switching to `tokio::fs` for inbox operations only.

**Implications:** Use `std::fs` for consistency with existing `backlog::load()` and `backlog::save()` patterns. The actor already does synchronous I/O for backlog operations. If async I/O is desired later, it should be a separate refactor across all file operations.

### Inbox File Format

**Question:** Should the inbox be a bare YAML list (`- title: "..."`) or a structured document with metadata?

**Summary:** The PRD specifies a simplified structure requiring only `title`. A bare YAML list is the simplest format for humans to write:

```yaml
- title: "Fix the login bug"
  description: "Users can't log in with special characters"

- title: "Add dark mode"
```

A structured document with schema version would add friction for human writers without clear benefit. The inbox is ephemeral — it's consumed and deleted. No migration needed.

**Implications:** Use a bare `Vec<InboxItem>` deserialization. No wrapper struct, no schema version. Keep it as simple as possible for the human writer.

---

## Synthesis

### Open Questions

| Question | Why It Matters | Resolution |
|----------|----------------|------------|
| Should inbox items be committed to git immediately? | Durability vs git noise tradeoff | **Resolved: Defer.** Consistent with existing pattern (backlog committed on phase completion). WRK-032 provides safety on halt. |
| Should there be an inbox size limit? | Prevent accidental bulk ingestion | **Resolved: No hard limit.** Log a warning for >50 items (nice-to-have per PRD). Expected usage is a few items at a time. |
| Should we reuse `ingest_follow_ups` or create a parallel path? | Code reuse vs data completeness | **Resolved: Parallel path.** `InboxItem` has different optional fields than `FollowUp`. New `ingest_inbox_items()` function following the same pattern. |
| `tokio::fs` vs `std::fs`? | Async safety vs consistency | **Resolved: Use `std::fs`.** Consistent with existing `backlog::load()` and `backlog::save()` patterns. Actor is already sequential. |
| Re-snapshot after inbox ingestion? | New items visible to scheduler | **Resolved: Not needed.** Inbox items enter as `New` and won't be selected by `select_actions()` until triage promotes them. Next loop iteration picks them up naturally. |

### Recommended Approaches

#### Inbox File Reading

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Direct `read_to_string` + `NotFound` match | Simple, TOCTOU-safe, no dependencies | Small partial-write window | **Always — this is the right approach** |
| File watcher (inotify/notify) | Real-time detection | Overkill, complexity, PRD rules it out | High-frequency, latency-sensitive use cases |

**Initial recommendation:** Direct read with `NotFound` match. This is what the PRD specifies and aligns with all research findings.

#### Ingestion Pathway

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Extend `FollowUp` struct with inbox fields | Code reuse, single code path | Modifies existing type, loses semantic clarity | Input types are nearly identical |
| Convert `InboxItem` → `FollowUp` | Reuses existing handler | Loses optional fields (`description`, `impact`, etc.) | Inbox items only need `title` + `context` |
| New `InboxItem` type + `ingest_inbox_items()` | Clean separation, preserves all fields, clear semantics | Some code duplication with `ingest_follow_ups()` | **Recommended** — input types have different field sets |

**Initial recommendation:** New parallel path. The code duplication is minimal (the function body is ~20 lines), and it maintains clean type semantics. `InboxItem` carries `title`, `description`, `size`, `risk`, `impact`, `pipeline_type`, `dependencies` — a different set from `FollowUp`'s `title`, `context`, `suggested_size`, `suggested_risk`.

#### Coordinator Integration

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| New `CoordinatorCommand::IngestInboxItems` variant | Explicit, clear separation of concerns | One more command variant | **Recommended** — semantically distinct operation |
| Reuse `IngestFollowUps` with `origin: "inbox"` | Less code change | Conflates two different operations, different return semantics | Operations are truly identical |

**Initial recommendation:** New command variant. The inbox ingestion includes file I/O (read + delete) which is fundamentally different from follow-up ingestion that receives items in-memory. The coordinator handler needs to do the file read, parse, validate, ingest, save, and delete — more than what `handle_ingest_follow_ups` does.

#### File Deletion Handling

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `remove_file` + ignore `NotFound` error | Simple, handles race with user deletion | — | **Always** |
| `remove_file` + error on any failure | Catches permission issues | Overly noisy for races | Only if delete failures are critical |

**Initial recommendation:** Delete with `NotFound` suppressed. If the file was already removed by the user between read and delete, that's fine — the items were ingested.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Maildir spec (DJB)](https://cr.yp.to/proto/maildir.html) | Specification | Gold standard for lock-free file-based message delivery; operation ordering principles |
| [LWN: Atomic writes](https://lwn.net/Articles/789600/) | Article | Deep dive into atomic file patterns on Linux |
| [Rust Book: Recoverable Errors](https://doc.rust-lang.org/book/ch09-02-recoverable-errors-with-result.html) | Documentation | Idiomatic Rust error handling for file operations |
| [tokio::fs module](https://docs.rs/tokio/latest/tokio/fs/index.html) | Documentation | Async file operations and spawn_blocking behavior |
| [Things UNIX can do atomically](https://rcrowley.org/2010/01/06/things-unix-can-do-atomically.html) | Article | Practical guide to POSIX atomic operations |
| [Serde error handling](https://serde.rs/error-handling.html) | Documentation | Handling deserialization errors for malformed inbox YAML |
| [Runtime Config Reloading (Vorner)](https://vorner.github.io/2019/08/11/runtime-configuration-reloading.html) | Article | Rust patterns for hot-reloading (context, not directly applicable) |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | External research: file-based IPC patterns, atomic writes, concurrent file access | Confirmed drop-file pattern is appropriate; documented 4 key patterns and 5 common pitfalls |
| 2026-02-12 | Internal research: coordinator actor, scheduler loop, backlog types, follow-up ingestion | Mapped exact integration points; identified `ingest_follow_ups` as model function; found `FollowUp` struct mismatch |
| 2026-02-12 | PRD analysis and synthesis | Resolved 5 open questions; recommended parallel ingestion path over follow-up reuse |

## Assumptions

Decisions made without human input during autonomous tech research:

1. **Parallel ingestion path over FollowUp reuse:** Chose to recommend a new `InboxItem` struct and `ingest_inbox_items()` function rather than extending `FollowUp`, because the optional field sets diverge (`description`, `impact`, `pipeline_type`, `dependencies` vs `context`, `suggested_size`, `suggested_risk`). The PRD assumed reuse (Assumption 3), but research shows the types are different enough to warrant separation.

2. **`std::fs` over `tokio::fs`:** Chose to recommend synchronous file I/O for consistency with existing `backlog::load()` and `backlog::save()` patterns. The coordinator actor is already a sequential task doing synchronous I/O. Switching to `tokio::fs` for inbox only would be inconsistent.

3. **No re-snapshot needed:** Determined that inbox items enter as `New` status and won't be selected by `select_actions()` until triage promotes them. The stale snapshot after ingestion is not a problem — items appear on the next loop iteration naturally.

4. **Deferred git commit:** Resolved the PRD's open question about git commit timing in favor of deferring to next phase completion. This is consistent with existing behavior and WRK-032 provides a safety net for halt scenarios.

5. **No inbox size limit:** Resolved the PRD's open question about maximum item count in favor of no hard limit, with a nice-to-have warning for >50 items. Expected usage is a handful of items.
