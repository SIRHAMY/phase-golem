# WRK-026: Add Inbox File for Adding New Backlog Items While Orchestrator Is Running

## What This Involves

Add a `BACKLOG_INBOX.yaml` file that humans can write to while the orchestrator is running. The orchestrator checks this file at the top of each scheduler loop iteration, ingests any items found into the in-memory backlog using existing ingestion machinery, and clears the inbox file. This eliminates the current problem where manual edits to `BACKLOG.yaml` are silently overwritten by the running orchestrator.

## Key Files

**Must modify:**
- `src/coordinator.rs` — Add `IngestInbox` command variant, `inbox_path` to `CoordinatorState`, and handler function
- `src/scheduler.rs` — Add inbox check at top of scheduler loop (after snapshot retrieval, before action selection)
- `src/backlog.rs` — Add inbox parsing function and inbox-specific item creation (similar to `ingest_follow_ups`)
- `src/types.rs` — Define `InboxItem` struct (simplified: title, description, optional size/risk/impact estimates)
- `src/main.rs` — Pass inbox path to coordinator on startup

**May modify:**
- `src/config.rs` — Optionally make inbox filename configurable (not required for v1)

## Approach Sketch

1. **Define `InboxItem` type** in `types.rs` — a simplified struct with `title`, optional `description`, optional `size`, `risk`, `impact` estimates. Uses serde for YAML deserialization. Simpler than `BacklogItem` since triage will fill in missing fields.

2. **Add inbox loading/clearing** in `backlog.rs` — A `load_inbox()` function that reads `BACKLOG_INBOX.yaml`, parses it as a `Vec<InboxItem>`, and returns the items. A `clear_inbox()` function that truncates or deletes the file after ingestion. Handle gracefully: missing file (no items), empty file (no items), malformed entries (log warning, skip).

3. **Add coordinator command** — New `IngestInbox` command in `CoordinatorCommand` enum. Handler reads inbox, converts `InboxItem`s to `BacklogItem`s with status `New`, assigns IDs via existing `generate_next_id()`, appends to in-memory backlog, saves, clears inbox.

4. **Check inbox in scheduler loop** — At the top of each iteration (after `get_snapshot()` on line 501), call `coordinator.ingest_inbox()`. If new items were ingested, re-fetch snapshot before calling `select_actions()`. This ensures new items are visible immediately.

5. **Ownership model** — Humans exclusively write `BACKLOG_INBOX.yaml`. Orchestrator exclusively writes `BACKLOG.yaml`. No competing writes, no merge logic needed. The inbox file is ephemeral — items move from inbox to backlog, then the inbox is cleared.

## Risks or Concerns

- **Race condition on inbox file**: Human could be mid-write when orchestrator reads. Mitigation: read the file, attempt to parse. If parse fails, leave the file alone and retry next iteration. Only clear the file after successful parse + ingestion.
- **Partial writes**: Human's editor might write a partial YAML file. Same mitigation as above — parse failure means skip and retry.
- **File locking**: Not needed. The read-parse-clear cycle is fast, and the retry-on-failure approach handles concurrent access adequately for this use case.
- **ID collisions with manual items**: Not a risk — the orchestrator generates IDs from `next_item_id` high-water mark, so inbox items get sequentially assigned IDs regardless of what the human wrote.

## Assessment

| Dimension  | Rating | Justification |
|------------|--------|---------------|
| Size       | medium | 5-7 files modified, new type + parsing + command + scheduler integration |
| Complexity | medium | Multiple design decisions (inbox format, error handling, re-snapshot logic), but follows established patterns |
| Risk       | low    | Purely additive — existing behavior unchanged if no inbox file exists. Graceful degradation on parse errors. |
| Impact     | high   | Directly solves a usability pain point encountered in practice (items being overwritten by running orchestrator) |

## Assumptions

- Inbox file will be `BACKLOG_INBOX.yaml` in the project root (same directory as `BACKLOG.yaml`)
- Inbox format is a simple YAML list of items, not a full `BacklogFile` with schema version
- Inbox items start as status `New` and go through the normal triage pipeline
- The orchestrator clears the inbox after successful ingestion (no archive of inbox entries)
- No CLI command needed to add to inbox — humans edit the file directly
