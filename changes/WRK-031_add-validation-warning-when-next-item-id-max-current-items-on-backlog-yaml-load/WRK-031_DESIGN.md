# Design: Validate next_item_id on BACKLOG.yaml Load

**ID:** WRK-031
**Status:** Complete
**Created:** 2026-02-20
**PRD:** ./WRK-031_PRD.md
**Tech Research:** ./WRK-031_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add a post-deserialization validation step in `backlog::load()` that warns when `next_item_id` is lower than the maximum item ID suffix. The design extracts a shared pure function `max_item_suffix()` (used by both `generate_next_id()` and the new validation), plus a small `warn_if_next_id_behind()` helper called before each `Ok(backlog)` return point. The helper loads the project config to obtain the prefix, skips validation gracefully if config loading fails, and logs via the existing `log_warn!` macro.

---

## System Design

### High-Level Architecture

No new components or modules. This is a small validation addition within the existing `backlog::load()` function. The change introduces two new helper functions in `src/backlog.rs`: a pure `max_item_suffix()` function and a `warn_if_next_id_behind()` validation function.

```
load() ──► parse YAML ──► migrate if needed ──► warn_if_next_id_behind() ──► Ok(backlog)
                                                       │
                                                       ├─ load_config() for prefix
                                                       ├─ max_item_suffix() (shared pure function)
                                                       ├─ compare max suffix vs next_item_id
                                                       └─ log_warn! if behind

generate_next_id() also calls max_item_suffix() ──► same parsing logic, single source of truth
```

### Component Breakdown

#### `max_item_suffix()` (new shared pure function)

**Purpose:** Computes the maximum numeric ID suffix across all items matching a given prefix.

**Responsibilities:**
- Filter items to those whose ID starts with `{prefix}-`
- Parse the numeric suffix of each matching ID
- Return the maximum suffix, or 0 if no matching items exist

**Interfaces:**
- Input: `&[BacklogItem]`, `&str` (prefix)
- Output: `u32` (max suffix, or 0)

**Dependencies:** None — pure function, no I/O.

#### `warn_if_next_id_behind()` (new validation helper)

**Purpose:** Checks whether `next_item_id` is behind the max item ID suffix and logs a warning if so.

**Responsibilities:**
- Load project config to get the prefix (via `load_config`)
- Call `max_item_suffix()` to compute the max
- Compare max suffix against `backlog.next_item_id`
- Log a warning with `next_item_id`, max suffix, and file path if behind

**Interfaces:**
- Input: `&BacklogFile`, `&Path` (backlog file path, used only for the warning message), `&Path` (project root, used to load config)
- Output: `()` (side-effect only — may log a warning)

**Dependencies:** `load_config()` (called internally to obtain the prefix), `max_item_suffix()`, `log_warn!`

### Data Flow

1. `load()` reads and parses the YAML file into a `BacklogFile`
2. If migration is needed, migration runs and produces a `BacklogFile`
3. Before returning `Ok(backlog)`, `warn_if_next_id_behind()` is called
4. The helper loads config, calls `max_item_suffix()`, compares, and optionally logs
5. `load()` returns the `BacklogFile` unchanged

### Key Flows

#### Flow: Normal load with consistent next_item_id

> Backlog loads normally, no warning emitted.

1. **Parse YAML** — `load()` reads and parses the v3 BACKLOG.yaml
2. **Call validation** — `warn_if_next_id_behind(&backlog, path, project_root)` runs
3. **Load config** — `load_config(project_root)` succeeds, prefix is `"WRK"`
4. **Compute max suffix** — `max_item_suffix(&backlog.items, "WRK")` parses item IDs, filters to those with `WRK-` prefix, parses numeric suffixes (skipping any that fail to parse), returns max. Result: 42. `next_item_id` is 42.
5. **No warning** — `42 >= 42`, condition not met, nothing logged
6. **Return** — `Ok(backlog)` returned

#### Flow: Load with behind next_item_id

> Backlog loads, warning logged, operation continues normally.

1. **Parse YAML** — `load()` reads and parses the v3 BACKLOG.yaml
2. **Call validation** — `warn_if_next_id_behind(&backlog, path, project_root)` runs
3. **Load config** — `load_config(project_root)` succeeds, prefix is `"WRK"`
4. **Compute max suffix** — `max_item_suffix(&backlog.items, "WRK")` returns 57. `next_item_id` is 42.
5. **Log warning** — `42 < 57`, logs: `[backlog] next_item_id (42) is behind max item suffix (57) in /path/to/BACKLOG.yaml. Consider setting next_item_id to 57.`
6. **Return** — `Ok(backlog)` returned unchanged

**Edge cases:**
- **Empty items list** — `max_item_suffix()` returns 0. `next_item_id >= 0` is always true for `u32`, so no warning logged. A nonzero `next_item_id` with no items is normal (items were archived).
- **Config loading fails** — `.ok()` converts to `None`, validation skips entirely. No warning, no error. This is acceptable because config parse errors surface through other code paths during normal operation.
- **Items with non-matching prefix** — Filtered out by `strip_prefix` in `max_item_suffix()`, not counted toward max.
- **Items with non-numeric suffixes** (e.g., `WRK-abc`) — Filtered out by `parse::<u32>().ok()` in `max_item_suffix()`, not counted toward max. Consistent with `generate_next_id()` behavior.
- **Migration path** — Same validation runs after migration completes, before returning.

---

## Technical Decisions

### Key Decisions

#### Decision: Extract shared `max_item_suffix()` function

**Context:** Both `generate_next_id()` and the new validation helper need to compute the maximum numeric ID suffix from items. The tech research recommended reusing the same parsing logic to prevent divergence.

**Decision:** Extract the `strip_prefix` + `parse::<u32>` + `filter_map` + `max` chain into a shared pure function `max_item_suffix(items, prefix) -> u32`.

**Rationale:** Single source of truth — if the ID format ever changes, only one function needs updating. The function is pure (no I/O, no side effects), making it trivially testable.

**Consequences:** `generate_next_id()` is refactored to call `max_item_suffix()` instead of inlining the parsing. This is a small, safe refactor.

#### Decision: Extract validation helper function

**Context:** `load()` has two return points (line 49 for migration path, line 65 for direct v3 load). Validation must cover both.

**Decision:** Extract a `warn_if_next_id_behind()` helper function, called before each `Ok(backlog)` return.

**Rationale:** DRY — avoids duplicating the comparison + logging at two call sites. The helper is small and self-contained.

**Consequences:** One new function in the module. Each return site gets a one-line call.

#### Decision: Load config inside the validation helper

**Context:** The prefix is needed for ID parsing. In the migration path, config is already loaded but not in the direct v3 path.

**Decision:** The helper loads config independently via `load_config(project_root).ok()`.

**Rationale:** Simplicity — avoids threading config through the function or restructuring `load()`. The redundant TOML parse on the migration path is negligible: `load_config()` parses a small TOML file and runs lightweight validation checks. Migration is a rare one-time event per schema version bump.

**Consequences:** On the migration path, config is loaded twice. This is an acceptable tradeoff for a simpler API.

#### Decision: Return `()` not `Result`

**Context:** The validation is advisory. Should it return errors?

**Decision:** The helper returns `()`. Config loading failure and empty-items cases are handled internally.

**Rationale:** The validation must never block loading. Making it infallible at the type level prevents callers from accidentally propagating errors.

**Consequences:** All failure modes (config load failure, no items) are silent — the warning is the only possible output.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Redundant config load | Config may be loaded twice on migration path | Simpler helper API (no config threading) | Config loading is a small TOML parse + lightweight validation; migration is rare |
| No auto-correction | Inconsistency persists until manual fix | Predictable, non-surprising behavior | `generate_next_id()` already compensates at runtime |
| Silent config failure | If config can't load, validation is skipped with no indication | Validation never blocks loading | Config failures surface through other code paths (e.g., phase execution) |

---

## Alternatives Considered

### Alternative: Inline validation at each return site

**Summary:** Duplicate the validation logic directly before each `Ok(backlog)` return.

**How it would work:**
- Copy the config load + compare + warn block before line 49 and line 65

**Pros:**
- No new function, minimal diff

**Cons:**
- Duplicated logic (violates DRY)
- Harder to maintain if logic changes
- Not independently testable

**Why not chosen:** The logic is non-trivial enough (config load + ID parsing + comparison + formatting) that duplication would be a maintenance burden.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Config file has a parse error, causing validation to skip silently | Warning not shown when it should be | Low — parse errors are caught earlier in normal operation | Acceptable — validation is advisory. If config is broken, other things will fail first. |

---

## Integration Points

### Existing Code Touchpoints

- `src/backlog.rs:23-66` — `load()` function: two call sites added (before line 49 return and before line 65 return)
- `src/backlog.rs` — New `max_item_suffix()` pure function added (private helper, shared between validation and generation)
- `src/backlog.rs` — New `warn_if_next_id_behind()` function added (private helper)
- `src/backlog.rs:108-125` — `generate_next_id()`: refactored to call `max_item_suffix()` instead of inlining the parsing chain (behavior unchanged)
- `src/config.rs:366-396` — `load_config()`: called by the validation helper (not modified)
- `src/log.rs:49-56` — `log_warn!` macro: used by the validation helper (not modified)

### External Dependencies

None — all components already exist in the codebase.

---

## Open Questions

None — all questions resolved during PRD and tech research phases.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-20 | Initial design draft | Helper function approach selected; light mode with one alternative considered |
| 2026-02-20 | Self-critique (7 agents) | 4 auto-fixes applied; design polished |
