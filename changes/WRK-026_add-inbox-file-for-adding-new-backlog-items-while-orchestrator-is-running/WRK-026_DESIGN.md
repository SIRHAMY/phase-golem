# Design: Add Inbox File for Adding New Backlog Items While Orchestrator Is Running

**ID:** WRK-026
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-026_PRD.md
**Tech Research:** ./WRK-026_TECH_RESEARCH.md
**Mode:** Medium

## Overview

The inbox feature adds a drop-file mechanism (`BACKLOG_INBOX.yaml`) that allows humans to add new work items to a running orchestrator. The orchestrator polls for this file at the top of each scheduler loop iteration, parses it as a YAML list of simplified items, ingests valid items into the backlog via a new coordinator command, persists the updated backlog, and deletes the inbox file. This follows the well-established drop-file/inbox pattern, reuses existing coordinator actor and backlog persistence machinery, and introduces a parallel `InboxItem` type to cleanly separate inbox input from follow-up input.

---

## System Design

### High-Level Architecture

```
Human writes                    Orchestrator reads
BACKLOG_INBOX.yaml ──────────> Scheduler Loop
                                    │
                                    ▼
                              Coordinator Actor
                                    │
                              ┌─────┴─────┐
                              │ backlog.rs │
                              │            │
                              │ parse      │
                              │ validate   │
                              │ create     │
                              │ items      │
                              └─────┬─────┘
                                    │
                                    ▼
                              BACKLOG.yaml (save)
                                    │
                                    ▼
                              Delete BACKLOG_INBOX.yaml
```

The inbox is a one-way channel: humans write, orchestrator reads and consumes. There are no competing writes to `BACKLOG.yaml` (only the coordinator writes it) and no merge logic needed.

### Component Breakdown

#### InboxItem Type (`src/types.rs`)

**Purpose:** Defines the simplified input schema for inbox items.

**Responsibilities:**
- Deserialize YAML inbox entries with lenient field handling
- Carry required (`title`) and optional (`description`, `size`, `risk`, `impact`, `pipeline_type`, `dependencies`) fields

**Interfaces:**
- Input: YAML list entries from `BACKLOG_INBOX.yaml`
- Output: Populated `InboxItem` structs for ingestion

**Dependencies:** `serde`, `serde_yaml_ng`

**Design:**
```rust
#[derive(Debug, Clone, Deserialize)]
pub struct InboxItem {
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub size: Option<SizeLevel>,
    #[serde(default)]
    pub risk: Option<DimensionLevel>,
    #[serde(default)]
    pub impact: Option<DimensionLevel>,
    #[serde(default)]
    pub pipeline_type: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}
```

Key design choices:
- No `#[serde(deny_unknown_fields)]` — unknown fields (including `id`) are silently ignored for forward compatibility
- `title` is required by serde (not `Option`), but validated post-deserialization for non-empty/non-whitespace via `title.trim().is_empty()` check
- Uses existing `SizeLevel` and `DimensionLevel` types for optional assessments
- `dependencies` defaults to empty vec via `#[serde(default)]`
- Invalid enum values in optional fields (e.g., `size: "mega"`) cause a parse error for the entire file — this is acceptable since the human can fix and retry on next iteration

#### Inbox Loading and Ingestion (`src/backlog.rs`)

**Purpose:** Pure functions for reading the inbox file, validating items, and creating backlog entries.

**Responsibilities:**
- Read and parse `BACKLOG_INBOX.yaml` as `Vec<InboxItem>`
- Validate individual items (non-empty title)
- Convert valid `InboxItem`s to `BacklogItem`s with generated IDs, status `New`, and `origin: "inbox"`
- Delete inbox file after successful ingestion and save

**Interfaces:**
- Input: `inbox_path: &Path`
- Output: `Result<Vec<InboxItem>, String>` for loading; `Vec<BacklogItem>` for ingestion

**Dependencies:** `std::fs`, `serde_yaml_ng`, existing `generate_next_id()`

**Functions:**

```rust
/// Attempts to read and parse the inbox file.
/// Directly calls fs::read_to_string() without prior existence check (TOCTOU-safe).
/// Returns Ok(None) if ErrorKind::NotFound — file doesn't exist (normal case).
/// Returns Ok(Some(vec)) if parsed successfully (may be empty vec for empty file).
/// Returns Err(message) if the file exists but can't be parsed (malformed YAML)
///   or if a non-NotFound I/O error occurs (e.g., permission denied).
pub fn load_inbox(inbox_path: &Path) -> Result<Option<Vec<InboxItem>>, String>

/// Converts valid InboxItems into BacklogItems, appending to backlog.
/// Validates each item: skips items where title.trim().is_empty() (logs warning).
/// Skipped items do NOT consume an ID — IDs are only generated for valid items.
/// Returns only the successfully created BacklogItems.
pub fn ingest_inbox_items(
    backlog: &mut BacklogFile,
    items: &[InboxItem],
    prefix: &str,
) -> Vec<BacklogItem>

/// Deletes the inbox file.
/// Ignores NotFound errors (file already gone — not a problem).
/// Logs warning for other errors (e.g., permission denied) but returns Ok
/// since items are already persisted to BACKLOG.yaml.
pub fn clear_inbox(inbox_path: &Path) -> Result<(), String>
```

**InboxItem → BacklogItem field mapping:**

| InboxItem field | BacklogItem field | Notes |
|-----------------|-------------------|-------|
| `title` | `title` | Required, validated non-empty |
| `description` | `description` | Optional, passed through |
| `size` | `size` | Optional, may be overridden by triage |
| `risk` | `risk` | Optional, may be overridden by triage |
| `impact` | `impact` | Optional, may be overridden by triage |
| `pipeline_type` | `pipeline_type` | Optional, passed through |
| `dependencies` | `dependencies` | Optional, defaults to empty vec |
| _(generated)_ | `id` | Via `generate_next_id()` |
| _(hardcoded)_ | `status` | Always `ItemStatus::New` |
| _(hardcoded)_ | `origin` | Always `Some("inbox".to_string())` |
| _(generated)_ | `created` | `chrono::Utc::now().to_rfc3339()` |
| _(generated)_ | `updated` | Same as `created` |
| _(default)_ | All other fields | `None`, `false`, `vec![]`, etc. |

#### Coordinator Command (`src/coordinator.rs`)

**Purpose:** Extends the coordinator actor with an inbox ingestion command.

**Responsibilities:**
- Receive `IngestInbox` command from scheduler
- Call `backlog::load_inbox()` to read and parse the file
- Call `backlog::ingest_inbox_items()` to create backlog entries
- Call `state.save_backlog()` to persist
- Call `backlog::clear_inbox()` to delete the inbox file
- Return the list of new item IDs (or empty vec if no file / no items)

**Interfaces:**
- Command: `CoordinatorCommand::IngestInbox { reply: oneshot::Sender<Result<Vec<String>, String>> }`
- Handle method: `CoordinatorHandle::ingest_inbox() -> Result<Vec<String>, String>`

**Dependencies:** `CoordinatorState` (which holds `inbox_path: PathBuf`), `backlog` module

**Error handling in handler:**
- `load_inbox()` returns `Err` → log warning, reply `Ok(vec![])` (file left in place)
- `load_inbox()` returns `Ok(None)` → reply `Ok(vec![])`
- `load_inbox()` returns `Ok(Some(vec![]))` → delete file, reply `Ok(vec![])`
- `ingest_inbox_items()` returns items → save backlog
- `save_backlog()` returns `Err` → log error, reply `Err(msg)` — `clear_inbox()` NOT called
- `save_backlog()` returns `Ok` → call `clear_inbox()`, reply `Ok(new_ids)`
- `clear_inbox()` returns `Err` → log warning but still reply `Ok(new_ids)` (items are saved)

#### Scheduler Integration (`src/scheduler.rs`)

**Purpose:** Triggers inbox ingestion at the top of each scheduler loop iteration.

**Responsibilities:**
- Call `coordinator.ingest_inbox()` after snapshot retrieval, before action selection
- Log info message on successful ingestion with count and IDs
- Log warning on ingestion failure, continue execution
- No re-snapshot needed (new items are status `New`, not selectable until triage promotes them)

**Dependencies:** `CoordinatorHandle`

#### Startup Configuration (`src/main.rs`)

**Purpose:** Pass the inbox file path to the coordinator on initialization.

**Responsibilities:**
- Construct inbox path as `project_root.join("BACKLOG_INBOX.yaml")`
- Pass to `CoordinatorState` initialization as `inbox_path: PathBuf` field (alongside existing `backlog_path: PathBuf`)

**Dependencies:** `CoordinatorState`

**Note:** The inbox file is deliberately NOT protected by the orchestrator's fslock. The fslock prevents multiple orchestrator instances from running simultaneously, but humans must be able to write to the inbox while the orchestrator holds the lock on `BACKLOG.yaml`.

### Data Flow

1. **Human writes** `BACKLOG_INBOX.yaml` in the project root with one or more items as a YAML list
2. **Scheduler loop** begins a new iteration and calls `coordinator.ingest_inbox()`
3. **Coordinator actor** receives the `IngestInbox` command
4. **Coordinator handler** calls `backlog::load_inbox(state.inbox_path)`
   - If file doesn't exist → returns `Ok(None)` → reply with `Ok(vec![])` → done
   - If file exists but can't be parsed → returns `Err(msg)` → log warning, reply with `Ok(vec![])` → done (file left in place for human to fix; this also handles partial writes from concurrent human editing)
   - If file parses to empty list → delete file, reply with `Ok(vec![])` → done
   - If file parses successfully with items → continue
5. **Coordinator handler** calls `backlog::ingest_inbox_items(backlog, items, prefix)` to create `BacklogItem`s in memory and append to `backlog.items`
6. **Coordinator handler** calls `state.save_backlog()` to persist to `BACKLOG.yaml`
   - If save fails → log error, reply with `Err(msg)`. Inbox file is NOT deleted. Items remain in memory but will be re-ingested from file on next iteration.
7. **Coordinator handler** calls `backlog::clear_inbox(state.inbox_path)` to delete the inbox file (only reached if save succeeded)
8. **Coordinator handler** replies with `Ok(new_item_ids)`
9. **Scheduler** logs the ingestion result and continues to action selection

### Key Flows

#### Flow: Successful Inbox Ingestion

> Human adds items to inbox, orchestrator ingests them into the backlog.

1. **Human creates file** — Writes `BACKLOG_INBOX.yaml` with one or more items
2. **Scheduler polls** — At top of loop iteration, calls `coordinator.ingest_inbox()`
3. **Read inbox** — `load_inbox()` reads file via `std::fs::read_to_string()`, parses YAML
4. **Validate items** — `ingest_inbox_items()` checks each item's title is non-empty, skips invalid items with warning
5. **Create backlog entries** — For each valid item, generates ID via `generate_next_id()`, creates `BacklogItem` with status `New` and `origin: "inbox"`
6. **Save backlog** — `save_backlog()` atomically persists updated `BACKLOG.yaml`
7. **Clear inbox** — `clear_inbox()` deletes `BACKLOG_INBOX.yaml`
8. **Log result** — Scheduler logs: `"Ingested 3 items from inbox: WRK-043, WRK-044, WRK-045"`

**Edge cases:**
- Empty inbox file (valid YAML, zero items) — deleted silently, no log
- Items with whitespace-only titles — skipped with warning, valid items still ingested
- Mixed valid/invalid items — valid items ingested, invalid items logged and skipped

#### Flow: Inbox File Does Not Exist

> Normal case when no items are pending — the common path.

1. **Scheduler polls** — Calls `coordinator.ingest_inbox()`
2. **Read attempt** — `load_inbox()` calls `read_to_string()`, gets `ErrorKind::NotFound`
3. **Return immediately** — Returns `Ok(None)`, no log output
4. **Scheduler continues** — Proceeds to action selection with no delay

#### Flow: Malformed Inbox File

> Human writes invalid YAML, orchestrator skips and leaves file for fixing.

1. **Scheduler polls** — Calls `coordinator.ingest_inbox()`
2. **Read succeeds** — File contents are read successfully
3. **Parse fails** — `serde_yaml_ng::from_str()` returns a deserialization error
4. **Log warning** — `"Failed to parse BACKLOG_INBOX.yaml: {parse_error}. File left in place for manual correction."`
5. **File preserved** — Inbox file is not deleted, human can fix and retry
6. **Scheduler continues** — Proceeds normally, will retry next iteration

#### Flow: Crash Between Save and Delete

> Orchestrator crashes after saving backlog but before deleting inbox.

1. **Items ingested and saved** — Backlog on disk contains the new items
2. **Crash before delete** — Inbox file still exists on disk
3. **Restart** — Orchestrator starts, loads backlog (which has the items)
4. **Next inbox poll** — Reads inbox again, creates duplicate items with new IDs
5. **Duplicates exist** — Same titles, different IDs, both with `origin: "inbox"`
6. **Triage handles** — During triage phase, duplicates are detectable by matching title + origin

**Note:** This is an accepted tradeoff. The crash window is very small (between save and delete), and duplicates are benign — triage can detect and handle them.

#### Flow: Backlog Save Failure

> Disk full or permission error prevents backlog save.

1. **Items parsed and ingested into memory** — Valid items created in memory
2. **Save fails** — `save_backlog()` returns error
3. **Inbox preserved** — `clear_inbox()` is NOT called (save error short-circuits)
4. **Warning logged** — `"Failed to save backlog after inbox ingestion: {error}"`
5. **Items lost from memory** — In-memory items exist until next command mutates state
6. **Next iteration** — Inbox is read again, items re-ingested (idempotent since previous save failed)

---

## Technical Decisions

### Key Decisions

#### Decision: Parallel InboxItem Type (Not Reusing FollowUp)

**Context:** The PRD initially assumed reusing the existing `ingest_follow_ups()` pathway. Tech research revealed that `FollowUp` has fields (`context`, `suggested_size`, `suggested_risk`) that don't map cleanly to inbox needs (`description`, `impact`, `pipeline_type`, `dependencies`).

**Decision:** Create a separate `InboxItem` struct and `ingest_inbox_items()` function.

**Rationale:**
- Clean type separation — inbox and follow-up are semantically different inputs
- No loss of optional fields — inbox items carry `description`, `impact`, `pipeline_type`, `dependencies` that `FollowUp` doesn't have
- Minimal code duplication — `ingest_inbox_items()` is ~25 lines, same pattern as `ingest_follow_ups()`
- No impact on existing follow-up callers — no changes to `FollowUp` struct

**Consequences:** Two similar-looking ingestion functions exist. This is acceptable because the input types and field mappings differ.

#### Decision: New CoordinatorCommand Variant (Not Reusing IngestFollowUps)

**Context:** Could either add a new `IngestInbox` command or route inbox items through the existing `IngestFollowUps` command.

**Decision:** Add `CoordinatorCommand::IngestInbox` as a new variant.

**Rationale:**
- Inbox ingestion includes file I/O (read, parse, delete) that follow-up ingestion doesn't
- The coordinator handler needs to orchestrate read → validate → ingest → save → delete, which is a different workflow than follow-up ingestion (which receives items in-memory)
- Clear separation of concerns — each command does one thing

**Consequences:** One more command variant in the enum. The actor loop gets one more match arm.

#### Decision: Use std::fs (Not tokio::fs)

**Context:** The coordinator actor runs async tasks. Using `std::fs` blocks the tokio executor thread.

**Decision:** Use `std::fs` for inbox file operations.

**Rationale:**
- Consistent with existing patterns — `backlog::load()` and `backlog::save()` both use `std::fs`
- The coordinator actor processes commands sequentially — it's already doing synchronous I/O for backlog operations
- The inbox file is small (1-10 items typically) — blocking window is negligible
- Switching to `tokio::fs` for inbox only would be inconsistent; if async I/O is desired, it should be a separate refactor across all file operations

**Consequences:** Blocks the tokio thread for the duration of the file read. For the expected file sizes (1-10 items, ~1KB), this is low single-digit milliseconds on local disk. This continues the existing blocking I/O pattern — if this becomes a bottleneck, all backlog I/O should be migrated to `tokio::fs` together as a separate refactor.

#### Decision: Inbox Check in Scheduler, Ingestion in Coordinator

**Context:** The inbox check could happen entirely in the scheduler (reading the file directly) or be routed through the coordinator actor.

**Decision:** Scheduler triggers the check; coordinator handles all file I/O and state mutation.

**Rationale:**
- All backlog state mutations must go through the coordinator actor (architectural constraint)
- ID generation requires access to `backlog.next_item_id` which lives in coordinator state
- The coordinator handler can do the entire read → parse → validate → ingest → save → delete sequence atomically (no interleaving with other commands)
- Scheduler remains simple — just calls `coordinator.ingest_inbox()` and logs the result

**Consequences:** The coordinator becomes responsible for inbox file I/O, not just state management. This is a minor expansion of coordinator responsibility but keeps all backlog-related I/O in one place. The coordinator processes commands sequentially via its actor loop, so the entire read → parse → validate → ingest → save → delete sequence completes without interleaving with other commands. This provides coordinator-level atomicity (no command can observe a partial state), though not system-level atomicity (a crash between save and delete can cause re-ingestion).

#### Decision: No Re-Snapshot After Ingestion

**Context:** After ingesting inbox items, the scheduler's snapshot is stale (doesn't include new items).

**Decision:** Do not re-fetch the snapshot after inbox ingestion.

**Rationale:**
- New items enter with status `New`, which is not selectable by `select_actions()` — only triage-promoted items (`Ready`) are selected
- The next scheduler loop iteration fetches a fresh snapshot that includes the new items
- Re-snapshotting would add latency with no functional benefit

**Consequences:** Newly ingested items are not visible to the current iteration's action selection. They become visible on the next iteration, which is the intended behavior.

#### Decision: Defer Git Commit

**Context:** PRD open question: should inbox-ingested items be committed to git immediately?

**Decision:** Defer git commit to the next phase completion (or halt, via WRK-032).

**Rationale:**
- Consistent with existing behavior — backlog changes from follow-up ingestion are committed on phase completion, not immediately
- Reduces git noise — no separate "ingested inbox items" commits
- WRK-032 (commit backlog on halt) provides a safety net for shutdown scenarios
- Items are persisted to disk immediately via `save_backlog()`, so only git history is deferred, not durability

**Consequences:** If the orchestrator crashes after ingesting but before the next phase commits, items are on disk but not in git. This is acceptable — the backlog file is the source of truth, not git.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Crash-window duplicates | If orchestrator crashes between save and delete, items are re-ingested with new IDs on restart | Simpler implementation — no idempotency markers or checksum tracking | Crash window is tiny (milliseconds), duplicates are benign and detectable |
| Partial write risk | If human is mid-write when orchestrator reads, parse fails and iteration is skipped | No file locking needed — simpler for both human and orchestrator | Retry on next iteration (seconds later) succeeds; no data loss |
| No immediate git commit | Inbox items are on disk but not in git until next phase completion | Less git noise, simpler implementation | Disk persistence provides durability; git is for history |
| Two similar ingestion functions | `ingest_inbox_items()` duplicates pattern of `ingest_follow_ups()` | Clean type separation, no impact on existing follow-up callers | ~25 lines of duplication is trivial cost for clear semantics |
| Blocking file I/O in async context | `std::fs` blocks tokio thread during inbox read | Consistency with existing backlog I/O patterns | Low single-digit millisecond blocking for small files is negligible |

---

## Alternatives Considered

### Alternative: Reuse FollowUp Type and IngestFollowUps Command

**Summary:** Convert `InboxItem` to `FollowUp` after parsing, route through existing `IngestFollowUps` command.

**How it would work:**
- Parse inbox as `Vec<InboxItem>`, convert to `Vec<FollowUp>` (mapping `title` → `title`, `description` → `context`)
- Call existing `coordinator.ingest_follow_ups(follow_ups, "inbox")`
- Add inbox file deletion as a separate step after ingestion

**Pros:**
- Fewer code changes — reuses existing command and handler
- No new coordinator command variant

**Cons:**
- Loses optional fields: `description` maps to `context` (imprecise), `impact`, `pipeline_type`, `dependencies` have no equivalent in `FollowUp`
- Inbox file deletion must happen outside the coordinator (or requires modifying `IngestFollowUps` to optionally delete a file)
- Conflates two semantically different operations

**Why not chosen:** The field set mismatch means inbox items lose metadata that users explicitly provided. The parallel path is only ~50 lines of new code and preserves all user-provided data.

### Alternative: File Watcher (inotify/notify)

**Summary:** Use the `notify` crate to watch for `BACKLOG_INBOX.yaml` creation/modification events.

**How it would work:**
- Set up a file watcher on the project root directory
- When `BACKLOG_INBOX.yaml` is created or modified, trigger ingestion immediately
- No polling needed — event-driven

**Pros:**
- Near-instant ingestion (no waiting for next scheduler loop)
- No polling overhead

**Cons:**
- Adds dependency on `notify` crate
- Cross-platform complexity (inotify on Linux, FSEvents on macOS, ReadDirectoryChangesW on Windows)
- File watcher events fire during partial writes, requiring debouncing
- PRD explicitly rules this out as out of scope
- Over-engineered for the expected usage pattern (human adding items occasionally)

**Why not chosen:** The scheduler loop already runs frequently enough (seconds to minutes). Polling once per iteration is simpler, reliable, and sufficient for the use case.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Crash between save and delete causes duplicate ingestion | Low — duplicates are detectable by matching title + origin | Low — crash window is milliseconds | Accept and document. Duplicates can be cleaned up during triage. |
| Concurrent write during read causes parse failure | Low — items are ingested on next iteration | Medium — depends on human timing | Parse failure logs warning, file left in place, retry next iteration |
| Backlog save failure loses inbox items from memory | Medium — items are in memory but not persisted | Low — disk failures are rare | Inbox file not deleted on save failure; items re-ingested on next attempt |
| Large inbox file (>50 items) causes slow ingestion | Low — even 1000 items would parse in milliseconds | Very Low — unrealistic usage | Log warning for >50 items as a sanity check (nice-to-have) |
| Permission error on inbox file deletion | Low — items are already saved to `BACKLOG.yaml`; inbox persists, causing duplicate ingestion on next iteration | Very Low — file was just read successfully | Log warning; items are safe. Persistent permission errors would cause repeated duplicates, but this indicates a misconfigured filesystem |
| Persistent malformed inbox file | Low — parse warning logged every iteration | Low — human may not notice typo | Log warning each iteration with parse error details. No backoff — keeps the feedback loop tight so the human sees the error quickly |

---

## Integration Points

### Existing Code Touchpoints

- `src/types.rs` — Add `InboxItem` struct (~15 lines)
- `src/backlog.rs` — Add `load_inbox()`, `ingest_inbox_items()`, `clear_inbox()` functions (~60 lines)
- `src/coordinator.rs` — Add `IngestInbox` command variant, `ingest_inbox()` handle method, `handle_ingest_inbox()` handler (~40 lines)
- `src/scheduler.rs` — Add inbox check call at top of scheduler loop (~10 lines)
- `src/main.rs` — Pass inbox path to coordinator state (~3 lines)
- `tests/backlog_test.rs` — Add tests for inbox loading, ingestion, and clearing (~80 lines)

### External Dependencies

None — all functionality uses existing crate dependencies (`serde`, `serde_yaml_ng`, `std::fs`, `chrono`).

---

## Inbox File Format

The inbox file is a bare YAML list, optimized for human writability:

```yaml
# Minimal — just a title
- title: "Fix login bug with special characters"

# With optional fields
- title: "Add dark mode support"
  description: "Users have requested dark mode for the dashboard"
  size: medium
  risk: low
  impact: high

# With dependencies
- title: "Migrate auth to OAuth2"
  description: "Replace custom auth with OAuth2 provider"
  dependencies:
    - WRK-012
    - WRK-015
```

No wrapper struct, no schema version. The file is ephemeral — consumed and deleted each iteration.

---

## Open Questions

- [x] ~~Should inbox items be committed to git immediately?~~ **Resolved: Deferred to next phase completion. WRK-032 provides safety net.**
- [x] ~~Should there be an inbox size limit?~~ **Resolved: No hard limit. Optional warning for >50 items.**
- [x] ~~Reuse FollowUp or parallel path?~~ **Resolved: Parallel InboxItem type + ingest_inbox_items().**
- [x] ~~tokio::fs vs std::fs?~~ **Resolved: std::fs for consistency with existing backlog I/O.**
- [x] ~~Re-snapshot after ingestion?~~ **Resolved: Not needed. New items are status New, not selectable.**

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements (must-have, should-have, nice-to-have)
- [x] Key flows are documented and make sense (5 flows: success, no-file, malformed, crash, save-failure)
- [x] Tradeoffs are explicitly documented and acceptable (5 tradeoffs)
- [x] Integration points with existing code are identified (6 files)
- [x] No major open questions remain (all 5 resolved)

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Initial design draft | Complete design with architecture, flows, decisions, alternatives, and risks |
| 2026-02-12 | Self-critique (7 agents) | Found ~86 raw issues. After dedup: 12 auto-fixed, 0 directional (all resolved by PRD/tech research), several quality items noted for SPEC phase |
| 2026-02-12 | Auto-fixes applied | Added: explicit field mapping table, TOCTOU implementation note, load_inbox/clear_inbox error semantics, inbox_path in CoordinatorState, fslock note, coordinator error handling flow, corrected blocking I/O time estimates, permission error risk, persistent malformed file risk |
