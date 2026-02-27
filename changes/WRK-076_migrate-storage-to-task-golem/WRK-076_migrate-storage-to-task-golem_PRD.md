# Change: Migrate Phase-Golem Storage to Task-Golem

**Status:** Reviewed
**Created:** 2026-02-24
**Author:** Human + Claude

## Problem Statement

Phase-golem currently manages its own task storage layer: a YAML-based backlog (`BACKLOG.yaml`) with schema migrations (v1->v2->v3), atomic file writes, an inbox drop-file mechanism, and sequential ID generation. This is ~1,350 lines of code (`backlog.rs`, `migration.rs`, parts of `coordinator.rs`) that duplicates concerns better handled by a dedicated tool.

Meanwhile, task-golem exists as a purpose-built, agent-native work tracker with JSONL storage, file locking, hex-based IDs, dependency cycle detection, and an extension field system — but it has no real consumers yet. Phase-golem continuing to maintain its own storage layer means:

1. **Duplicated effort** — Two projects solving the same "persist task state" problem differently
2. **No dogfooding** — task-golem doesn't get real usage pressure to validate its design
3. **No standard** — There's no shared substrate for other agents/tools to build against
4. **Inbox friction** — Adding items requires a YAML drop-file; `tg add` would be simpler
5. **Coordinator complexity** — The coordinator is a full async actor holding in-memory state, when simpler read-through patterns (loading current state from disk on every read rather than maintaining an in-memory copy) would suffice

## User Stories / Personas

- **Phase-golem (the orchestrator)** — Needs reliable task state persistence, dependency tracking, and atomic updates. Wants to focus on pipeline orchestration, not storage plumbing.
- **Human operator** — Wants to inspect and modify work items via a familiar CLI (`tg list`, `tg show`, `tg add`) instead of editing YAML files. Wants to add items while the orchestrator is running without a special inbox mechanism.
- **Future agents/tools** — Want a standard task storage layer they can read from and write to without understanding phase-golem's internals.

## Desired Outcome

Phase-golem uses task-golem as a Rust crate dependency for all task state persistence. The YAML backlog, schema migrations, inbox mechanism, and sequential ID generation are removed. Task-golem gains a library interface (`lib.rs`) and a thin git module for committing staged changes. Phase-golem's coordinator remains a thin async actor for write serialization but delegates all storage operations to task-golem. The coordinator retains in-memory state only for batch commit accumulation (`pending_batch_phases`) and reads backlog state through the task-golem store.

Items are stored in task-golem's JSONL format. Phase-golem-specific fields (phase, pipeline type, assessments, structured descriptions) live in task-golem's extension fields (`x-pg-*`). A typed adapter layer in phase-golem provides ergonomic access to these extensions.

Git commits are triggered by task-golem when state changes are persisted. The orchestrator stages phase artifacts (PRDs, code, designs) before calling task-golem to update state and commit. This makes the task state transition the natural commit boundary.

## Success Criteria

### Must Have

- [ ] task-golem exposes `model`, `store`, and `errors` modules via `lib.rs`
- [ ] task-golem exposes `generate_id_with_prefix()` via the library API so consumers can generate IDs with custom prefixes (e.g., `WRK`)
- [ ] task-golem has a git module that can stage its own files and commit all currently-staged changes with a caller-provided message
- [ ] phase-golem depends on task-golem crate for all task CRUD and persistence
- [ ] phase-golem's `BacklogItem` fields are fully representable via task-golem `Item` + `x-pg-*` extensions
- [ ] Status mapping works correctly: New/Scoping/Ready -> Todo + `x-pg-status`, InProgress -> Doing, Done -> Done, Blocked -> Blocked
- [ ] `blocked_from_status` preserved at full fidelity via `x-pg-blocked-from-status` extension (task-golem's native 4-variant `blocked_from_status` is insufficient for phase-golem's 6-variant model)
- [ ] Adapter layer provides typed access to extensions (no raw JSON manipulation in scheduler/executor)
- [ ] `backlog.rs` and `migration.rs` are deleted from phase-golem
- [ ] Inbox mechanism is removed; `tg add` is the way to add items
- [ ] Coordinator remains a thin async actor for write serialization, delegates all storage to task-golem, reads backlog state through task-golem store (no in-memory backlog cache), retains in-memory `pending_batch_phases` for non-destructive commit accumulation
- [ ] Non-destructive phase batching still works — non-destructive phases (e.g., research, design) can be batched: coordinator accumulates staged files + phase metadata, flushes when ready
- [ ] Destructive phase exclusivity still works — destructive phases (e.g., build) require exclusive execution and commit immediately upon completion
- [ ] Scheduler produces equivalent scheduling decisions (`select_actions()` parameter type changes from `BacklogFile` to adapted items, but scheduling logic and output are equivalent)
- [ ] Existing phase-golem tests pass (adapted to new storage layer and types)
- [ ] New IDs use hex format (e.g., `WRK-a1b2c`)
- [ ] task-golem store calls from async phase-golem code are wrapped in `spawn_blocking` (task-golem's `with_lock()` is synchronous/blocking)
- [ ] Follow-up ingestion pipes `PhaseResult.follow_ups` through the adapter to task-golem — coordinator creates new task-golem items via `generate_id_with_prefix("WRK")` with appropriate `x-pg-*` extensions
- [ ] Item merge retained via adapter layer — `MergeItem` combines two items using task-golem's store API (append descriptions, merge dependencies, remove source item)
- [ ] Item archival delegates to task-golem's `append_to_archive()` — scheduler treats dependencies on archived/missing items as satisfied (stale dependency refs are harmless)
- [ ] Phase-golem adopts a proper error enum for coordinator/adapter errors, mapping `TgError` variants to explicit categories (retryable vs. fatal)
- [ ] Adapter populates task-golem's native `description` field with the `context` field from `StructuredDescription` (provides useful display for `tg show`)
- [ ] Bidirectional status mapping: phase-golem → task-golem as specified above; task-golem → phase-golem: if Todo, check `x-pg-status` (absent defaults to `New` — item enters pipeline at triage); if Doing/Done/Blocked, map directly

### Should Have

- [ ] `tg list`, `tg show`, `tg ready` work on the migrated data (human ergonomics)

### Nice to Have

- [ ] task-golem's `claimed_by`/`claimed_at` fields used to track which phase-golem executor is working on an item
- [ ] task-golem's `priority` field used to influence scheduling order

## Scope

### In Scope

- **task-golem**: Add `lib.rs`, add git module (stage + commit), expose `generate_id_with_prefix()`, ensure `model`/`store`/`errors` are publicly usable
- **phase-golem**: Add task-golem dependency, create adapter layer (`pg_item.rs`), simplify coordinator to use task-golem store, delete `backlog.rs` and `migration.rs`, remove inbox mechanism
- **Extension schema**: Define the `x-pg-*` extension field mapping for all phase-golem-specific data (including `x-pg-blocked-from-status` to handle the status enum width mismatch)
- **Worklog**: Phase-golem retains its own worklog writes (`_worklog/YYYY-MM.md`); task-golem's archive (`archive.jsonl`) handles item storage after completion; these serve different purposes and coexist

### Out of Scope

- Migration of existing items (WRK-001 through WRK-075) — existing backlog is stale due to overhaul; fresh start with task-golem store
- Multi-agent/worktree parallelism (future work; architecturally compatible)
- task-golem CLI changes (existing `tg` commands already work)
- Changes to phase-golem's scheduler logic, executor, prompt builder, or agent runner (parameter types change but logic is equivalent)
- task-golem MCP server or other integration interfaces
- Separate git branch for task state (Beads-style isolation — storing task state on a separate orphan git branch rather than in the working tree)
- Preventing `tg` CLI from making state changes that violate phase-golem's state machine (accepted risk; human operator responsibility)

## Non-Functional Requirements

- **Concurrency:** task-golem's file locking must prevent corruption when phase-golem and a human both use `tg` simultaneously. Phase-golem must wrap synchronous `with_lock()` calls in `spawn_blocking` to avoid blocking the Tokio runtime.
- **Performance:** Read-through (loading from disk on every read rather than caching in memory) to JSONL store must not regress scheduler loop cycle time for backlogs under 200 items (current backlog is tens of items; JSONL parse at this scale is sub-millisecond)
- **Data integrity:** Atomic writes via temp-file-rename pattern (already implemented in task-golem)

## Constraints

- task-golem must remain independently useful as a CLI tool; the library exposure must not break existing `tg` binary behavior
- phase-golem's pipeline orchestration logic (scheduler, executor, prompt builder) must remain functionally unchanged (parameter types may change but behavior is equivalent)
- The two projects live in separate repositories (`Code/task-golem`, `Code/phase-golem`); dependency is via path or git
- task-golem is Rust 2024 edition; both crates require rustc >= 1.85
- `.task-golem/` directory must exist before phase-golem can use the store. User runs `tg init` as a prerequisite (phase-golem does not auto-initialize the store)

## Dependencies

- **Depends On:**
  - task-golem `lib.rs` must exist before phase-golem can depend on it (Part 1 before Part 2)
  - task-golem git module must exist before phase-golem can delegate commits (Part 1 before Part 2)
- **Blocks:**
  - Future multi-agent worktree parallelism (architecturally compatible but not implemented here)
  - Other tools/agents using task-golem as a shared substrate

## Risks

- [ ] **Extension field ergonomics** — Storing ~15 fields as `x-pg-*` extensions may be verbose. Mitigation: the adapter layer abstracts this away; internal code never touches raw extensions.
- [ ] **Status model mismatch** — Phase-golem's 6-state model mapped onto task-golem's 4-state model via extensions could cause confusion when using `tg` CLI directly, and a human could make transitions via `tg` that violate phase-golem's state machine (e.g., `tg do` on a `New` item skips Scoping/Ready). Mitigation: this is accepted as a human operator responsibility; phase-golem's scheduler will skip items in inconsistent states. Document valid `tg` operations for phase-golem items.
- [ ] **StructuredDescription as JSON extension** — The 5-field structured description stored as a JSON object in `x-pg-description` is less human-readable via `tg show` (displays as raw JSON with alphabetically-ordered keys). Mitigation: phase-golem's own status/display commands can format it nicely.
- [ ] **Coordinator simplification** — Coordinator remains a thin async actor for write serialization but delegates all storage to task-golem. Retains `pending_batch_phases` as in-memory state; task-golem's `with_lock()` provides additional storage-level locking; `spawn_blocking` bridges async/sync boundary.
- [ ] **Cross-repo coordination** — Changes span two repos; must be sequenced correctly. Mitigation: Part 1 (task-golem) is independently useful and ships first.
- [ ] **`blocked_from_status` type width** — task-golem's native `blocked_from_status` is `Option<Status>` (4 variants), insufficient for phase-golem's 6-variant `ItemStatus`. Mitigation: adapter uses `x-pg-blocked-from-status` extension for the full-fidelity value; task-golem's native field may hold a lossy mapping but is not authoritative for phase-golem.
- [ ] **Lock timeout under concurrent load** — task-golem's `with_lock()` has a 5-second hard timeout. If a human runs a slow `tg` command while multiple phase-golem executors complete simultaneously, some writes could get `LockTimeout`. Mitigation: phase-golem serializes write operations through the coordinator's actor; on `LockTimeout`, coordinator retries up to 3 times with 1-second backoff before treating as transient error (scheduler retries on next loop iteration).
- [ ] **`blocked_from_status` divergence via CLI** — If a human runs `tg block` on a Todo item, task-golem stores the native 4-variant `blocked_from_status` but cannot update `x-pg-blocked-from-status`. On `tg unblock`, task-golem clears native `blocked_from_status` but leaves the stale extension. Mitigation: adapter detects divergence (native field cleared but extension present) and falls back to lossy native value with warning log. Accepted as a rare edge case since blocking is typically orchestrator-initiated.
- [ ] **Corrupted `tasks.jsonl`** — If the JSONL file is corrupted (truncated write, manual edit error), task-golem's store fails fast on load and phase-golem cannot start. Mitigation: JSONL is source of truth but is also committed to git, so previous state is recoverable via `git checkout .task-golem/tasks.jsonl`. Phase-golem logs a clear error pointing to this recovery path.
- [ ] **`serde_yaml` version mismatch** — task-golem depends on `serde_yaml = "0.9"` (deprecated); phase-golem uses `serde_yaml_ng = "0.10"`. Both compile as separate crates (no type conflicts since YAML types don't cross the boundary), but adds binary bloat. Can be cleaned up in a separate task-golem hygiene pass.

## Decisions

- **Explicit `commit()` API** — task-golem's git module exposes a `commit(message)` method; callers invoke it when ready. No auto-commit on state change. This gives phase-golem full control over batching.
- **Path dependency** — phase-golem depends on task-golem via local path for now. Can switch to git dependency later.
- **Keep `phase-golem.lock`** — Phase-golem retains its process-level lock. It prevents multiple orchestrator instances, which is a separate concern from task-golem's storage-level file lock.
- **JSONL is source of truth** — If `tasks.jsonl` is updated but `git commit` fails, the JSONL state is authoritative. Git is best-effort logging. Commit failure is logged but not fatal.
- **Retain thin actor pattern** — Coordinator remains an async actor with mpsc channel for write serialization. This provides strong concurrency guarantees without relying solely on task-golem's file lock. The actor delegates all storage operations to task-golem but serializes access from phase-golem's side.
- **Error enum adoption** — Phase-golem adopts a proper error enum in the coordinator/adapter layer. `TgError` variants are mapped to explicit phase-golem error categories (e.g., `LockTimeout` → retryable, `StorageCorruption` → halt, `ItemNotFound` → log and skip). No more `Result<T, String>` in coordinator paths.
- **Status default for new items** — Items created via `tg add` (Todo status, no `x-pg-status` extension) default to `New` in phase-golem's model. They enter the pipeline at the beginning and are triaged like any other new item (human or agent-created).
- **No migration of existing items** — The existing backlog (WRK-001 through WRK-075) is stale due to the overhaul. Fresh start with task-golem store rather than migrating old data.
- **Native description populated** — Adapter populates task-golem's native `Item.description` with the `context` field from `StructuredDescription`, giving `tg show` useful human-readable output.
- **Convergent ID prefixes** — All item creation goes through task-golem with a configured prefix. Old items may have different prefixes but the goal is eventual consistency.
- **Git module stays narrow** — task-golem's git module ONLY stages its own files (`stage_self()`) and commits all currently-staged changes (`commit(message)`). It does NOT stage arbitrary files, resolve conflicts, or manage branches.
- **Dependency cleanup on archive** — Scheduler treats dependencies on archived/missing items as satisfied. No explicit dependency stripping on archive; stale dependency refs are harmless.
- **LockTimeout retry** — Coordinator retries `LockTimeout` errors up to 3 times with 1-second backoff. If still failing, treat as transient error and let the scheduler retry on next loop iteration. Other `TgError` variants (e.g., `StorageCorruption`) are not retried.
- **Commit message format preserved** — Phase-golem continues generating commit messages in the existing format (e.g., `[WRK-a1b2c][build] Add login form`). The message is passed to task-golem's `commit(message)` as-is.
- **`spawn_blocking` panic handling** — If task-golem code panics inside `spawn_blocking`, the `JoinError` is treated as a fatal coordinator error (logged and propagated). Panics in storage code indicate a bug, not a transient condition.
- **`prune_stale_dependencies` removed** — No longer needed. The scheduler treats dependencies on missing/archived items as satisfied, making stale dependency refs harmless. The explicit pruning logic in `backlog.rs` is deleted along with the rest of that module.

## Open Questions

*(None remaining — all directional decisions resolved via critique triage.)*

## Implementation Notes

This change should be split into two sequential work items:

### Part 1: task-golem Library + Git Module
- Add `lib.rs` exposing `model`, `store`, `errors`
- Add `[lib]` section to `Cargo.toml`
- Expose `generate_id_with_prefix()` in the public API
- Add git module with `stage_self()` (stages `.task-golem/tasks.jsonl` and `.task-golem/archive.jsonl`) and `commit(message)` (commits all currently-staged changes) functions — narrow scope, no arbitrary file staging
- Verify existing `tg` binary still works
- No changes to existing CLI commands or behavior

### Part 2: Phase-Golem Storage Migration
- Add `task_golem` as path dependency
- Create `pg_item.rs` adapter: typed extension access, bidirectional status mapping (absent `x-pg-status` on Todo defaults to `New`), `StructuredDescription` serde for `x-pg-description`, native `description` populated with `context` field
- Introduce phase-golem error enum mapping `TgError` variants to actionable categories (retryable/fatal/skip)
- Thin coordinator: retain async actor pattern for write serialization, remove in-memory backlog, keep `pending_batch_phases`; read-through via task-golem store; wrap `with_lock()` calls in `spawn_blocking`
- Rewire follow-up ingestion: pipe `PhaseResult.follow_ups` through adapter to create task-golem items via `generate_id_with_prefix("WRK")` with `x-pg-*` extensions
- Retain item merge via adapter: `MergeItem` uses task-golem store API to combine items
- Rewire archival: delegate to task-golem's `append_to_archive()`; scheduler treats deps on archived/missing items as satisfied; retain worklog write separately
- Rewire git commit logic: phase-golem stages artifact files via `git add`, then calls task-golem to update item state (task-golem stages `tasks.jsonl`), then task-golem commits all staged changes. Steps must not be interleaved with other git operations.
- Delete `backlog.rs`, `migration.rs`, inbox handling
- Retain worklog writes (`_worklog/`) — these are phase-golem-specific logging, separate from task-golem's archive
- Remove `backlog_path` from `ProjectConfig` (no longer needed; store location is the `.task-golem/` directory)
- Update `select_actions()` and related functions to accept adapter types instead of `BacklogFile`; remove `BacklogFile` and `BacklogItem` types
- Adapter validates extension field values on deserialization: invalid values (e.g., `x-pg-status: "running"`) treated as absent with warning log, not panics
- ID generation via `generate_id_with_prefix("WRK")` requires loading all known IDs (active + archive) for collision avoidance. At current scale (tens of items) this is negligible.
- Update/adapt tests
- No migration of existing data — fresh start with task-golem store

### `with_lock()` Mutation Pattern

All read-modify-write operations on task-golem's store happen inside a single `with_lock()` closure to prevent TOCTOU races. The canonical pattern:

```
store.with_lock(|store| {
    let mut items = store.load_active()?;       // read inside lock
    let item = find_and_mutate(&mut items, id);  // mutate local copy
    store.save_active(&items)?;                  // write inside lock
    Ok(result)
})
```

The coordinator's actor serialization provides an additional layer of protection from phase-golem's side. `pending_batch_phases` lives on the coordinator struct outside the lock — it is only accessed through the actor's mpsc channel.

### Extension Field Schema

| Phase-golem field | Extension key | Value type | Notes |
|---|---|---|---|
| status (sub-state) | `x-pg-status` | `"new"`, `"scoping"`, `"ready"` | Absent when task-golem status is Doing/Done/Blocked. Reverse mapping: if Todo + absent → defaults to `New` (enters triage) |
| phase | `x-pg-phase` | `"prd"`, `"build"`, etc. | |
| phase_pool | `x-pg-phase-pool` | `"pre"`, `"main"` | |
| size | `x-pg-size` | `"small"`, `"medium"`, `"large"` | |
| complexity | `x-pg-complexity` | `"low"`, `"medium"`, `"high"` | |
| risk | `x-pg-risk` | `"low"`, `"medium"`, `"high"` | |
| impact | `x-pg-impact` | `"low"`, `"medium"`, `"high"` | |
| requires_human_review | `x-pg-requires-human-review` | `true` / `false` | |
| pipeline_type | `x-pg-pipeline-type` | `"feature"`, etc. | |
| origin | `x-pg-origin` | Item ID string | |
| blocked_type | `x-pg-blocked-type` | `"clarification"`, `"decision"` | |
| blocked_from_status | `x-pg-blocked-from-status` | `"new"`, `"scoping"`, `"ready"`, `"in_progress"` | Authoritative; only 4 of 6 `ItemStatus` variants are valid here (cannot block from Done or Blocked). task-golem's native field holds lossy 4-variant mapping |
| unblock_context | `x-pg-unblock-context` | String | |
| last_phase_commit | `x-pg-last-phase-commit` | Git SHA | |
| description | `x-pg-description` | JSON object: `{context, problem, solution, impact, sizing_rationale}` | Keys display alphabetically in `tg show`; native `Item.description` populated with `context` field for CLI readability |
| tags | *(native)* | `Vec<String>` | Maps directly to task-golem's native `tags` field |
| blocked_reason | *(native)* | `Option<String>` | Maps directly to task-golem's native `blocked_reason` field |
| dependencies | *(native)* | `Vec<String>` | Maps directly; mixed ID formats (sequential + hex) coexist |
| created/updated | *(native)* | `DateTime<Utc>` | task-golem uses chrono `DateTime<Utc>` natively; adapter exposes as `DateTime<Utc>` (no more String timestamps) |

## References

- task-golem source: `/home/sirhamy/Code/task-golem`
- phase-golem source: `/home/sirhamy/Code/phase-golem`
- Beads (distributed git-backed issue tracker): https://github.com/steveyegge/beads — Informed the git ownership decision; chose simpler in-repo model over Beads-style branch isolation
- Research on agent task managers (Backlog.md, git-bug, dstask, GitHub Agentic Workflows) informed the "commit on state change" pattern
