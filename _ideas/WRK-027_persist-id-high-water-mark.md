# WRK-027: Persist high-water mark for item IDs in BACKLOG.yaml

## Problem Statement

`generate_next_id()` in `backlog.rs` determines the next item ID by scanning all current items for the highest numeric suffix and incrementing it. When items are archived (removed from `BACKLOG.yaml` and written to the worklog), their IDs are no longer visible to the scan. If enough items are archived, the next generated ID can collide with previously used IDs, leading to:

- Conflicting worklog references
- Overwritten `changes/` directories on disk
- Ambiguous item references in commit messages and phase results

## Proposed Approach

1. **Add `next_id` field to `BacklogFile`** (`types.rs`): A new `Option<u32>` field with `#[serde(default)]` so existing BACKLOG.yaml files without the field deserialize correctly (defaulting to `None`).

2. **Update `generate_next_id()`** (`backlog.rs`): Use `max(current_items_max, next_id.unwrap_or(0)) + 1` as the new ID number. After generating, update the `next_id` field on the `BacklogFile` to the newly generated value.

3. **Update callers** (`add_item`, `ingest_follow_ups`): These already take `&mut BacklogFile` and call `generate_next_id()`. The function signature changes to `&mut BacklogFile` (currently `&BacklogFile`) so it can update the high-water mark in place.

4. **Backward compatibility**: `#[serde(default)]` + `skip_serializing_if` handles the migration naturally. On first ID generation against an old backlog, it computes from current items (safe as long as no items have been archived yet without the field). On save, the field is persisted going forward.

## Files Affected

- **Modified:**
  - `orchestrator/src/types.rs` — Add `next_id: Option<u32>` to `BacklogFile`
  - `orchestrator/src/backlog.rs` — Update `generate_next_id()` signature and logic, update callers
  - `orchestrator/tests/backlog_test.rs` — Update tests for new behavior

## Assessment

| Dimension  | Rating | Rationale |
|------------|--------|-----------|
| Size       | Small  | 2-3 production files, ~20 lines of logic change |
| Complexity | Low    | Single field addition + one function logic change |
| Risk       | Medium | Modifies core data structure serialization; backward compat needs care |
| Impact     | High   | Prevents silent data corruption from ID collisions |

## Assumptions

- The `next_id` field uses `Option<u32>` with `#[serde(default)]` rather than requiring a v2→v3 schema migration. This is safe because `None` falls back to the current scan behavior, which is correct for backlogs that have never archived an item.
- The field name `next_id` is preferred over `id_high_water_mark` for brevity, storing the next ID to assign rather than the last assigned ID.
- No worklog scanning is needed for the migration path — if a backlog has already archived items without this field, the user accepts that IDs may overlap with very old archived items. The field prevents *future* collisions from the point it's introduced.
- The `generate_next_id()` signature changes from `&BacklogFile` to `&mut BacklogFile`, which is a compatible change since all call sites already have mutable access.
