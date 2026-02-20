# Change: Validate next_item_id on BACKLOG.yaml Load

**Status:** Proposed
**Created:** 2026-02-20
**Author:** phase-golem (autonomous)

## Terminology

- **Item ID** — The unique identifier for a backlog item, formatted as `{PREFIX}-{SUFFIX}` (e.g., `WRK-031`).
- **Prefix** — The alphanumeric project prefix configured in `phase-golem.toml` (e.g., `WRK`). All generated item IDs share this prefix.
- **ID suffix** — The numeric portion of an item ID after the prefix and dash (e.g., in `WRK-031`, the suffix is `31`). Parsed as `u32`.
- **`next_item_id`** — A high-water mark stored in `BACKLOG.yaml` representing the highest suffix ever assigned. Used as a floor in `generate_next_id()` to prevent ID reuse after archival.

## Problem Statement

When `BACKLOG.yaml` is loaded, the `next_item_id` field may be lower than the maximum ID suffix among existing items. This represents a data integrity issue — if `next_item_id` were the sole source for ID generation, it could cause ID collisions (two items assigned the same ID). Currently `generate_next_id()` defensively takes `max(items_max, next_item_id)` so there is no functional bug, but the inconsistency is silently ignored.

This is a warning rather than a hard error because the system is not currently broken — `generate_next_id()` already compensates. The warning surfaces the inconsistency so operators can investigate the root cause (manual edit, migration bug, partial write) without blocking operations.

This matters because:
- A `next_item_id` below the max item suffix indicates something went wrong
- Silent data inconsistencies erode trust in the system's integrity
- Surfacing the warning helps operators notice and fix the root cause

## User Stories / Personas

- **Operator/Developer** — Wants to know when backlog data is inconsistent so they can investigate and fix the root cause, rather than discovering problems later when IDs behave unexpectedly.

## Desired Outcome

When `BACKLOG.yaml` is loaded and `next_item_id` is less than the maximum ID suffix found among current items, the system logs a warning. The warning includes the current `next_item_id` value, the computed maximum, and the file path, providing the information needed for diagnosis. The system continues to operate normally — this is informational only, not a hard error.

Example warning message:
```
[backlog] next_item_id (42) is behind max item suffix (57) in /path/to/BACKLOG.yaml. Consider setting next_item_id to 57.
```

## Success Criteria

### Must Have

- [ ] On `backlog::load()`, after successful parse (and after any migrations), if `next_item_id < max(item ID suffixes)`, a warning is logged
- [ ] Warning message includes the `next_item_id` value, the max item ID suffix value, and the file path
- [ ] Warning includes a suggested fix value (the computed max)
- [ ] System continues loading normally after the warning (no error, no abort)
- [ ] No warning is logged when `next_item_id >= max(item ID suffixes)` (the normal case)
- [ ] No warning is logged when the backlog has no items (empty `items` list)

### Should Have

- [ ] The validation uses the same ID-parsing logic as `generate_next_id()` (filter by configured prefix, parse numeric suffix) to ensure consistency

### Nice to Have

- [ ] (none identified)

## Scope

### In Scope

- Adding a validation check in `backlog::load()` after successful parse and after all schema migrations complete
- Logging a warning via the existing `log_warn!` macro
- The validation logic: parse item ID suffixes (matching the configured prefix), compare max against `next_item_id`
- Graceful handling when config cannot be loaded (skip validation, do not error)

### Out of Scope

- Auto-correcting `next_item_id` (the existing `generate_next_id()` already handles this at runtime)
- Making this a hard error that prevents loading
- Validating `next_item_id` during save
- Validating other BACKLOG.yaml fields (item status transitions, duplicate IDs, etc.)
- Adding a CLI flag to suppress the warning
- Overflow protection for `next_item_id` approaching `u32::MAX`

## Non-Functional Requirements

- **Performance:** Negligible — iterating items to find max ID is O(n) on a list already fully loaded into memory. Config loading may add minor overhead but `load_config()` is a single TOML parse.

## Constraints

- Must use the existing `log_warn!` macro for consistency with other warnings in the codebase
- Must not change the return type or error behavior of `backlog::load()`
- The prefix used for ID parsing should come from the loaded config, consistent with how `generate_next_id()` works
- If `load_config()` fails (e.g., missing config file), skip the validation silently rather than failing the backlog load
- Warning is subject to log-level filtering (only shown at `Warn` level or below), which is acceptable behavior

## Dependencies

- **Depends On:** Nothing — the `load()` function and `log_warn!` macro already exist
- **Blocks:** Nothing

## Risks

- [ ] The prefix for parsing IDs needs to be available in `load()`. Currently `load()` takes `(path, project_root)` and calls `load_config(project_root)` only during migrations (schema version < 3). For v3 files, config is not loaded in the current code path. The validation will need to call `load_config()` unconditionally. If config loading fails, validation is skipped gracefully — this is acceptable because the validation is advisory.

## Open Questions

(All resolved during autonomous drafting — see Assumptions.)

## Assumptions

- **Mode: Light** — This is a straightforward validation addition with clear requirements.
- **No auto-correction** — The warning is purely informational. Auto-correction was considered but rejected because `generate_next_id()` already handles it at runtime, and silently mutating the file on load would be surprising behavior.
- **Prefix handling** — The validation calls `load_config(project_root)` to obtain the prefix. If config loading fails, the validation is skipped (no warning logged, no error returned). This keeps the validation advisory and non-breaking.
- **Empty backlog** — When the items list is empty, there is no max suffix to compare against, so no warning is logged regardless of `next_item_id` value. A nonzero `next_item_id` with no items is normal (items were archived).
- **Suggested fix in warning** — The warning includes actionable guidance: "Consider setting next_item_id to N" where N is the computed max suffix. This helps operators fix the issue without needing to compute the value themselves.

## References

- `src/backlog.rs:23-66` — `load()` function where the validation will be added
- `src/backlog.rs:108-125` — `generate_next_id()` which contains the ID parsing logic to reuse
- `src/types.rs:227-237` — `BacklogFile` struct with `next_item_id` field documentation
