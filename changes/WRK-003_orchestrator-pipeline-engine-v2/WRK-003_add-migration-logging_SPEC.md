# SPEC: Add Logging When Auto-Migrating BACKLOG.yaml v1 to v2

**ID:** WRK-003
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-003_add-migration-logging_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

When the orchestrator auto-migrates a v1 `BACKLOG.yaml` to v2, the migration runs silently — rewriting the file on disk with mapped statuses, new fields, and a bumped schema version. Users have no visibility into what changed. This SPEC adds `log_info!`, `log_warn!`, and `log_debug!` calls to the existing `migrate_v1_to_v2()` function in `src/migration.rs` so users can see exactly what changed during migration.

The migration code and logging infrastructure both already exist. This is purely an observability addition — no behavioral changes, no new types, no new modules.

## Approach

Modify the single function `migrate_v1_to_v2()` in `src/migration.rs` to add logging at four points in its control flow:

1. **Migration start** — `log_info!` with file path and item count after v1 parse succeeds
2. **Per-item changes** — Replace the `.map(map_v1_item).collect()` pattern (line 199) with a for-loop that calls `map_v1_item()`, compares v1 vs v2 fields, and logs differences. `map_v1_item()` remains unchanged as a pure function.
3. **Summary** — `log_info!` with cumulative counts (status changes, phase clears, unchanged)
4. **Migration complete** — `log_info!` after the atomic file write succeeds

Status names use `{:?}` (Debug format) which produces PascalCase variant names matching the PRD examples exactly (`Researching`, `Scoping`, etc.). Change detection compares `format!("{:?}", v1_status) != format!("{:?}", v2_status)`.

**Patterns to follow:**

- `.claude/skills/changes/orchestrator/src/log.rs:42-74` — `log_info!`, `log_warn!`, `log_debug!` macro usage (format-string style, `eprintln!` wrappers with level gating)
- `.claude/skills/changes/orchestrator/src/migration.rs:108-166` — `map_v1_item()` pure function (not modified; logging happens in caller)

**Implementation boundaries:**

- Do not modify: `map_v1_item()`, `map_v1_status()`, or any types
- Do not modify: atomic write logic (lines 206-227)
- Do not modify: early-return path for v2 files (lines 187-192) — no logging on this path
- Do not modify: error paths — no logging on `Result::Err` propagation (out of scope per PRD)
- Do not add: new types, traits, modules, or external dependencies
- Do not add: tests for log output (out of scope per PRD — log macros write to stderr)

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Add migration logging | Low | Add all logging to `migrate_v1_to_v2()`: start log, for-loop with per-item logs, summary, completion log |

**Ordering rationale:** Single phase — all changes are in one function with no dependencies between them. Splitting into multiple phases would create unnecessary overhead for ~60 lines of changes in a single file.

---

## Phases

### Phase 1: Add migration logging

> Add all logging calls to `migrate_v1_to_v2()`: start log, for-loop with per-item change detection and logging, summary counts, and completion log

**Phase Status:** complete

**Complexity:** Low

**Goal:** Add observability to the v1→v2 migration so users see what changed when their backlog file is auto-migrated.

**Files:**

- `.claude/skills/changes/orchestrator/src/migration.rs` — modify — restructure `migrate_v1_to_v2()` item mapping from `.map().collect()` to for-loop with logging

**Tasks:**

- [x] Add `log_info!` after v1 parse (line 196) to log migration start with path and item count: `"Migrating BACKLOG.yaml v1 → v2: {path} ({n} items)"`
- [x] Initialize summary counters before the for-loop: `status_changes`, `phase_clears`, `unchanged` (all `usize`, start at 0)
- [x] Replace `let items: Vec<BacklogItem> = v1.items.iter().map(map_v1_item).collect();` (line 199) with a for-loop that:
  - Calls `map_v1_item(v1_item)` to get the v2 item
  - Compares `format!("{:?}", v1_item.status)` vs `format!("{:?}", v2_item.status)` — if different, `log_info!("  {id}: status {v1_status} → {v2_status}")` and increment `status_changes`
  - Checks if phase was cleared: use `if let Some(ref old_phase) = v1_item.phase` combined with `v2_item.phase.is_none()` — if both true, `log_info!("  {id}: phase cleared (was '{}')", old_phase.as_str())` and increment `phase_clears`. Use `if let` (not `unwrap()`) for idiomatic Rust.
  - Checks `blocked_from_status`: if both v1 and v2 have it and `format!("{:?}", v1_blocked) != format!("{:?}", v2_blocked)`, `log_warn!("  {id}: blocked_from_status mapped {v1_blocked} → {v2_blocked}")`
  - Emits `log_debug!` with full field mapping using `{:?}` for all fields: `"  {id}: v1{{status:{:?}, phase:{:?}}} → v2{{status:{:?}, phase:{:?}, phase_pool:{:?}, pipeline_type:{:?}}}"`
  - Track per-item booleans `let status_changed = ...` and `let phase_cleared = ...` for the comparisons above, then increment `unchanged` only when `!status_changed && !phase_cleared`
  - Pushes v2 item to result vec
- [x] Add summary `log_info!` after the for-loop: `"Migrated {n} items: {status_changes} status changes, {phase_clears} phase cleared, {unchanged} unchanged"`
- [x] Add completion `log_info!` after the atomic write succeeds (after line 227): `"Migration complete: {path}"`
- [x] Run `cargo test --test migration_test` to verify all 9 existing tests pass
- [x] Run `cargo build` to verify compilation

**Verification:**

- [x] `cargo test --test migration_test` — all 10 tests pass (migration behavior unchanged)
- [x] `cargo build` succeeds without warnings
- [x] Code review: verify start log uses `path.display()` and `v1.items.len()`
- [x] Code review: verify per-item status change logs use `{:?}` format and only fire when Debug names differ
- [x] Code review: verify phase-cleared log only fires when `v1_item.phase.is_some() && v2_item.phase.is_none()`
- [x] Code review: verify `blocked_from_status` warning only fires when Debug names differ (not identity mappings)
- [x] Code review: verify completion log is placed after `temp_file.persist(path)?` (not before)
- [x] Code review: verify no logging on early-return path (schema_version >= 2)
- [x] Code review: verify no logging on error paths
- [x] Code review: verify summary counts are cumulative (item with both status change and phase clear increments both counters; "unchanged" = neither)

**Commit:** `[WRK-003][P1] Feature: Add logging when auto-migrating BACKLOG.yaml v1 to v2`

**Notes:**

- The `V1WorkflowPhase` enum has `as_str()` returning the phase name string — use this for the phase-cleared log message
- Debug format for `V1WorkflowPhase` gives PascalCase (`Research`, `Prd`); `as_str()` gives lowercase (`research`, `prd`). The PRD example uses lowercase: `phase cleared (was 'research')` — use `as_str()`
- The `blocked_from_status` comparison works identically to the status comparison — both use Debug string equality
- Summary counter semantics: `status_changes` counts items where v1 status Debug name differs from v2; `phase_clears` counts items where v1 had a phase and v2 has `None`; `unchanged` counts items where neither happened. An item can be counted in both `status_changes` and `phase_clears` (e.g., a `Researching` item with a phase increments both). `unchanged` counts items where BOTH conditions are false. The sum `status_changes + phase_clears + unchanged` may exceed total items (overlap) or `status_changes + unchanged` may be less than total (items counted in `phase_clears` only). Concrete example using `backlog_v1_full.yaml` (5 items): WRK-001 (InProgress, no change) → unchanged; WRK-002 (Done, no change) → unchanged; WRK-003 (Blocked, blocked_from_status changes but status itself is identity) → unchanged; WRK-004 (Researching+research → Scoping, phase cleared) → status_changes AND phase_clears; WRK-005 (Scoped → Ready) → status_changes. Expected summary: `Migrated 5 items: 2 status changes, 1 phase cleared, 3 unchanged`.

**Known limitations:**

- If migration fails after the start log but before completion (e.g., I/O error during atomic write), users will see "Migrating..." but no "Migration complete" — this is correct behavior (the absence of completion log plus the error message from `Result::Err` propagation indicates failure). Error path logging is out of scope per PRD.
- No automated tests for log output — log macros write to stderr and existing migration tests validate correctness of migration behavior, not log content. This is explicitly out of scope per PRD.

**Followups:**

None identified.

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met:
  - [x] `log_info!` when migration starts: file path and item count
  - [x] `log_info!` per item where v1 status name differs from v2 status name
  - [x] `log_info!` per item where phase was cleared during migration
  - [x] `log_info!` when migration completes and file has been written
  - [x] `log_warn!` per item where `blocked_from_status` maps to differently-named status
  - [x] Uses existing log macros from `src/log.rs`
  - [x] No logging output on v2 early-return path
  - [x] Existing migration tests pass
  - [x] Completion log after atomic write succeeds (not before)
  - [x] `log_debug!` per item with full field mapping (should-have)
  - [x] Summary line with counts (nice-to-have)
- [x] Tests pass (`cargo test --test migration_test`)
- [x] No regressions introduced
- [x] Code reviewed

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | Complete | `[WRK-003][P1] Feature: Add logging when auto-migrating BACKLOG.yaml v1 to v2` | All 10 tests pass, build clean, code reviewed. Fixed redundant `phase_cleared` variable during review. |

## Followups Summary

### Critical

### High

### Medium

### Low

## Design Details

### Architecture Details

No architectural changes. The modification is contained entirely within the `migrate_v1_to_v2()` function in `src/migration.rs`. The function signature, return type, and behavior are unchanged — only side effects (log output to stderr) are added.

### Design Rationale

**For-loop over `.map().collect()`:** The current one-liner `v1.items.iter().map(map_v1_item).collect()` doesn't provide a natural point to compare v1 and v2 data. A for-loop gives access to both the v1 item and the mapped v2 item at each iteration, enabling per-item change detection and logging without a second pass.

**Debug format for status names:** `V1ItemStatus` and `ItemStatus` both derive `Debug`. The PascalCase variant names (`Researching`, `Scoping`, etc.) match the PRD's example log messages exactly. Adding `Display` impls would be more idiomatic but adds code for zero benefit since variant names ARE the desired display names.

**String comparison for change detection:** Comparing `format!("{:?}", v1_status)` with `format!("{:?}", v2_status)` catches exactly the right cases (the two renames: `Researching→Scoping`, `Scoped→Ready`) while correctly ignoring identity mappings. One allocation per comparison is negligible for a one-time migration.
