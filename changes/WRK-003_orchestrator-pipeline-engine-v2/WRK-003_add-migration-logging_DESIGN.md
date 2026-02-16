# Design: Add Logging When Auto-Migrating BACKLOG.yaml v1 to v2

**ID:** WRK-003
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-003_add-migration-logging_PRD.md
**Tech Research:** ./WRK-003_add-migration-logging_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add `log_info!`, `log_warn!`, and `log_debug!` calls to the existing `migrate_v1_to_v2()` function in `src/migration.rs` so users can see what changed when their backlog file is auto-migrated. The approach replaces the functional `.map().collect()` pattern with a for-loop that logs per-item changes inline, keeping `map_v1_item()` as a pure function. Status names in log output use `{:?}` (Debug format) to produce PascalCase names matching the PRD examples. No new types, traits, or modules are introduced.

---

## System Design

### High-Level Architecture

No architectural changes. The design modifies a single function (`migrate_v1_to_v2()` in `src/migration.rs`) by inserting log macro calls at four points in the existing control flow:

```
migrate_v1_to_v2(path)
  │
  ├─ schema_version >= 2? → return Ok(backlog)  [no logging]
  │
  ├─ Parse as v1
  │
  ├─ log_info!("Migrating BACKLOG.yaml v1 → v2: {path} ({n} items)")   ← NEW
  │
  ├─ For each v1 item:                                                   ← CHANGED from .map().collect()
  │     ├─ map_v1_item(v1_item) → v2_item
  │     ├─ if status name changed: log_info!                             ← NEW
  │     ├─ if phase cleared: log_info!                                   ← NEW
  │     ├─ if blocked_from_status name changed: log_warn!                ← NEW
  │     ├─ log_debug!(full field mapping)                                ← NEW (should-have)
  │     └─ push v2_item to vec
  │
  ├─ log_info!(summary counts)                                           ← NEW (nice-to-have)
  │
  ├─ Atomic write (existing logic, unchanged)
  │
  └─ log_info!("Migration complete: {path}")                             ← NEW (after persist)
```

### Component Breakdown

#### `migrate_v1_to_v2()` — MODIFIED

**Purpose:** Migrate a v1 BACKLOG.yaml to v2 format, now with logging of what changed.

**Responsibilities (added):**
- Log migration start with file path and item count
- Log per-item status name changes (only when v1 name differs from v2 name)
- Log per-item phase clearing (only for `Researching` items that had a phase)
- Log per-item `blocked_from_status` name changes at warn level (only when the mapped v2 status has a different Debug name than the v1 status — same two cases: `Researching`→`Scoping`, `Scoped`→`Ready`; identity mappings like `Blocked` with `blocked_from_status: Ready` → `Ready` are not warned)
- Log full per-item field mapping at debug level (format: `  {id}: v1{{status:{v1_status}, phase:{v1_phase}}} → v2{{status:{v2_status}, phase:{v2_phase}, phase_pool:{pool}, pipeline_type:{type}}}`)
- Log summary counts at info level (nice-to-have) — counts are cumulative: an item with both a status rename and a phase clear increments both counters; "unchanged" counts items with neither a status rename nor a phase clear
- Log migration completion after atomic file write succeeds

**Interfaces:** Unchanged — `pub fn migrate_v1_to_v2(path: &Path) -> Result<BacklogFile, String>`

**Dependencies:** Existing `log_info!`, `log_warn!`, `log_debug!` macros from `src/log.rs`. These macros are `#[macro_export]` and available crate-wide without explicit `use` imports. They respect the global log level set by `set_log_level()` in `main.rs`, which is always initialized before `backlog::load()` (and thus before migration) is called.

#### `map_v1_item()` — UNCHANGED

Remains a pure function. All logging happens in the caller (`migrate_v1_to_v2`) by comparing v1 item fields against the mapped v2 item fields.

### Data Flow

1. `migrate_v1_to_v2()` reads and parses v1 BACKLOG.yaml (existing)
2. **NEW:** Logs migration start with path and item count
3. For each v1 item, calls `map_v1_item()` to get the v2 item (existing, restructured from `.map()` to for-loop)
4. **NEW:** Compares v1 vs v2 fields and logs changes
5. Builds v2 `BacklogFile`, serializes, performs atomic write (existing)
6. **NEW:** Logs migration completion after successful persist

### Key Flows

#### Flow: Migration with Logging

> User runs orchestrator after upgrading to v2 schema. Backlog has 5 items, 2 with renamed statuses, 1 with a cleared phase.

1. **Read and parse** — Existing file read and v1 parse logic
2. **Start log** — `log_info!("Migrating BACKLOG.yaml v1 → v2: /path/to/BACKLOG.yaml (5 items)")`
3. **Per-item loop** — For each of the 5 items:
   - Call `map_v1_item()` to get v2 item
   - Item with `Researching` status: `log_info!("  WRK-001: status Researching → Scoping")` and `log_info!("  WRK-001: phase cleared (was 'research')")`
   - Item with `Scoped` status: `log_info!("  WRK-002: status Scoped → Ready")`
   - Items with identity mappings (`New`→`New`, `Ready`→`Ready`, etc.): no info-level log
   - All items: `log_debug!` with full field mapping
4. **Summary** — `log_info!("Migrated 5 items: 2 status changes, 1 phase cleared, 2 unchanged")`
5. **Atomic write** — Existing temp-file-rename logic (unchanged)
6. **Completion log** — `log_info!("Migration complete: /path/to/BACKLOG.yaml")`

**Edge cases:**
- Empty backlog (0 items) — Start log shows `(0 items)`, no per-item logs, summary shows `Migrated 0 items: 0 status changes, 0 phase cleared, 0 unchanged`, completion log emitted
- Already v2 — Early return at `schema_version >= 2` check. No log output at any level (not even debug). Error handling for v2 parse failures uses the existing `Result::Err` propagation to the caller — no log calls on this path (PRD: "Logging when migration fails" is out of scope)
- Blocked item with `blocked_from_status: Scoped` — `log_warn!("  WRK-003: blocked_from_status mapped Scoped → Ready")`
- Blocked item with `blocked_from_status: Ready` — No warn emitted (identity mapping: `Ready`→`Ready`, Debug names match)
- `Researching` item with no phase (`phase: None`) — Status change is logged (`Researching → Scoping`) but no phase-cleared log (there was no phase to clear)
- Migration failure (I/O error, parse error) — Existing `Result::Err` propagation; no log calls on error paths (out of scope per PRD)

---

## Technical Decisions

### Key Decisions

#### Decision: For-Loop Replacing `.map().collect()`

**Context:** Per-item logging requires access to both v1 and v2 item data at each iteration. The current `.map(map_v1_item).collect()` pattern produces v2 items without a natural logging point.

**Decision:** Replace the one-line `.map().collect()` with a for-loop that calls `map_v1_item()`, logs changes, and pushes to a result vec.

**Rationale:** Single pass, natural logging point at each iteration, keeps `map_v1_item()` pure. The alternative (post-mapping zip comparison) adds a second iteration and more complexity for no benefit.

**Consequences:** Slightly more verbose code (~20 lines replacing 1 line), but straightforward and linear.

#### Decision: `{:?}` (Debug Format) for Status Names

**Context:** The PRD uses PascalCase status names (`Researching → Scoping`). Neither `V1ItemStatus` nor `ItemStatus` has a `Display` impl or `as_str()` method. Both derive `Debug`.

**Decision:** Use `{:?}` format specifier for status names in log messages.

**Rationale:** `Debug` derive produces PascalCase variant names (`Researching`, `Scoping`, `Scoped`, `Ready`), which match the PRD examples exactly. Zero additional code. Using `Debug` for user-facing log messages is mildly unconventional but pragmatic for enums whose variant names are the desired display names.

**Consequences:** If enum variant names ever diverge from desired display names, a `Display` impl would be needed. For now, the variant names match perfectly.

#### Decision: Change Detection via Debug String Comparison

**Context:** Need to determine if a v1 status maps to a differently-named v2 status. The two enums (`V1ItemStatus` and `ItemStatus`) are separate types with no shared trait.

**Decision:** Compare `format!("{:?}", v1_status)` with `format!("{:?}", v2_status)`. If they differ, the status was renamed.

**Rationale:** Simple, correct, and catches exactly the right cases. The known rename cases are `Researching`→`Scoping` and `Scoped`→`Ready`. All identity mappings (`New`→`New`, `Ready`→`Ready`, etc.) produce equal Debug strings. Hardcoding the two known cases would also work but is less maintainable if mappings change.

**Consequences:** One string allocation per item for each comparison (negligible — migration runs once). If a future v1 status has a different Debug name than its v2 equivalent but shouldn't be logged, this would over-report — but that scenario doesn't exist and is unlikely.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Debug format for user-facing logs | Slightly unconventional | Zero additional code, exact PRD match | Enum variant names ARE the desired display names |
| String comparison for change detection | Allocation per comparison | Generic detection of any rename | Allocation cost is negligible for a one-time migration |
| For-loop over functional style | More verbose | Natural per-item logging point | Readability and simplicity win for this use case |

---

## Alternatives Considered

### Alternative: Post-Mapping Zip Comparison

**Summary:** Keep `.map(map_v1_item).collect()` unchanged, then iterate `v1.items.iter().zip(v2_items.iter())` in a separate pass to detect and log changes.

**How it would work:**
- Map all items in one pass (existing code unchanged)
- Iterate zipped v1/v2 items in a second pass for logging

**Pros:**
- Keeps the mapping line unchanged
- Separates logging from mapping more clearly

**Cons:**
- Second iteration over all items
- More code (zip setup, iteration, same comparison logic)
- Logging is still coupled to field knowledge — no real separation benefit

**Why not chosen:** The for-loop is simpler, single-pass, and equally clear. The "separation" benefit is illusory since the logging code needs the same field knowledge either way.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Log noise on first run with many items | Many info-level lines on first migration | Low (one-time event) | This is appropriate: migration is a file-rewriting operation and users should see what changed. Verbose field details are debug-only. |

---

## Integration Points

### Existing Code Touchpoints

- `src/migration.rs:174-230` (`migrate_v1_to_v2()`) — Restructure item mapping from `.map().collect()` to for-loop with logging. Add 4 log insertion points.

### External Dependencies

None — uses only existing `log_info!`, `log_warn!`, `log_debug!` macros.

---

## Open Questions

None — scope is fully defined by the PRD and the existing code structure.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements (start log, per-item status/phase/blocked_from_status logs, completion log, summary, debug-level full mapping)
- [x] Key flows are documented (migration with logging flow, edge cases)
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Initial design draft | Light mode design for adding log calls to migrate_v1_to_v2(). For-loop approach, Debug format for status names, no new types or modules. |
| 2026-02-12 | Self-critique + auto-fix | 7 critic agents ran. ~48 raw issues reduced to 5 deduplicated concerns after cross-agent synthesis. Auto-fixed 5 items: clarified summary counting logic (cumulative), specified blocked_from_status logging condition, added debug-level log format example, documented early-return path behavior, added log level initialization note. 2 quality items noted (Display impl vs Debug, explicit match vs string comparison) — both are valid quality improvements but not directional; current approach is correct and matches PRD. Status → Complete. |
