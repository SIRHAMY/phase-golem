# Change: Add Inbox File for Adding New Backlog Items While Orchestrator Is Running

**Status:** Proposed
**Created:** 2026-02-13
**Author:** AI Agent (autonomous PRD)

## Problem Statement

The orchestrator loads `BACKLOG.yaml` once at startup and holds it in memory as the single source of truth. All state mutations flow through the coordinator actor (a tokio-based actor that serializes all backlog reads and writes through a command channel), which writes changes back to disk on phase completions, follow-up ingestion (when agents discover new work items during phase execution), and archival. This architecture ensures consistency but creates a critical usability gap: **users cannot add work items to a running orchestrator**.

Manual edits to `BACKLOG.yaml` while the orchestrator is running are silently ignored and overwritten on the next coordinator save. This behavior was confirmed in practice: newly added items in `BACKLOG.yaml` were overwritten when the orchestrator next persisted state. The only workaround is stopping the orchestrator, editing the file, and restarting — which disrupts long-running sessions, loses scheduler momentum, and risks re-processing partially completed items.

## User Stories / Personas

- **Solo developer running the orchestrator** - Discovers a new bug or feature idea mid-session and wants to queue it without interrupting the current run. Currently must stop the orchestrator, add the item, and restart.

- **Developer reviewing orchestrator output** - While reviewing completed phases, identifies follow-up work that wasn't captured by the agent's own follow-up detection. Wants to add it to the backlog immediately rather than tracking it externally and remembering to add it later.

- **Developer with urgent priority work** - A high-priority issue surfaces during a long orchestrator run. Wants to add it to the backlog so the scheduler can pick it up on the next iteration, rather than waiting for the current run to complete.

## Desired Outcome

Users can create a `BACKLOG_INBOX.yaml` file alongside `BACKLOG.yaml` and add new work items to it at any time — even while the orchestrator is actively running. The orchestrator checks for this file at the top of each scheduler loop iteration (the main loop in `run_scheduler()` that selects and dispatches work — each iteration typically takes seconds to minutes depending on phase execution time), ingests any items it finds into the in-memory backlog (assigning proper IDs and setting status to `new`), persists them to `BACKLOG.yaml`, and clears the inbox. Items appear in the backlog within one scheduler loop iteration without any orchestrator restart.

The inbox is human-write-only, orchestrator-read-only. There are no competing writes to `BACKLOG.yaml` and no merge logic needed.

## Success Criteria

### Must Have

- [ ] Orchestrator attempts to read `BACKLOG_INBOX.yaml` from the project root on each scheduler loop iteration (using direct read, not existence-check-then-read, to avoid TOCTOU race conditions)
- [ ] If the file does not exist, the orchestrator proceeds silently with no log output
- [ ] Inbox items are assigned unique IDs via the existing `generate_next_id()` machinery (thread-safe because all ID generation runs through the serialized coordinator actor)
- [ ] Inbox items are ingested into the in-memory backlog with status `new` and persisted to `BACKLOG.yaml`
- [ ] Inbox items have `origin` set to `"inbox"` for traceability (distinguishing them from phase-generated follow-ups which use `origin: "<item_id>/<phase>"`)
- [ ] `BACKLOG_INBOX.yaml` is deleted after successful ingestion and backlog save
- [ ] Malformed inbox YAML (invalid YAML syntax or structural errors) logs a warning with the parse error details and does not halt the orchestrator; the file is left in place for the user to fix
- [ ] Inbox items require `title` as a mandatory field (must be non-empty and not whitespace-only); items with missing or blank titles are skipped with a warning
- [ ] Any `id` field provided in inbox items is silently ignored; the orchestrator always generates IDs
- [ ] Unknown YAML fields in inbox items are silently ignored (forward compatibility)

### Should Have

- [ ] Orchestrator logs an info-level message when items are ingested, including count and assigned IDs (e.g., `"Ingested 3 items from inbox: WRK-043, WRK-044, WRK-045"`)
- [ ] Individual malformed items (missing title, invalid field values) are skipped with a warning naming the problematic item, while valid items in the same file are still ingested
- [ ] Inbox items can optionally include `description`, `size`, `risk`, `impact`, `pipeline_type`, and `dependencies` fields; optional fields provided by the user are preserved on the ingested backlog item (triage may later override assessments)
- [ ] Operations are ordered safely: read inbox -> ingest items -> save backlog -> delete inbox file
- [ ] If an empty inbox file is detected (valid YAML but zero items), it is deleted with no warning

### Nice to Have

- [ ] Graceful handling of concurrent inbox writes (human writing while orchestrator reads) via atomic file operations
- [ ] Warning logged if inbox file contains an unexpectedly large number of items (e.g., >50)

## Scope

### In Scope

- Single `BACKLOG_INBOX.yaml` file at the project root (sibling to `BACKLOG.yaml`)
- Inbox item schema: simplified structure requiring only `title`, with optional metadata fields
- Scheduler loop integration: check for inbox at top of each iteration
- Ingestion via existing coordinator command pathway (reuse or extend `ingest_follow_ups`)
- Inbox clearing (file deletion) after successful ingestion
- Error handling: log and skip on parse failures, continue orchestrator execution
- Unit and integration tests for inbox loading, ingestion, and clearing

### Out of Scope

- Real-time file watching (inotify/FSEvents) — polling once per scheduler loop is sufficient
- Directory-based inbox (`_inbox/` with multiple files) — single file is simpler for v1
- Inbox item approval/staging workflow — items are ingested immediately like follow-ups
- Structured description format in inbox — deferred to WRK-028; inbox keeps freeform `description`
- CLI commands for inbox management (`orchestrate inbox add`, `orchestrate inbox list`) — users edit the YAML directly
- Duplicate title detection — trust users to avoid duplicates; ID uniqueness is enforced
- Inbox archival or rotation — deleted after ingestion; git history provides audit trail
- Merge or conflict resolution between inbox and backlog

## Non-Functional Requirements

- **Performance:** Inbox check adds < 1ms per loop iteration when no inbox file exists (single failed file read). Full parse of a 1-10 item inbox is < 5ms. No measurable impact on scheduler loop latency.
- **Durability:** Operation ordering (ingest -> save backlog -> delete inbox) ensures items are persisted to `BACKLOG.yaml` before the inbox is cleared. If the orchestrator crashes between save and delete, duplicate ingestion on restart is acceptable — items get new IDs but the backlog remains consistent. If the backlog save fails (e.g., disk full), the inbox file is not deleted so items are not lost.
- **Observability:** Info-level log on successful ingestion with item count and IDs. Warning-level log on parse errors or skipped items with error details. Missing inbox file produces no log output. No new metrics or alerting required.

## Constraints

- Must use the existing coordinator actor pattern — inbox ingestion flows through `CoordinatorCommand` (the message-passing interface to the coordinator actor), not direct state mutation. This ensures all backlog modifications are serialized and thread-safe.
- Must reuse existing ID generation (`generate_next_id()`) to maintain `next_item_id` consistency. This function runs within the coordinator actor, so it is inherently serialized — no additional synchronization needed.
- Inbox items enter as status `new` — the existing triage phase (the orchestrator's assessment stage that evaluates, sizes, and promotes items to `ready`) handles all assessment and promotion
- The inbox file is not protected by the orchestrator's file system lock (fslock) — this is intentional. The fslock prevents multiple orchestrator instances from running simultaneously, but humans must be able to write to the inbox while the orchestrator holds the lock on `BACKLOG.yaml`.
- Inbox items should not include an `id` field — any provided `id` is silently ignored, and the orchestrator generates IDs to prevent collisions

## Dependencies

- **Depends On:** None — this is additive with no prerequisites
- **Blocks:** WRK-028 (structured description format) depends on this for inbox format coordination

## Risks

- [ ] **Race condition on inbox deletion:** If the orchestrator crashes after saving the backlog but before deleting the inbox, items will be re-ingested on restart with new IDs. The re-ingested items create duplicates with identical titles but different IDs. Mitigation: these duplicates are detectable (same title, `origin: "inbox"`) and can be cleaned up during triage. Preflight duplicate title warnings could catch this in a future improvement.
- [ ] **Concurrent inbox write during read:** If a human is mid-write when the orchestrator reads, the YAML may be partially written and fail to parse. Mitigation: the orchestrator logs a warning with the parse error and skips the inbox for this iteration. The human's write will complete before the next scheduler loop iteration, at which point the file will parse successfully.
- [ ] **Inbox items with `id` field:** If a user includes an `id` field, it could conflict with existing IDs. Mitigation: any `id` field in inbox items is silently ignored; the orchestrator always generates IDs via `generate_next_id()`.
- [ ] **Backlog save failure:** If `BACKLOG.yaml` fails to write (e.g., disk full, permission error) after inbox items have been ingested into memory, the inbox file must not be deleted. The in-memory state will contain the items but they won't be persisted until the next successful save. Mitigation: only delete the inbox file after a confirmed successful backlog save.

## Open Questions

- [ ] Should the orchestrator commit inbox-ingested items to git immediately after ingestion, or let them be committed with the next phase completion? Immediate commit ensures durability; deferred commit reduces git noise. Interaction with WRK-032 (commit backlog on halt) may resolve this — if halt commits the backlog, deferred commits are safe.
- [ ] Should there be a maximum inbox file size or item count limit to prevent accidental bulk ingestion, or is unbounded ingestion acceptable for the expected use case?

## Assumptions

Decisions made without human input during autonomous PRD creation:

1. **Single file over directory:** Chose `BACKLOG_INBOX.yaml` (single file) over `_inbox/` directory for simplicity. A directory model adds complexity (glob, ordering, partial processing) without clear benefit for the expected usage pattern of adding a few items at a time.

2. **Delete over truncate:** Chose to delete the inbox file after ingestion rather than truncating to empty. Deletion is simpler and makes it obvious the inbox was processed. An empty file is ambiguous (was it processed or was it always empty?).

3. **Reuse follow-up ingestion:** Chose to reuse the existing `ingest_follow_ups()` coordinator command pathway rather than creating a separate inbox-specific command. The operations are nearly identical (create items with generated IDs, save backlog). Using `origin: "inbox"` distinguishes inbox items from phase-generated follow-ups.

4. **No batch size limit:** Chose not to limit how many inbox items can be ingested per loop iteration. The expected usage is a handful of items. A limit adds complexity and could confuse users whose items aren't immediately visible.

5. **Silently ignore `id` field:** Chose to silently ignore any `id` field provided in inbox items rather than warning. Users shouldn't need to think about IDs when adding items — the orchestrator handles ID generation exclusively.

6. **No idempotency marker for crash recovery:** Chose to accept duplicate ingestion on crash (between save and delete) rather than adding a `.inbox.processed` marker file or checksum tracking. The crash window is small, duplicates are detectable by title match, and the added complexity is not justified for the expected failure frequency.

7. **Inbox ingestion before action selection:** The inbox check must happen in the scheduler loop before `select_actions()` is called, so newly ingested items are immediately visible to scheduling logic. This is a synchronous operation through the coordinator actor.

8. **Mode: medium:** This PRD was created in medium mode with 3 discovery agents (problem space, scope, risk/constraints).

## References

- WRK-026 backlog item description (BACKLOG.yaml)
- Existing follow-up ingestion: `src/backlog.rs::ingest_follow_ups()`
- Coordinator actor: `src/coordinator.rs::run_coordinator()`
- Scheduler loop: `src/scheduler.rs::run_scheduler()`
- WRK-028: Structured description format (depends on inbox format decisions)
- WRK-032: Commit backlog on halt (ensures inbox-ingested items are committed on shutdown)
