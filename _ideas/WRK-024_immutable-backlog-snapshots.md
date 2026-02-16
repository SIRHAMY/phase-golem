# WRK-024: Investigate using immutable data structures for backlog snapshots to avoid cloning

## Problem Statement

The coordinator actor in `coordinator.rs:295-300` creates `BacklogSnapshot` values by deep-cloning the entire `Vec<BacklogItem>` on every call to `handle_get_snapshot()`. Each `BacklogItem` contains ~18 fields including multiple `String` and `Vec<String>` values, so cloning is not trivial. The scheduler calls `get_snapshot()` 12+ times across its various functions (`select_actions`, `select_targeted_actions`, status helpers, etc.), meaning a single scheduler iteration can produce many full copies of the item list.

For small backlogs (5-20 items) this is negligible, but it introduces unnecessary allocation overhead and scales linearly with backlog size. Replacing `Vec<BacklogItem>` with a persistent/immutable data structure (e.g., the `im` crate's `Vector`) would make snapshot creation O(1) through structural sharing — the snapshot simply holds a reference-counted pointer to the same tree, and mutations create new nodes only for the changed path.

## Proposed Approach

### 1. Add the `im` crate as a dependency

Add `im = "15"` to `Cargo.toml`. The `im` crate provides persistent immutable data structures (`Vector`, `HashMap`, `OrdMap`) that implement structural sharing via balanced trees with reference-counted nodes. `im::Vector` is the direct replacement for `Vec` in this use case.

### 2. Replace `Vec<BacklogItem>` with `im::Vector<BacklogItem>` in `BacklogFile` and `BacklogSnapshot`

Change the `items` field in both `BacklogFile` (types.rs:194) and `BacklogSnapshot` (types.rs:222) from `Vec<BacklogItem>` to `im::Vector<BacklogItem>`. This makes `.clone()` on the items field O(1) — it increments a reference count rather than deep-copying every item.

### 3. Update serialization layer

`im::Vector` implements `serde::Serialize` and `serde::Deserialize` when the `serde` feature is enabled on the `im` crate. The YAML serialization in `backlog.rs` should work without changes, but needs verification. Deserialization from YAML produces a fresh `im::Vector`, and serialization iterates the structure just like a `Vec`.

### 4. Adapt mutation call sites

Code that mutates `items` (e.g., `push`, index assignment, `iter_mut()`, `retain`) needs to use `im::Vector`'s API instead. Key differences:
- `push` → `push_back`
- `items[i] = x` → `items.set(i, x)` (returns new vector) or `items[i] = x` (in-place with `&mut`)
- `iter_mut()` → `iter_mut()` (supported by `im::Vector`)
- `retain()` → `retain()` (supported)

Most call sites in `backlog.rs` and `coordinator.rs` do single-item mutations that map naturally to `im::Vector`'s API.

### 5. Remove explicit `.clone()` in `handle_get_snapshot`

After the switch, `state.backlog.items.clone()` becomes a cheap O(1) operation. The code can remain as-is (clone is now trivially cheap) or the snapshot could hold a direct reference/`Arc` to the same vector, but keeping the clone is simpler and idiomatic with `im`.

### 6. Update tests

Test code that constructs `BacklogFile` or `BacklogSnapshot` with `vec![...]` will need to use `im::vector![...]` or `.into()` conversions. Fixture loading via YAML deserialization should work unchanged.

## Files Affected

- **Modified:** `orchestrator/Cargo.toml` — Add `im` dependency with `serde` feature
- **Modified:** `orchestrator/src/types.rs` — Change `Vec<BacklogItem>` to `im::Vector<BacklogItem>` in `BacklogFile` and `BacklogSnapshot`
- **Modified:** `orchestrator/src/backlog.rs` — Adapt item mutation APIs (`push_back`, etc.)
- **Modified:** `orchestrator/src/coordinator.rs` — Minor API adjustments for item mutations
- **Modified:** `orchestrator/src/scheduler.rs` — Update any direct `Vec` usage on snapshot items
- **Modified:** `orchestrator/src/preflight.rs` — Update iteration patterns if needed
- **Modified:** `orchestrator/src/main.rs` — Update status display iteration if needed
- **Modified:** `orchestrator/tests/*.rs` — Update test item construction

## Assessment

| Dimension  | Rating | Rationale |
|------------|--------|-----------|
| Size       | Medium | 6-8 files modified; mostly mechanical type changes but touches core data structures throughout |
| Complexity | Medium | Need to evaluate `im` crate API compatibility, serde integration, and mutation patterns; conceptually straightforward but requires careful verification |
| Risk       | Medium | Changes the core data type used throughout the system; any API mismatch or serialization issue would be a compile error (good), but subtle behavioral differences in iteration order or mutation semantics could surface at runtime |
| Impact     | Low | Nice-to-have optimization; current backlog sizes (5-30 items) make the cloning cost negligible; benefit only materializes at scale that may never be reached |

## Alternatives Considered

- **`Arc<Vec<BacklogItem>>`**: Wrap items in `Arc` and clone the `Arc` for snapshots. Simpler change (no new crate), but mutations require `Arc::make_mut` which still does a full clone when the refcount > 1, defeating the purpose if the coordinator mutates between snapshot reads.
- **Read-write lock instead of message passing**: Replace the actor model with a shared `RwLock<BacklogFile>`. Would avoid cloning entirely but changes the concurrency model, which is a larger architectural shift.
- **Do nothing**: The backlog is small and likely to stay small. Cloning 20-30 items with ~18 fields each is microseconds. The optimization may not be worth the added dependency and API surface change.

## Assumptions

- The `im` crate's `Vector` type with the `serde` feature enabled will serialize/deserialize identically to `Vec` for YAML format. This needs verification but is a documented feature of the crate.
- No code depends on `Vec`-specific traits or methods that `im::Vector` doesn't implement (e.g., `as_slice()`, `Deref<Target=[T]>`). `im::Vector` does not deref to a slice, so any code using slice methods would need adaptation.
- The performance benefit is theoretical at current scale. This is a forward-looking optimization and code quality improvement (making the cheap-clone semantics explicit), not a fix for an observed performance problem.
- The `im` crate is well-maintained and suitable as a long-term dependency. It has broad adoption in the Rust ecosystem.
