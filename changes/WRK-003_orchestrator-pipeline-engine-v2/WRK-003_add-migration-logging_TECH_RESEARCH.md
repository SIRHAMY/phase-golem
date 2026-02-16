# Tech Research: Add Logging When Auto-Migrating BACKLOG.yaml v1 to v2

**ID:** WRK-003
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-003_add-migration-logging_PRD.md
**Mode:** Light

## Overview

Researching the implementation approach for adding `log_info!`, `log_warn!`, and `log_debug!` calls to the existing `migrate_v1_to_v2()` function in `src/migration.rs`. The function already works correctly — this change adds observability so users know when their backlog file is silently rewritten. The scope is narrow: no new infrastructure, no behavioral changes, just inserting log macro calls at the right points in an existing function.

## Research Questions

- [x] What data is available at each point in `migrate_v1_to_v2()` for logging?
- [x] Which v1 statuses map to differently-named v2 statuses?
- [x] When are phases cleared during migration, and what data is available to log the clearing?
- [x] How does `blocked_from_status` get mapped, and what data is available to detect a name change?
- [x] Can per-item logging be done within the existing `map_v1_item()` or does the function need restructuring?
- [x] Do the existing `log_info!`/`log_warn!`/`log_debug!` macros support the needed formatting?

---

## External Research

_Skipped — this change uses existing logging infrastructure with no external dependencies or patterns to research. The `log_info!`/`log_warn!`/`log_debug!` macros are simple `eprintln!` wrappers with level filtering. No logging frameworks, structured logging, or external standards are involved._

---

## Internal Research

### Existing Codebase State

#### `migrate_v1_to_v2()` — `src/migration.rs:174-230`

```rust
pub fn migrate_v1_to_v2(path: &Path) -> Result<BacklogFile, String>
```

**Execution flow with logging insertion points:**

| Step | Lines | What Happens | Available Data | Logging Opportunity |
|------|-------|-------------|----------------|---------------------|
| 1. Read file | 175-176 | `fs::read_to_string(path)` | `path` | — |
| 2. Check version | 178-192 | Parse YAML, check `schema_version` | `path`, `schema_version` | Early return at v2 — no logging needed |
| 3. Parse v1 | 194-196 | Deserialize to `V1BacklogFile` | `v1.items`, `path` | **Start log**: path + item count |
| 4. Map items | 199 | `v1.items.iter().map(map_v1_item).collect()` | v1 items, v2 items | **Per-item logs**: status changes, phase clears |
| 5. Build v2 | 201-204 | Construct `BacklogFile` | `backlog` | — |
| 6. Serialize | 211-212 | `serde_yaml_ng::to_string` | YAML string | — |
| 7. Atomic write | 214-227 | temp file → rename | `path` | **Completion log**: after line 227 |

**Key observation:** Step 4 uses `.map(map_v1_item)` as a pure function. Per-item logging requires either:
- **(a)** Replacing the `.map()` with a manual loop that logs before/after each item, or
- **(b)** Adding a post-mapping comparison loop that iterates `zip(v1.items, v2_items)` after the mapping to detect and log changes.

Option (b) is cleaner — it keeps `map_v1_item()` pure and separates logging concerns from mapping logic.

#### `map_v1_item()` — `src/migration.rs:108-166`

Pure function: `fn map_v1_item(v1: &V1BacklogItem) -> BacklogItem`

**No access to file path, item index, or logging context.** Adding logging here would break its purity and require threading the path through. Better to log from the caller.

**Status mapping logic (`map_v1_status()`, lines 96-106):**

| V1 Status | V2 Status | Name Changed? | Serde V1 | Serde V2 |
|-----------|-----------|---------------|----------|----------|
| `New` | `New` | No | `"new"` | `"new"` |
| `Researching` | `Scoping` | **Yes** | `"researching"` | `"scoping"` |
| `Scoped` | `Ready` | **Yes** | `"scoped"` | `"ready"` |
| `Ready` | `Ready` | No | `"ready"` | `"ready"` |
| `InProgress` | `InProgress` | No | `"in_progress"` | `"in_progress"` |
| `Done` | `Done` | No | `"done"` | `"done"` |
| `Blocked` | `Blocked` | No | `"blocked"` | `"blocked"` |

Only `Researching` and `Scoped` produce differently-named v2 statuses. These are the two cases to log at info level.

**Phase clearing logic (lines 131-137):**

Phases are cleared (set to `None`) only when `v1.status == V1ItemStatus::Researching`. The v1 phase value (e.g., `"research"`) is available from `v1.phase` before clearing. This is the value to include in the log message.

**`blocked_from_status` mapping (lines 139-140):**

```rust
let blocked_from_status = v1.blocked_from_status.as_ref().map(map_v1_status);
```

Uses the same `map_v1_status()`. A `log_warn!` is needed when the v1 `blocked_from_status` name differs from the v2 name (same two cases: `Researching`→`Scoping`, `Scoped`→`Ready`).

#### Logging macros — `src/log.rs:42-74`

All macros are `#[macro_export]` and available crate-wide without `use` imports.

| Macro | Level Gate | Output |
|-------|-----------|--------|
| `log_error!` | Always | `eprintln!` |
| `log_warn!` | `>= Warn` | `eprintln!` |
| `log_info!` | `>= Info` (default) | `eprintln!` |
| `log_debug!` | `>= Debug` | `eprintln!` |

All accept `format!`-style arguments: `log_info!("Item {}: {} → {}", id, from, to)`.

Default log level is `Info` (set via `AtomicU8` global, line 13).

#### `backlog::load()` — `src/backlog.rs:17-48`

Calls `migration::migrate_v1_to_v2(path)` when `schema_version < 2` and returns its result. The migration function is the right place for logging (not `load()`), as confirmed by the PRD.

### Reusable Components

| Component | Reusability |
|-----------|-------------|
| `log_info!`, `log_warn!`, `log_debug!` macros | Use directly — no new infrastructure needed |
| `V1ItemStatus` enum variants | Available for comparison in v1 items before mapping |
| `V1WorkflowPhase::as_str()` | Returns phase name string for logging cleared phases |
| `map_v1_status()` | Already exists — can compare v1 status vs mapped v2 status for change detection |

### Constraints from Existing Code

- `map_v1_item()` is a pure function with no logging context — per-item logging must happen in the caller (`migrate_v1_to_v2`)
- `V1ItemStatus` and `ItemStatus` are separate enums — comparing status names requires comparing string representations or matching known rename cases
- The `V1ItemStatus` enum does not have an `as_str()` method — one needs to be added (or use `format!("{:?}", status)` which gives `PascalCase`, not `snake_case`)
- The `ItemStatus` enum has serde `rename_all = "snake_case"` but no explicit `as_str()` — need a way to get the display string for logging

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Log messages use status names like `Researching` and `Scoping` (PascalCase in examples) | V1 and V2 status enums use `serde(rename_all = "snake_case")` for serialization, but the PRD examples use PascalCase display names | Need to decide: log the serde form (`researching → scoping`) or the Rust variant form (`Researching → Scoping`). PRD examples use PascalCase — implement `Display` or a helper to match. |
| `V1WorkflowPhase::as_str()` exists for logging cleared phases | Confirmed: `as_str()` returns `"prd"`, `"research"`, `"design"`, `"spec"`, `"build"`, `"review"` | Can use `v1.phase.as_ref().map(\|p\| p.as_str())` directly in the log message. |
| `log_info!` per item where v1 status name differs from v2 | Only 2 cases exist: `Researching→Scoping` and `Scoped→Ready`. All others are identity mappings. | Could hardcode the two known rename cases instead of doing generic comparison — simpler and equally correct. However, generic comparison is more maintainable if mappings change later. |
| Completion log emitted only after atomic file write succeeds | The atomic rename is on line 227 (`temp_file.persist(path)`). The `?` operator propagates errors. | Log must be placed after line 227, before `Ok(backlog)` on line 229. Straightforward. |

---

## Critical Areas

### Status Name Display for Log Messages

**Why it's critical:** The PRD specifies log messages like `WRK-001: status Researching → Scoping` (PascalCase). Neither enum has a display method that produces this form — `V1ItemStatus` has no `as_str()` at all, and `ItemStatus` only has serde rename (snake_case).

**Why it's easy to miss:** The PRD examples look simple, but reproducing them requires either: adding `Display`/`as_str()` impls to both status enums, using `Debug` format (`{:?}` gives PascalCase from derive), or hardcoding the two known rename cases in the log calls.

**What to watch for:**
- `format!("{:?}", V1ItemStatus::Researching)` gives `"Researching"` — this matches the PRD examples and is the simplest approach
- Using `{:?}` (Debug) for user-facing log messages is slightly unconventional but pragmatic for an enum with no custom Display impl
- Alternative: add `fn display_name(&self) -> &'static str` to both enums, mapping each variant to its capitalized form

### Per-Item Logging Without Breaking `map_v1_item()` Purity

**Why it's critical:** `map_v1_item()` is a clean, pure function. Adding logging inside it would mix concerns and require threading path/context through.

**Why it's easy to miss:** The natural instinct is to add logging where the transformation happens, but the right approach is to log from the caller by comparing v1 items against mapped v2 items.

**What to watch for:**
- Option A: Replace `.map(map_v1_item)` with a for-loop that logs per-item, then calls `map_v1_item()`
- Option B: Keep `.map(map_v1_item).collect()`, then iterate `v1.items.iter().zip(v2_items.iter())` in a separate logging pass
- Option A is simpler and avoids a second iteration. Recommended.

---

## Deep Dives

_None needed — scope is fully understood from code inspection._

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| How to format status names in log output? | PRD uses PascalCase (`Researching → Scoping`), enums serialize as snake_case | Use `{:?}` (Debug format gives PascalCase from derive). Simple, matches PRD examples. |

### Recommended Approaches

#### Per-Item Logging Strategy

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| For-loop replacing `.map()` | Single pass, natural logging point, simple | Slightly more verbose than functional style | Status change + phase clear need logging (this case) |
| Post-mapping zip comparison | Keeps `map_v1_item()` call unchanged | Second iteration, more complex | Logging is optional/conditional |

**Initial recommendation:** Replace the `.map().collect()` on line 199 with a for-loop that:
1. Calls `map_v1_item()` for each v1 item
2. Compares v1 status debug name vs v2 status debug name — logs if different
3. Checks if v1 had a phase and v2 has `None` — logs phase clearing
4. Checks if v1 `blocked_from_status` maps to a differently-named v2 status — logs warning
5. Logs full field mapping at debug level
6. Pushes mapped item to result vec

This is a single-pass approach with clear, linear logic.

#### Status Name Formatting

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `{:?}` (Debug derive) | Zero additional code, PascalCase matches PRD examples | Unconventional for user-facing output | Enum variants match desired display names (this case) |
| `Display` impl on both enums | Idiomatic Rust, explicit control | More code for minimal benefit | Display names differ from variant names |
| Hardcoded strings in log calls | Absolute minimal change | Fragile if mappings change | Only 2 cases and they'll never change |

**Initial recommendation:** Use `{:?}` format. Both `V1ItemStatus` and `ItemStatus` derive `Debug`, and the PascalCase variant names (`Researching`, `Scoping`, `Scoped`, `Ready`) match the PRD's example log messages exactly.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| `src/migration.rs:174-230` | Source | The function being modified |
| `src/migration.rs:96-106` | Source | Status mapping logic (which statuses change names) |
| `src/migration.rs:108-166` | Source | Per-item mapping logic (phase clearing, blocked_from_status) |
| `src/log.rs:42-74` | Source | Logging macros — usage patterns and level gating |
| `src/backlog.rs:17-48` | Source | Load function that calls migration |

---

## Assumptions

Decisions made without human input during autonomous tech research:

1. **Mode: Light** — This is a small, well-scoped change (adding log calls to an existing function). No external research needed — the logging infrastructure and migration code both already exist and are fully understood from code inspection.
2. **No external research needed** — The change uses existing `eprintln!`-based log macros. No logging frameworks, structured logging, or external patterns are involved.
3. **`{:?}` format is acceptable for status names** — The PascalCase output from `Debug` derive matches the PRD examples. If the user prefers snake_case or custom display names, a `Display` impl or `as_str()` method would be needed instead.
4. **For-loop over post-mapping zip** — A for-loop replacing `.map().collect()` is simpler and more readable than a separate comparison pass. This is a style choice.

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Internal codebase research | Read migration.rs, log.rs, backlog.rs. Mapped all insertion points, data availability, status rename cases, phase clearing logic, blocked_from_status handling. |
| 2026-02-12 | PRD analysis | 1 concern (status name formatting), 2 critical areas (display format, map purity). All addressable with simple approaches. |
| 2026-02-12 | Research complete | Scope confirmed as small/low-complexity. No blockers. Ready for design/spec. |
