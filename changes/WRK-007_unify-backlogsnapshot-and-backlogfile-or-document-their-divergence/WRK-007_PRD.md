# Change: Eliminate BacklogSnapshot by Using BacklogFile Directly

**Status:** Proposed
**Created:** 2026-02-13
**Author:** AI (autonomous)

## Problem Statement

The codebase has two near-identical types for representing backlog state: `BacklogFile` and `BacklogSnapshot`. They share the same two core fields (`schema_version: u32`, `items: Vec<BacklogItem>`) and differ only in that `BacklogFile` has an additional `next_item_id: u32` field used for ID generation.

`BacklogSnapshot` exists as a read-only projection of `BacklogFile`, created by `handle_get_snapshot()` in `coordinator.rs:354-358` which copies `items` and `schema_version` while excluding `next_item_id`. The scheduler and filter modules consume `BacklogSnapshot`, never needing `next_item_id`.

While the divergence is intentional (the snapshot is a read-only view that omits the mutable ID counter), this creates unnecessary complexity:

1. **Two types for one concept** — Readers must understand both types and the relationship between them. The names don't clearly communicate the difference; `BacklogSnapshot` suggests a point-in-time copy (which it is), but `BacklogFile` suggests a file-backed struct (which is incidental).

2. **Maintenance burden** — Any new field added to `BacklogFile` that should be visible to the scheduler must also be added to `BacklogSnapshot` and the mapping code in `handle_get_snapshot()`.

3. **No compile-time safety** — There's no enforcement that `BacklogSnapshot` stays in sync with `BacklogFile`. If a field is added to `BacklogFile` but not to `BacklogSnapshot`, the compiler won't catch it — the snapshot will silently omit the new field.

The simplest fix: eliminate `BacklogSnapshot` entirely and pass `BacklogFile` (or `&BacklogFile`) to the scheduler and filter code. The `next_item_id` field carries no side effects for readers and adds negligible overhead (a single `u32`). Immutability is enforced by passing `&BacklogFile` references — the scheduler and filter modules cannot mutate the coordinator's state.

## User Stories / Personas

- **Orchestrator developer** — Wants a single backlog representation type instead of two near-duplicates. Wants to add fields to the backlog struct without updating a mapping function.

## Desired Outcome

There is a single struct representing backlog state. The coordinator, scheduler, and filter modules all use the same type. Adding a new field to the backlog requires changes only at the struct definition and any code that actually uses the field — no manual projection/mapping step.

## Success Criteria

### Must Have

- [ ] `BacklogSnapshot` is removed from `types.rs`
- [ ] All sites that previously used `BacklogSnapshot` now use `BacklogFile`
- [ ] `handle_get_snapshot()` in `coordinator.rs` returns `BacklogFile` (cloning items + schema_version + next_item_id)
- [ ] `select_actions()`, `select_targeted_actions()`, `advance_to_next_active_target()`, and related scheduler functions accept `&BacklogFile` instead of `&BacklogSnapshot`
- [ ] `apply_filter()` in `filter.rs` accepts and returns `BacklogFile` instead of `BacklogSnapshot`
- [ ] All existing tests pass without behavior changes
- [ ] The coordinator actor's `CoordinatorCommand::GetSnapshot` variant uses `BacklogFile`
- [ ] Test construction sites that build `BacklogSnapshot` are updated to build `BacklogFile` (adding `next_item_id: 0` or using `..Default::default()`)
- [ ] `apply_filter()` carries forward `next_item_id` and `schema_version` from the input `BacklogFile`
- [ ] Zero references to `BacklogSnapshot` remain in the codebase (confirmed by grep)

### Should Have

- [ ] None identified

### Nice to Have

- [ ] None identified

## Scope

### In Scope

- Removing `BacklogSnapshot` from `types.rs`
- Updating `coordinator.rs` to return `BacklogFile` from `GetSnapshot`
- Updating `scheduler.rs` to accept `&BacklogFile` in all functions that currently take `&BacklogSnapshot`
- Updating `filter.rs` to use `BacklogFile` instead of `BacklogSnapshot`
- Updating test files that construct or reference `BacklogSnapshot`

### Out of Scope

- Renaming `BacklogFile` to something else (e.g., `Backlog`) — a reasonable follow-up but not in scope here
- Changing `BacklogFile` to use `im::Vector` or `Arc` for cheap cloning (tracked separately as WRK-024)
- Adding `next_item_id` validation or semantics to the scheduler
- Changing serialization format

## Non-Functional Requirements

- **Performance:** No change — the clone in `handle_get_snapshot()` now includes one extra `u32` field (`next_item_id`), which is negligible

## Constraints

- `BacklogFile` derives `Serialize, Deserialize` so passing it through channels is fine (it's already `Clone + Debug + PartialEq`)
- The `next_item_id` field on `BacklogFile` has `#[serde(default)]`, which tells the deserializer to use the type's `Default` value (0) if the field is missing from YAML. This ensures backward compatibility with test fixtures that predate the field
- The `apply_filter()` function constructs a new `BacklogFile` from filtered items — it must carry forward `next_item_id` and `schema_version` from the input. The `next_item_id` is semantically irrelevant in filtered results (they are never persisted), but carrying it forward avoids inventing sentinel values
- Immutability for scheduler/filter is enforced via `&BacklogFile` references. The coordinator clones the `BacklogFile` before sending it through the channel; the scheduler receives an owned copy that it cannot use to mutate the coordinator's state

## Dependencies

- **Depends On:** Nothing
- **Blocks:** Nothing directly, but simplifies future changes to backlog state (any new field is automatically available to all consumers)

## Risks

- [ ] Very low risk: The change is mechanical — find `BacklogSnapshot`, replace with `BacklogFile`, and update construction/mapping sites. The compiler will catch any missed references since `BacklogSnapshot` will no longer exist.

## Open Questions

None — all decisions resolved in Assumptions section.

## Assumptions

- **Elimination over documentation** — Rather than documenting why two types exist, we eliminate the second type entirely. The cost of maintaining two types (even well-documented) exceeds the benefit of the read-only projection, especially since `next_item_id` is a single `u32` that readers can trivially ignore.
- **`BacklogFile` name retained as-is** — While `Backlog` might be a better name (since the type is now used beyond file I/O), renaming is a separate concern. This change focuses on removing duplication, not perfecting naming.
- **`apply_filter` returns `BacklogFile`** — The filter function currently constructs a new `BacklogSnapshot` with filtered items. It will now construct a `BacklogFile`, carrying forward `schema_version` and `next_item_id` from the source. The `next_item_id` value in a filtered result is semantically irrelevant (the filtered result is never persisted), but carrying it forward is simpler than inventing a sentinel value.
- **No behavioral change to `next_item_id` handling** — The coordinator remains the only code path that reads or modifies `next_item_id`. The scheduler receives it as part of `BacklogFile` but never accesses it. This is fine — unused struct fields are common in Rust and carry no runtime cost.

## References

- `orchestrator/src/types.rs:231-240` — `BacklogFile` definition
- `orchestrator/src/types.rs:265-268` — `BacklogSnapshot` definition (to be removed)
- `orchestrator/src/coordinator.rs:354-358` — `handle_get_snapshot()` mapping
- `orchestrator/src/scheduler.rs` — Primary consumer of `BacklogSnapshot`
- `orchestrator/src/filter.rs` — Filter uses `BacklogSnapshot`
- `_ideas/WRK-024_immutable-backlog-snapshots.md` — Related optimization (separate scope)
