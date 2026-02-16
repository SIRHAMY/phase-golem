# Change: Add Logging When Auto-Migrating BACKLOG.yaml v1 to v2

**Status:** Proposed
**Created:** 2026-02-12
**Author:** Claude (autonomous)

## Problem Statement

When the orchestrator loads a v1 `BACKLOG.yaml` file, the migration function silently auto-migrates the file to v2 format — rewriting it on disk with mapped statuses, new fields (`pipeline_type`, `phase_pool`, `description`, `last_phase_commit`), and bumped `schema_version`. The user has no visibility into this happening. If something unexpected occurs (a status mapping the user didn't anticipate, cleared phases, new default fields), the user has no indication that a migration occurred, let alone what changed.

This is particularly concerning because the migration overwrites the original file (via atomic temp-file rename), and the status/phase mappings involve non-obvious transformations (e.g., `Researching` → `Scoping` with phase cleared, `Scoped` → `Ready`).

## User Stories / Personas

- **Solo Developer** — Runs the orchestrator after updating to a version with v2 schema support. Wants to know their backlog was migrated, how many items were affected, and whether any items had their status or phase changed in unexpected ways.

## Desired Outcome

When a v1 `BACKLOG.yaml` is auto-migrated to v2, the orchestrator logs:
1. That a migration is starting (info-level)
2. Per-item details when the v1 status maps to a differently-named v2 status (info-level)
3. Per-item details when a phase is cleared during migration (info-level)
4. That the migration completed and the file was written (info-level)

When a v2 file is loaded (schema_version >= 2), no migration logging occurs — the function returns the parsed backlog without any log output.

## Success Criteria

### Must Have

- [ ] `log_info!` when migration starts: file path and item count
  - Example: `Migrating BACKLOG.yaml v1 → v2: /path/to/BACKLOG.yaml (5 items)`
- [ ] `log_info!` per item where the v1 status name differs from the mapped v2 status name (only `Researching` → `Scoping` and `Scoped` → `Ready` in the current mapping; identity mappings like `New` → `New` are not logged)
  - Example: `  WRK-001: status Researching → Scoping`
- [ ] `log_info!` per item where phase was cleared during migration (this occurs for items with v1 status `Researching`, because v1 workflow phases don't map to v2 pre-phases — the scheduler re-assigns on next run)
  - Example: `  WRK-001: phase cleared (was 'research')`
- [ ] `log_info!` when migration completes and the file has been written to disk
  - Example: `Migration complete: /path/to/BACKLOG.yaml`
- [ ] `log_warn!` per item where `blocked_from_status` maps to a differently-named status (e.g., a blocked item whose `blocked_from_status` was `Scoped` now has `blocked_from_status: Ready`)
  - Example: `  WRK-003: blocked_from_status mapped Scoped → Ready`
- [ ] Logging uses the existing `log_info!` / `log_warn!` macros from `src/log.rs` (which output to stderr, filtered by the configured log level) — no new logging infrastructure
- [ ] No logging output when `migrate_v1_to_v2()` is called on an already-v2 file (the early-return path at schema_version >= 2 produces no log output)
- [ ] Existing migration tests continue to pass
- [ ] Completion log is emitted only after the atomic file write succeeds (not before)

### Should Have

- [ ] `log_debug!` per item showing the full before/after field mapping (verbose, only visible at `--log-level=debug`)
  - Example: `  WRK-001: v1{status:researching, phase:research} → v2{status:scoping, phase:None, phase_pool:None, pipeline_type:feature}`

### Nice to Have

- [ ] Summary line at info-level counting items by change type
  - Example: `Migrated 5 items: 2 status changes, 1 phase cleared, 2 unchanged`
  - "status changed" = v1 status name differs from v2 status name
  - "phase cleared" = item had a phase in v1 that was set to None in v2
  - "unchanged" = neither status name nor phase changed
  - Items can appear in multiple categories (e.g., status changed AND phase cleared)

## Scope

### In Scope

- Adding `log_info!`, `log_warn!`, and `log_debug!` calls to `migrate_v1_to_v2()` in `src/migration.rs`
- Comparing v1 item values against mapped v2 values within the migration function to determine what changed (the v1 `V1BacklogItem` is available alongside the mapped `BacklogItem`)

### Out of Scope

- Changing migration logic or status mappings
- Adding new log levels or logging infrastructure
- Logging in `backlog::load()` (the migration function is the right place since `load()` just delegates)
- Structured logging (JSON format, log files) — stderr via `eprintln!` is the current pattern
- Migration rollback or backup functionality
- Tests that verify log output (log macros write to stderr; the existing migration tests validate correctness of the migration itself)
- Logging for `PhaseResult` v1-era result file handling — only the BACKLOG.yaml migration is covered
- Logging when migration fails (migration errors are already returned as `Result::Err` and propagated to the caller, which handles error reporting)

## Non-Functional Requirements

- **Observability:** All migration logging goes to stderr via the existing log macros, respecting the configured log level. At `--log-level=warn` or below, only the `blocked_from_status` warning will be visible. Users should run with at least `--log-level=info` (the default) to see migration details.

## Constraints

- Must use the existing `log_info!` / `log_warn!` / `log_debug!` macros — no new dependencies
- Must not change migration behavior, only add observability
- Log messages must include item IDs for traceability
- Log only non-sensitive metadata (item ID, status, phase) — not titles or descriptions

## Dependencies

- **Depends On:** None — the migration code and logging infrastructure both already exist
- **Blocks:** Nothing

## Risks

- [ ] **Log noise on first run after upgrade** — A backlog with many items will produce many info-level log lines during migration. Mitigation: this is a one-time event per backlog file; info-level is appropriate for a destructive file operation. Verbose per-item field details are at debug level only.

## Assumptions

- The existing PRD/DESIGN/SPEC in this change folder cover the larger v2 pipeline engine work. This PRD covers only the focused logging addition for the migration path, which is what WRK-003's backlog title specifies. The migration logic itself was already implemented and is not changed here.
- No interview phase is needed — the requirements are self-evident from the code.
- The orchestrator runs in a user-visible context (interactive CLI or logged process output) where stderr is observable.
- `#[macro_export]` macros from `log.rs` are available crate-wide without explicit `use` imports (standard Rust macro behavior).

## Open Questions

None — the scope is fully defined by the existing migration code and logging infrastructure.

## References

- Migration function: `src/migration.rs`, `migrate_v1_to_v2()`
- Item mapping function: `src/migration.rs`, `map_v1_item()`
- V1 status mapping: `src/migration.rs`, `map_v1_status()`
- Logging macros: `src/log.rs`
- Backlog loading (migration entry point): `src/backlog.rs`, `load()`
