# WRK-026: Add inbox file for adding new backlog items while orchestrator is running

## Problem Statement

The orchestrator loads `BACKLOG.yaml` once at startup and holds it in memory as the source of truth. It writes back to the file when state changes occur (phase completions, follow-up ingestion, archival). Any manual edits to `BACKLOG.yaml` while the orchestrator is running are silently ignored and overwritten on the next save. This was confirmed in practice when newly added items were clobbered by a running orchestrator.

Users cannot add work items without stopping and restarting the orchestrator, which is disruptive during long-running sessions.

## Proposed Approach

### 1. Inbox file format (`BACKLOG_INBOX.yaml`)

Introduce a new file `BACKLOG_INBOX.yaml` that humans write to while the orchestrator runs. The format should be simple — a list of items with minimal required fields:

```yaml
items:
  - title: "Fix login timeout bug"
    description: "Users report 30s timeout on login page"
    size: small
    risk: low
    impact: high
  - title: "Add CSV export to reports"
```

Only `title` is required. Other fields (description, size, risk, impact) are optional — triage will fill in missing assessments. Items in the inbox do not have IDs; the orchestrator assigns IDs from its `next_item_id` counter during ingestion.

### 2. Inbox ingestion in the scheduler loop (`scheduler.rs`)

At the top of each scheduler loop iteration (after the shutdown/circuit-breaker checks, before `get_snapshot()`), add an inbox check step:

1. Check if `BACKLOG_INBOX.yaml` exists and is non-empty
2. Parse the inbox items
3. Send them to the coordinator for ingestion (reuse or extend the existing `ingest_follow_ups` machinery)
4. Clear/delete the inbox file after successful ingestion
5. Log how many items were ingested

This ensures new items are picked up within one loop iteration (typically seconds).

### 3. Coordinator command for inbox ingestion (`coordinator.rs`)

Add a new `CoordinatorCommand::IngestInbox` variant (or reuse `IngestFollowUps` with a different origin marker). The coordinator:

1. Assigns IDs from the `next_item_id` counter
2. Sets `status: New` and `created`/`updated` timestamps
3. Appends items to the in-memory backlog
4. Persists to `BACKLOG.yaml`
5. Returns the count of ingested items

### 4. Inbox data types (`types.rs`)

Define an `InboxItem` struct with:
- `title: String` (required)
- `description: Option<String>`
- `size: Option<SizeLevel>`
- `risk: Option<DimensionLevel>`
- `impact: Option<DimensionLevel>`
- `dependencies: Option<Vec<String>>`

This is intentionally simpler than `BacklogItem` — it's the human-friendly entry format.

### 5. Error handling

- Malformed YAML: Log a warning, leave the inbox file intact (don't lose the user's input), continue the scheduler loop without ingesting
- Partial parse: If some items parse and others don't, consider rejecting the entire file to avoid confusion about which items were ingested
- File locking: Not needed — the orchestrator is the only reader, and human writes are atomic enough at the file level for this use case
- Empty file: Treat as no-op (no items to ingest)

## Files Affected

- **New:**
  - None (inbox type can live in existing files)

- **Modified:**
  - `orchestrator/src/types.rs` — Add `InboxItem` and `InboxFile` structs
  - `orchestrator/src/scheduler.rs` — Add inbox check at top of scheduler loop
  - `orchestrator/src/coordinator.rs` — Add `IngestInbox` command handler (or extend `IngestFollowUps`)
  - `orchestrator/src/backlog.rs` — Add inbox parsing function, conversion from `InboxItem` to `BacklogItem`
  - `orchestrator/tests/` — Integration tests for inbox ingestion (happy path, malformed YAML, empty file, concurrent writes)

Estimated ~5-7 files modified, ~200-300 lines of new code, ~200 lines of tests.

## Assessment

| Dimension  | Rating | Rationale |
|------------|--------|-----------|
| Size       | Medium | 5-7 files modified; new data type, scheduler integration, coordinator command, parsing logic, tests |
| Complexity | Medium | Design decisions around error handling strategy, inbox format, and coordinator integration. The inbox-to-backlog conversion needs to handle missing fields gracefully. |
| Risk       | Low    | Purely additive — if no inbox file exists, behavior is identical to today. No changes to existing data flow. The inbox is a separate file with no competing-write risk. |
| Impact     | High   | Directly solves a usability pain point encountered in practice. Prerequisite for WRK-028 (structured descriptions). Enables continuous orchestrator operation without restart. |

## Edge Cases

- **Race condition on clear**: Orchestrator reads inbox, user writes new items before orchestrator clears it. Mitigation: read-then-delete atomically (rename to `.ingesting`, parse, delete). Or accept the tiny window — items will be picked up on next iteration if written after the read.
- **Duplicate titles**: Items with identical titles to existing backlog items should still be ingested (they get different IDs). Triage can identify and handle duplicates.
- **Dependency references**: Inbox items referencing dependencies by ID requires the user to know existing IDs. This is acceptable since `orchestrate status` shows IDs.
- **Large inbox**: Hundreds of items at once. Should work fine — ingestion is a single coordinator command.
- **Orchestrator not running**: Inbox file sits untouched. When orchestrator starts, it should check for inbox items during startup (or first loop iteration handles it).

## Assumptions

- The inbox file is `BACKLOG_INBOX.yaml` at the project root (same level as `BACKLOG.yaml`). Alternative: an `_inbox/` directory with one file per item was considered but YAML list is simpler.
- Only `title` is required in inbox items. All other fields are optional and default to `None`/unset, letting triage fill them in.
- Inbox items always start as `status: New` regardless of any fields provided. The orchestrator's triage phase handles assessment and promotion.
- The inbox file is deleted (not truncated) after successful ingestion. Users create a new file for the next batch.
- No file locking mechanism is needed. The orchestrator reads once per loop iteration, and human writes between reads are safe at the filesystem level.
- Error handling strategy: reject the entire inbox file if any item fails to parse, to avoid partial ingestion confusion. Log the parse error so the user can fix and retry.
