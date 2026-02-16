# SPEC: Eliminate BacklogSnapshot by Using BacklogFile Directly

**ID:** WRK-007
**Status:** Ready
**Created:** 2026-02-13
**PRD:** ./WRK-007_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

The orchestrator codebase has two near-identical types: `BacklogFile` (3 fields) and `BacklogSnapshot` (2 fields, missing only `next_item_id: u32`). `BacklogSnapshot` is a read-only projection created by `handle_get_snapshot()` in `coordinator.rs`, consumed by the scheduler and filter modules. This change eliminates `BacklogSnapshot` entirely, using `BacklogFile` (via `&BacklogFile` for read-only consumers) everywhere. The Rust borrow checker enforces immutability — no separate "view" type is needed.

## Approach

This is a mechanical type-level refactoring. The strategy:

1. Add `Default` derive to `BacklogFile` (prerequisite for test construction with `..Default::default()`)
2. Update all source files to use `BacklogFile` instead of `BacklogSnapshot`
3. Delete `BacklogSnapshot` from `types.rs`
4. Update all test files to use `BacklogFile`
5. Verify zero references to `BacklogSnapshot` remain

The change is bottom-up: types first, then coordinator, then consumers (scheduler, filter, main), then tests. Since the refactoring is small and tightly coupled, all changes are in a single phase — splitting would create intermediate states that don't compile.

**Patterns to follow:**

- `orchestrator/src/types.rs:187-228` — `BacklogItem` struct with `Default` derive (same pattern for `BacklogFile`)
- `orchestrator/src/coordinator.rs:354-359` — current `handle_get_snapshot()` (will be simplified to `state.backlog.clone()`)

**Implementation boundaries:**

- Do not rename `BacklogFile` to `Backlog` (out of scope, noted in PRD)
- Do not optimize clone cost with `Arc`/`im::Vector` (tracked as WRK-024)
- Do not add `next_item_id` validation or semantics to the scheduler
- Do not change serialization format

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Eliminate BacklogSnapshot | Low | Replace all BacklogSnapshot usage with BacklogFile across source and test files |

**Ordering rationale:** Single phase — the change is small (~80 lines across 8 files) and all modifications are tightly coupled. Splitting would create intermediate compilation failures.

---

## Phases

### Phase 1: Eliminate BacklogSnapshot

> Replace all BacklogSnapshot usage with BacklogFile across source and test files

**Phase Status:** complete

**Complexity:** Low

**Goal:** Remove `BacklogSnapshot` entirely and use `BacklogFile` in all modules, preserving identical runtime behavior.

**Files:**

- `orchestrator/src/types.rs` — modify — Add `Default` derive to `BacklogFile`; delete `BacklogSnapshot` struct (lines 264-268)
- `orchestrator/src/coordinator.rs` — modify — Change `GetSnapshot` reply type, `get_snapshot()` return type, and simplify `handle_get_snapshot()` to `state.backlog.clone()`
- `orchestrator/src/scheduler.rs` — modify — Change 3 function signatures from `&BacklogSnapshot` to `&BacklogFile`; update import
- `orchestrator/src/filter.rs` — modify — Change `apply_filter()` to take `&BacklogFile` and return `BacklogFile` with `next_item_id` carry-forward; update import
- `orchestrator/src/main.rs` — modify — Simplify filter preview to pass `&backlog` directly instead of constructing `BacklogSnapshot`
- `orchestrator/tests/types_test.rs` — modify — Update `BacklogSnapshot` serde test to use `BacklogFile`
- `orchestrator/tests/scheduler_test.rs` — modify — Rename `make_snapshot()` to `make_backlog()`; add `next_item_id: 0`; update import and all call sites
- `orchestrator/tests/filter_test.rs` — modify — Rename `make_snapshot()` to `make_backlog()`; add `next_item_id: 0`; update import and all call sites

**Tasks:**

- [x] Add `Default` to `BacklogFile` derive list in `types.rs` (line 230): `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]`
- [x] Delete `BacklogSnapshot` struct from `types.rs` (lines 264-268)
- [x] Update `coordinator.rs` imports: remove `BacklogSnapshot` from import list (line 7)
- [x] Change `CoordinatorCommand::GetSnapshot` reply type to `oneshot::Sender<BacklogFile>` (line 15)
- [x] Change `CoordinatorHandle::get_snapshot()` return type to `Result<BacklogFile, String>` (line 95)
- [x] Simplify `handle_get_snapshot()` to `state.backlog.clone()` (lines 354-359)
- [x] Update `scheduler.rs` imports: replace `BacklogSnapshot` with `BacklogFile` (line 16)
- [x] Change `select_actions()` parameter from `&BacklogSnapshot` to `&BacklogFile` (line 146)
- [x] Change `advance_to_next_active_target()` parameter from `&BacklogSnapshot` to `&BacklogFile` (line 466)
- [x] Change `select_targeted_actions()` parameter from `&BacklogSnapshot` to `&BacklogFile` (line 840)
- [x] Update `filter.rs` imports: replace `BacklogSnapshot` with `BacklogFile` (line 2)
- [x] Change `apply_filter()` signature to take `&BacklogFile` and return `BacklogFile` (line 144)
- [x] Add `next_item_id: backlog.next_item_id` to the `BacklogFile` construction in `apply_filter()` with comment: `// next_item_id is carried forward for structural completeness only. Filtered results are never persisted; the coordinator owns ID generation.`
- [x] Simplify `main.rs` filter preview (lines 343-346) to `filter::apply_filter(criterion, &backlog)` — remove manual `BacklogSnapshot` construction
- [x] Update `types_test.rs`: change `BacklogSnapshot` serde test to use `BacklogFile` (add `next_item_id` field)
- [x] Update `scheduler_test.rs`: replace import, rename `make_snapshot()` to `make_backlog()`, update return type and construction (add `next_item_id: 0`), update all call sites
- [x] Update `filter_test.rs`: replace import, rename `make_snapshot()` to `make_backlog()`, update return type and construction (add `next_item_id: 0`), update all call sites and direct `BacklogSnapshot` constructions (line 334)
- [x] Add test in `filter_test.rs` verifying `apply_filter()` preserves `next_item_id`: create `BacklogFile` with `next_item_id: 42`, filter it, assert result has `next_item_id: 42`
- [x] Run `cargo build` — verify zero compilation errors
- [x] Run `cargo test` — verify all tests pass
- [x] Run `grep -r "BacklogSnapshot" orchestrator/` — verify zero references remain

**Verification:**

- [x] `cargo build` succeeds with no errors or warnings related to BacklogSnapshot
- [x] `cargo test` passes — all existing tests pass without behavior changes
- [x] `grep -r "BacklogSnapshot" orchestrator/` returns zero results
- [x] `apply_filter()` carries forward `next_item_id` from input (verified by new `apply_filter_preserves_next_item_id` test)
- [x] `handle_get_snapshot()` returns a full `BacklogFile` clone (verified by code review — `state.backlog.clone()` is a one-liner)
- [x] All PRD success criteria are met

**Commit:** `[WRK-007][P1] Clean: Eliminate BacklogSnapshot via direct BacklogFile usage`

**Notes:**

- All changes must be applied atomically (single commit) since partial application creates compilation failures. The Rust compiler will catch any missed `BacklogSnapshot` references as type errors, making the change safe despite being large in scope
- The `next_item_id` comment in `apply_filter()` is required per design doc — prevents future developers from assuming the value is semantically meaningful in filtered output
- Test helper constructions should use explicit `next_item_id: 0` rather than `..Default::default()` — explicit construction is clearer and avoids hiding the field that motivated this change

**Followups:**

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met
- [x] Tests pass
- [x] No regressions introduced
- [x] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | complete | 2118873 | All 8 files modified, all tests pass, zero BacklogSnapshot references remain |

## Followups Summary

### Critical

### High

### Medium

### Low

## Design Details

### Key Types

**BacklogFile (after change):**
```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct BacklogFile {
    pub schema_version: u32,
    #[serde(default)]
    pub items: Vec<BacklogItem>,
    /// Highest numeric suffix ever assigned for ID generation.
    #[serde(default)]
    pub next_item_id: u32,
}
```

**BacklogSnapshot** — DELETED

### Architecture Details

No architectural changes. The existing actor model is preserved:
- Coordinator owns `BacklogFile` as internal state
- `handle_get_snapshot()` clones the entire `BacklogFile` (was: manual field projection)
- Scheduler receives owned `BacklogFile` through oneshot channel, passes `&BacklogFile` to pure functions
- Filter takes `&BacklogFile`, returns new `BacklogFile` with filtered items

### Design Rationale

See `WRK-007_DESIGN.md` for full rationale. Key points:
- Direct elimination is idiomatic Rust — the borrow checker enforces immutability via `&BacklogFile`
- Newtype wrapper alternative rejected as over-engineered for hiding a single harmless `u32`
- Clone cost increase is negligible (4 bytes vs `Vec<BacklogItem>` which dominates); WRK-024 tracks optimization if needed

---

## Retrospective

### What worked well?

- Single-phase approach was correct — all changes were tightly coupled and the Rust compiler verified completeness
- Mechanical refactoring with zero runtime behavior changes, exactly as planned

### What was harder than expected?

- Nothing — the change was straightforward and the SPEC accurately predicted the scope

### What would we do differently next time?

- Nothing — this was a textbook small refactoring
