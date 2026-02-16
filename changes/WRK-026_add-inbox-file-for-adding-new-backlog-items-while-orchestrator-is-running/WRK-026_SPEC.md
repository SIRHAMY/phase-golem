# SPEC: Add Inbox File for Adding New Backlog Items While Orchestrator Is Running

**ID:** WRK-026
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-026_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

The orchestrator loads `BACKLOG.yaml` once at startup and holds it in memory. All mutations flow through the coordinator actor. Manual edits to `BACKLOG.yaml` while the orchestrator is running are silently overwritten. The only workaround is stopping the orchestrator, editing the file, and restarting. This feature adds a `BACKLOG_INBOX.yaml` drop-file that humans can write to at any time, which the orchestrator polls, ingests, and deletes each scheduler loop iteration.

The implementation follows the well-established drop-file/inbox pattern, reuses existing coordinator actor and backlog persistence machinery, and introduces a parallel `InboxItem` type to cleanly separate inbox input from follow-up input.

## Approach

Add a new `InboxItem` struct in `types.rs`, three pure functions in `backlog.rs` (`load_inbox`, `ingest_inbox_items`, `clear_inbox`), a new `CoordinatorCommand::IngestInbox` variant with its handler in `coordinator.rs`, a single call at the top of the scheduler loop in `scheduler.rs`, and startup wiring in `main.rs`.

The data flow is: scheduler calls `coordinator.ingest_inbox()` at the top of each loop iteration. The coordinator handler reads and parses `BACKLOG_INBOX.yaml`, validates items, creates `BacklogItem`s with generated IDs and status `New`, saves the backlog, then deletes the inbox file. The critical ordering is: read -> ingest -> save -> delete (never delete before save succeeds).

Key design decisions (from Design doc):
- **Parallel InboxItem type** — not reusing `FollowUp` because field sets diverge (`description`, `impact`, `pipeline_type`, `dependencies` vs `context`, `suggested_size`, `suggested_risk`)
- **New CoordinatorCommand variant** — inbox ingestion includes file I/O that follow-up ingestion doesn't
- **Use `std::fs`** — consistent with existing `backlog::load()` and `backlog::save()` patterns
- **No re-snapshot** — new items are status `New`, not selectable until triage promotes them
- **Defer git commit** — consistent with existing behavior; WRK-032 provides shutdown safety net

**Patterns to follow:**

- `src/backlog.rs:239-284` (`ingest_follow_ups()`) — model function for item creation: ID generation via `generate_next_id()`, field mapping, status setting, origin tracking, appending to backlog
- `src/coordinator.rs:201-216` (`CoordinatorHandle::ingest_follow_ups()`) — model for handle method: oneshot channel creation, command sending via `send_command()`
- `src/coordinator.rs:415-425` (`handle_ingest_follow_ups()`) — model for handler: call backlog functions, extract IDs, save state, return result
- `src/coordinator.rs:592-599` — model for actor loop match arm pattern
- `src/types.rs:147-188` (`BacklogItem`) — model for struct definition with serde attributes (`#[serde(default)]`, `Option<T>`)
- `tests/backlog_test.rs:1-58` — model for test setup: `fixture_path()`, `make_item()`, `empty_backlog()`, `TempDir` usage

**Implementation boundaries:**

- Do not modify: `FollowUp` struct or `ingest_follow_ups()` function
- Do not modify: existing `CoordinatorCommand` variants or their handlers
- Do not refactor: existing backlog I/O to `tokio::fs` (defer to separate refactor if needed)
- Do not add: file watching, CLI commands, duplicate detection, or git commit logic

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Types and Core Functions | Low | Define `InboxItem` struct and implement `load_inbox`, `ingest_inbox_items`, `clear_inbox` functions with unit tests |
| 2 | Coordinator Integration and Wiring | Med-High | Add `IngestInbox` command, handler, handle method, `inbox_path` state field, update all existing `spawn_coordinator()` call sites, wire into scheduler loop and main.rs, with integration tests |

**Ordering rationale:** Phase 1 defines the types and pure functions that Phase 2's coordinator handler depends on. Phase 2 includes all coordinator changes plus scheduler/main.rs wiring so that the codebase compiles and passes tests after each phase.

---

## Phases

Each phase should leave the codebase in a functional, stable state. Complete and verify each phase before moving to the next.

---

### Phase 1: Types and Core Functions

> Define `InboxItem` struct and implement inbox loading, ingestion, and clearing functions with unit tests

**Phase Status:** complete

**Complexity:** Low

**Goal:** Establish the `InboxItem` type and all pure/IO functions needed for inbox processing, fully tested in isolation before coordinator integration.

**Files:**

- `src/types.rs` — modify — add `InboxItem` struct (~15 lines)
- `src/backlog.rs` — modify — add `load_inbox()`, `ingest_inbox_items()`, `clear_inbox()` functions (~70 lines)
- `tests/backlog_test.rs` — modify — add unit tests for all three functions (~100 lines)

**Patterns:**

- Follow `src/types.rs:147-188` (`BacklogItem`) for struct definition with serde attributes
- Follow `src/backlog.rs:239-284` (`ingest_follow_ups()`) for item creation, ID generation, and field mapping
- Follow `src/backlog.rs:18-56` (`load()`) for file reading and error handling with `map_err()`
- Follow `tests/backlog_test.rs:1-58` for test helpers and `TempDir` usage

**Tasks:**

- [x] Add `InboxItem` struct to `src/types.rs` after the `FollowUp` type definition:
  - Required field: `title: String` (not Option — serde enforces presence)
  - Optional fields with `#[serde(default)]`: `description: Option<String>`, `size: Option<SizeLevel>`, `risk: Option<DimensionLevel>`, `impact: Option<DimensionLevel>`, `pipeline_type: Option<String>`, `dependencies: Vec<String>`
  - Derive only `Debug, Clone, Deserialize` (no Serialize — inbox is input-only)
  - No `#[serde(deny_unknown_fields)]` — unknown fields (including user-provided `id`) silently ignored for forward compatibility
- [x] Implement `load_inbox(inbox_path: &Path) -> Result<Option<Vec<InboxItem>>, String>` in `src/backlog.rs`:
  - Call `fs::read_to_string()` directly (TOCTOU-safe — no prior existence check)
  - If `ErrorKind::NotFound` → return `Ok(None)`
  - If other I/O error → return `Err(format!("Failed to read {}: {}", path.display(), e))`
  - If file is empty string or whitespace → return `Ok(Some(vec![]))` (treated as empty list)
  - Parse via `serde_yaml_ng::from_str::<Vec<InboxItem>>()` — if parse fails, return `Err` with parse error details
  - If parse succeeds → return `Ok(Some(items))`
- [x] Implement `ingest_inbox_items(backlog: &mut BacklogFile, items: &[InboxItem], prefix: &str) -> Vec<BacklogItem>` in `src/backlog.rs`:
  - For each item, validate `title.trim().is_empty()` — if empty/whitespace, log warning with `log_warn!` and skip (do not consume an ID)
  - For valid items: call `generate_next_id(backlog, prefix)` to get `(id, suffix)`, update `backlog.next_item_id = suffix`
  - Create `BacklogItem` with field mapping per design:
    - `id`: generated, `title`: from inbox item, `status`: `ItemStatus::New`
    - `origin`: `Some("inbox".to_string())`
    - `created` and `updated`: `chrono::Utc::now().to_rfc3339()`
    - `description`, `size`, `risk`, `impact`, `pipeline_type`, `dependencies`: pass through from inbox item
    - All other BacklogItem fields: `None`, `false`, `vec![]`, `0` as appropriate
  - Append each created item to `backlog.items`
  - Return vec of created `BacklogItem`s
- [x] Implement `clear_inbox(inbox_path: &Path) -> Result<(), String>` in `src/backlog.rs`:
  - Call `fs::remove_file(inbox_path)`
  - If `ErrorKind::NotFound` → return `Ok(())` (file already gone, not a problem)
  - If other error → return `Err` with error details (caller decides whether to propagate or log)
- [x] Write tests for `load_inbox()`:
  - File does not exist → returns `Ok(None)`
  - Valid YAML list with items → returns `Ok(Some(items))` with correct field values
  - Empty file → returns `Ok(Some(vec![]))` or handles gracefully
  - Malformed YAML → returns `Err` containing parse error details
  - Items with all optional fields populated → fields deserialized correctly
  - Items with unknown fields (including `id`) → unknown fields silently ignored
  - Items with invalid enum values (e.g., `size: "mega"`) → returns `Err` (entire file rejected)
  - YAML that is a mapping instead of a list (e.g., `title: "foo"` without leading `- `) → returns `Err` with clear parse error (realistic user mistake)
- [x] Write tests for `ingest_inbox_items()`:
  - Valid items create `BacklogItem`s with correct field mapping (status `New`, origin `"inbox"`, timestamps, pass-through fields)
  - Items with whitespace-only titles are skipped, valid items in same batch still ingested
  - IDs are generated sequentially via `generate_next_id()`, `next_item_id` updated correctly
  - Empty items slice → returns empty vec, no backlog changes
- [x] Write tests for `clear_inbox()`:
  - Existing file is deleted successfully
  - Non-existent file → returns `Ok(())` (no error)

**Verification:**

- [x] All new tests pass: `cargo test --test backlog_test`
- [x] Existing tests still pass (no regressions): `cargo test`
- [x] `cargo clippy` passes with no new warnings
- [x] Codebase builds without errors: `cargo build`

**Commit:** `[WRK-026][P1] Feature: Add InboxItem type and inbox load/ingest/clear functions with tests`

**Notes:** All 66 backlog tests pass. Full test suite (371 tests) passes with no regressions. No new clippy warnings. Code review found no critical or high issues; medium issue about missing `complexity` field is by-design per SPEC field mapping.

**Followups:**

---

### Phase 2: Coordinator Integration and Wiring

> Add IngestInbox coordinator command, handler, handle method, update all call sites, wire into scheduler and main.rs, with integration tests

**Phase Status:** complete

**Complexity:** Med-High

**Goal:** Extend the coordinator actor with an `IngestInbox` command that orchestrates the full inbox processing flow (read -> validate -> ingest -> save -> delete), wire the inbox path through from startup, call it from the scheduler loop, and update all existing `spawn_coordinator()` call sites to pass the new `inbox_path` parameter.

**Files:**

- `src/coordinator.rs` — modify — add command variant (~3 lines), state field (~1 line), handle method (~12 lines), handler function (~40 lines), actor loop match arm (~5 lines), update `run_coordinator()` signature (~2 lines)
- `src/scheduler.rs` — modify — add inbox ingestion call at top of scheduler loop (~8 lines)
- `src/main.rs` — modify — construct inbox path and pass to `spawn_coordinator()` (~3 lines per call site, 2 call sites)
- `tests/coordinator_test.rs` — modify — update all existing `spawn_coordinator()` call sites to pass inbox path (~39 call sites), add inbox integration tests (~100 lines)
- `tests/scheduler_test.rs` — modify — update all existing `spawn_coordinator()` call sites to pass inbox path (~14 call sites)
- `tests/executor_test.rs` — modify — update all existing `spawn_coordinator()` call sites to pass inbox path (~14 call sites)

**Patterns:**

- Follow `src/coordinator.rs:12-64` (`CoordinatorCommand` enum) for command variant structure
- Follow `src/coordinator.rs:201-216` (`ingest_follow_ups()` handle method) for oneshot channel and `send_command()` pattern
- Follow `src/coordinator.rs:415-425` (`handle_ingest_follow_ups()`) for handler: call backlog functions, extract IDs, save, return
- Follow `src/coordinator.rs:592-599` for actor loop match arm
- Follow `src/scheduler.rs:1248-1268` (`ingest_follow_ups()` helper) for coordinator call and logging pattern
- Follow `src/main.rs` coordinator initialization for path construction
- Follow `tests/coordinator_test.rs` existing test setup for `spawn_coordinator()` call patterns

**Tasks:**

- [x] Add `inbox_path: PathBuf` field to `CoordinatorState` struct (alongside existing `backlog_path`)
- [x] Add `IngestInbox` variant to `CoordinatorCommand` enum:
  ```rust
  IngestInbox {
      reply: oneshot::Sender<Result<Vec<String>, String>>,
  },
  ```
- [x] Add `ingest_inbox()` method to `CoordinatorHandle` impl:
  - Signature: `pub async fn ingest_inbox(&self) -> Result<Vec<String>, String>`
  - Create oneshot channel, send `IngestInbox { reply }`, await via `send_command()`
- [x] Implement `handle_ingest_inbox(state: &mut CoordinatorState) -> Result<Vec<String>, String>` with the following error flow:
  1. `load_inbox()` returns `Err(msg)` → `log_warn!("Failed to parse BACKLOG_INBOX.yaml: {}. File left in place for manual correction.", msg)`, return `Ok(vec![])`
  2. `load_inbox()` returns `Ok(None)` → return `Ok(vec![])` (no log — normal path)
  3. `load_inbox()` returns `Ok(Some(items))` where items is empty → call `clear_inbox()`, return `Ok(vec![])`
  4. Items are non-empty but all have blank titles → `ingest_inbox_items()` returns empty vec. Still call `save_backlog()` (no-op since nothing changed) and `clear_inbox()`. Return `Ok(vec![])`.
  5. `ingest_inbox_items()` creates items → call `state.save_backlog()`:
     - If save fails → `log_error!("Failed to save backlog after inbox ingestion: {}", e)`, **roll back in-memory changes** (truncate `backlog.items` to pre-ingestion length and restore `next_item_id`), return `Err(e)` — do NOT call `clear_inbox()`
     - If save succeeds → continue
  6. Call `clear_inbox(state.inbox_path)`:
     - If clear fails → `log_warn!("Failed to delete inbox file after ingestion: {}. Items already saved.", e)` — still return `Ok(new_ids)`
     - If clear succeeds → return `Ok(new_ids)`
  - Note: `prefix` for ID generation is accessed via `state.prefix`
- [x] Add match arm in `run_coordinator()` actor loop for `IngestInbox { reply }`:
  ```rust
  CoordinatorCommand::IngestInbox { reply } => {
      let result = handle_ingest_inbox(&mut state);
      let _ = reply.send(result);
  }
  ```
- [x] Update `run_coordinator()` function signature to accept `inbox_path: PathBuf` and pass it to `CoordinatorState` initialization
- [x] Update `spawn_coordinator()` function signature to accept `inbox_path: PathBuf` and forward it to `run_coordinator()`
- [x] In `src/main.rs`, construct inbox path and pass to `spawn_coordinator()`:
  - Add `let inbox_path = root.join("BACKLOG_INBOX.yaml");` before coordinator spawn call
  - Pass `inbox_path` to `spawn_coordinator()` in both call sites (orchestrate and triage commands)
- [x] Update all existing `spawn_coordinator()` call sites in test files to pass a dummy inbox path:
  - `tests/coordinator_test.rs` — ~39 call sites: pass `dir.path().join("BACKLOG_INBOX.yaml")` (using each test's existing temp dir)
  - `tests/scheduler_test.rs` — ~14 call sites: same pattern
  - `tests/executor_test.rs` — ~14 call sites: same pattern
- [x] Add inbox ingestion call in `run_scheduler()` loop, before `get_snapshot()` call (so items are persisted to disk before the snapshot is taken):
  ```rust
  match coordinator.ingest_inbox().await {
      Ok(new_ids) if !new_ids.is_empty() => {
          log_info!("Ingested {} items from inbox: {}", new_ids.len(), new_ids.join(", "));
      }
      Err(e) => {
          log_warn!("Inbox ingestion failed: {}", e);
      }
      _ => {} // No inbox file or empty — silent, no log
  }
  ```
- [x] Write integration tests in `tests/coordinator_test.rs` for inbox handling:
  - Inbox file with valid items → items appear in backlog after `ingest_inbox()`, inbox file deleted, returned IDs are correct, items have `origin: "inbox"` and `status: New`
  - Inbox file does not exist → `ingest_inbox()` returns `Ok(vec![])`, no side effects
  - Malformed inbox file → `ingest_inbox()` returns `Ok(vec![])`, file preserved on disk
  - Empty inbox file → `ingest_inbox()` returns `Ok(vec![])`, file deleted
  - Save failure (e.g., read-only backlog path) → `ingest_inbox()` returns `Err`, inbox file preserved, in-memory backlog unchanged (rolled back)
  - Clear failure (e.g., permissions) → `ingest_inbox()` returns `Ok(ids)`, items saved to backlog on disk

**Verification:**

- [x] All new and existing tests pass: `cargo test`
- [x] All existing coordinator, scheduler, and executor tests compile with updated `spawn_coordinator()` call sites
- [x] Coordinator handler correctly processes all error branches (verified by tests)
- [x] `cargo clippy` passes with no new warnings
- [x] Codebase builds without errors: `cargo build`

**Commit:** `[WRK-026][P2] Feature: Add IngestInbox coordinator command, handler, and scheduler wiring`

**Notes:** All 377 tests pass (6 new inbox integration tests + 371 existing). No new clippy warnings (3 pre-existing warnings unchanged). All 65 existing `spawn_coordinator()` call sites across 3 test files updated with `inbox_path` parameter. Handler implements all 6 error branches per SPEC with in-memory rollback on save failure.

The handler is the most complex part of this feature, with distinct error branches. Each branch has specific side effects (log level, file deletion, reply value). Key implementation details:
- Parse errors leave the inbox file in place for human fixing
- Save failures leave the inbox file in place AND roll back in-memory changes (truncate `backlog.items` to pre-ingestion length, restore `next_item_id`) to prevent duplicate accumulation on retry
- Clear failures still reply with success (items are already saved)
- `prefix` for ID generation is accessed via `state.prefix` (already on `CoordinatorState`)
- The `spawn_coordinator()` signature change cascades to ~67 existing call sites across 3 test files — this is mechanical but must be done to keep the test suite compiling

No re-snapshot is needed after inbox ingestion. New items enter with status `New`, which is not selectable by `select_actions()`. The next scheduler loop iteration fetches a fresh snapshot that includes the new items. The inbox check is placed before `get_snapshot()` so items are persisted to disk before the scheduler takes its snapshot.

**Followups:**

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met:
  - [x] Orchestrator reads `BACKLOG_INBOX.yaml` on each scheduler loop iteration via direct read (TOCTOU-safe)
  - [x] Missing file produces no log output
  - [x] Inbox items assigned unique IDs via `generate_next_id()`
  - [x] Items ingested with status `New` and persisted to `BACKLOG.yaml`
  - [x] Items have `origin: "inbox"` for traceability
  - [x] Inbox file deleted after successful ingestion and save
  - [x] Malformed YAML logs warning, doesn't halt orchestrator, file left in place
  - [x] Title is required and validated non-empty
  - [x] User-provided `id` field silently ignored
  - [x] Unknown YAML fields silently ignored
  - [x] Info-level log on ingestion with count and IDs
  - [x] Individual invalid items skipped while valid items still ingested
  - [x] Optional fields preserved on ingested items
  - [x] Safe operation ordering: read -> ingest -> save -> delete
  - [x] Empty inbox file deleted with no warning
- [x] Tests pass
- [x] No regressions introduced
- [x] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| Phase 1: Types and Core Functions | complete | [WRK-026][P1] | InboxItem type + load_inbox/ingest_inbox_items/clear_inbox + 24 tests |
| Phase 2: Coordinator Integration and Wiring | complete | [WRK-026][P2] | IngestInbox command + handler with rollback + scheduler wiring + 6 integration tests + 65 test call site updates |

## Followups Summary

### Critical

### High

### Medium

- [ ] Add warning log for large inbox files (>50 items) — PRD Nice to Have, deferred from initial implementation. A simple `if items.len() > 50 { log_warn!(...) }` in the handler.
- [ ] Add `BacklogItem` constructor or `Default` impl — Currently three manual construction sites (`ingest_follow_ups`, `ingest_inbox_items`, `make_item` test helper) spell out all ~20 fields. Future field additions require updating all sites. A builder, factory function, or `Default` impl would reduce this maintenance burden.

### Low

- [ ] Atomic file operations for concurrent inbox writes — PRD Nice to Have. Current approach handles partial writes via parse-error-and-retry, which is adequate for human writers. Atomic write-to-temp-then-rename would eliminate the partial-write window entirely but adds complexity for minimal benefit given the polling frequency.
- [ ] Duplicate title detection for crash-recovery scenarios — If orchestrator crashes between save and delete, inbox items are re-ingested with new IDs on restart. A future triage enhancement could detect items with matching title + `origin: "inbox"` and flag potential duplicates.

## Design Details

### Key Types

```rust
/// Simplified input schema for human-written inbox items.
/// Deserialized from BACKLOG_INBOX.yaml.
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

```rust
/// New coordinator command variant
CoordinatorCommand::IngestInbox {
    reply: oneshot::Sender<Result<Vec<String>, String>>,
}
```

### InboxItem to BacklogItem Field Mapping

| InboxItem field | BacklogItem field | Value |
|-----------------|-------------------|-------|
| `title` | `title` | Required, validated non-empty |
| `description` | `description` | Optional, passed through |
| `size` | `size` | Optional, may be overridden by triage |
| `risk` | `risk` | Optional, may be overridden by triage |
| `impact` | `impact` | Optional, may be overridden by triage |
| `pipeline_type` | `pipeline_type` | Optional, passed through |
| `dependencies` | `dependencies` | Optional, defaults to empty vec |
| _(generated)_ | `id` | Via `generate_next_id()` |
| _(hardcoded)_ | `status` | `ItemStatus::New` |
| _(hardcoded)_ | `origin` | `Some("inbox".to_string())` |
| _(generated)_ | `created` | `chrono::Utc::now().to_rfc3339()` |
| _(generated)_ | `updated` | Same as `created` |
| _(default)_ | All other fields | `None`, `false`, `vec![]`, `0` |

### Architecture Details

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
                              │ load_inbox │
                              │ validate   │
                              │ ingest     │
                              └─────┬─────┘
                                    │
                                    ▼
                              BACKLOG.yaml (save)
                                    │
                                    ▼
                              Delete BACKLOG_INBOX.yaml
```

### Coordinator Handler Error Flow

```
load_inbox()
├── Err(msg) → log_warn, reply Ok(vec![]), file preserved
├── Ok(None) → reply Ok(vec![]), no log
└── Ok(Some(items))
    ├── items.is_empty() → clear_inbox, reply Ok(vec![])
    └── items non-empty
        └── record pre-ingestion state (items len, next_item_id)
            └── ingest_inbox_items()
                ├── returns empty vec (all invalid) → save_backlog, clear_inbox, reply Ok(vec![])
                └── returns created items → save_backlog()
                    ├── Err(e) → ROLLBACK (truncate items, restore next_item_id),
                    │             log_error, reply Err(e), file preserved
                    └── Ok(()) → clear_inbox()
                        ├── Err(e) → log_warn, reply Ok(ids) (items saved)
                        └── Ok(()) → reply Ok(ids)
```

### Design Rationale

**Why parallel InboxItem type:** The `FollowUp` struct has fields (`context`, `suggested_size`, `suggested_risk`) that don't map to inbox needs (`description`, `impact`, `pipeline_type`, `dependencies`). Forcing inbox items through the follow-up path loses user-provided metadata. The parallel path is ~25 lines of new code — minimal duplication for clean semantics.

**Why std::fs over tokio::fs:** The coordinator actor already uses synchronous I/O for `backlog::load()` and `backlog::save()`. Switching to `tokio::fs` for inbox only would be inconsistent. The blocking window is negligible for small files.

**Why no re-snapshot:** New inbox items enter as status `New`, which `select_actions()` doesn't select. Re-snapshotting would add latency with no functional benefit.

**Why defer git commit:** Consistent with existing behavior where backlog changes from follow-up ingestion are committed on phase completion. WRK-032 (commit backlog on halt) provides a safety net for shutdown scenarios.

## Assumptions

Decisions made without human input during autonomous SPEC creation:

1. **Two-phase structure:** Initially planned three phases (types/functions, coordinator, wiring), but self-critique revealed that separating coordinator changes from `spawn_coordinator()` call site updates would leave the codebase non-compiling between phases. Merged coordinator integration and wiring into a single Phase 2 to ensure each phase leaves the codebase in a stable, building state.

2. **Tests co-located with implementation phases:** Unit tests for backlog functions are in Phase 1 (`backlog_test.rs`), coordinator integration tests are in Phase 2 (`coordinator_test.rs`). This follows the existing codebase convention where coordinator handle tests live in `coordinator_test.rs`.

3. **In-memory rollback on save failure:** Self-critique identified that if `save_backlog()` fails after `ingest_inbox_items()` mutates the backlog in place, the in-memory state would contain orphaned items that accumulate as duplicates on retry. The handler must truncate `backlog.items` and restore `next_item_id` on save failure. This was not in the Design doc but is necessary for correctness.

4. **Empty string handling in load_inbox:** An empty or whitespace-only file will be treated as an empty list rather than a parse error. This matches the PRD's "empty inbox file... deleted with no warning" requirement.

5. **Inbox check placement:** Placed before `get_snapshot()` rather than after. The PRD says "at the top of each scheduler loop iteration" and placing it before snapshot ensures items are persisted to disk before the snapshot is taken. Since new items are status `New` and not selectable by `select_actions()`, the exact placement relative to snapshot has no functional effect, but before-snapshot is cleaner semantically.

6. **PRD scope deviation — parallel InboxItem type:** The PRD assumed reusing `ingest_follow_ups()` (Assumption #3, In-Scope section). The Design doc and tech research found that `FollowUp` and `InboxItem` have divergent field sets, so a parallel type and function were chosen instead. This is a documented deviation with clear rationale (preserving user-provided `description`, `impact`, `pipeline_type`, `dependencies` fields).

7. **PRD assumption clarification — immediate visibility:** PRD Assumption #7 states items should be "immediately visible to scheduling logic." Since inbox items enter as status `New` (not selectable until triage promotes them), they are persisted immediately but not schedulable until the next triage cycle. This is the intended and correct behavior per both Design doc and PRD success criteria.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
