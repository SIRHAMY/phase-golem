# SPEC: Validate next_item_id on BACKLOG.yaml Load

**ID:** WRK-031
**Status:** Ready
**Created:** 2026-02-20
**PRD:** ./WRK-031_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

When `BACKLOG.yaml` is loaded, `next_item_id` may be lower than the maximum item ID suffix — indicating a data integrity issue (manual edit, migration bug, partial write). Currently `generate_next_id()` defensively compensates, but the inconsistency is silently ignored. This change adds an advisory warning on load so operators can investigate the root cause.

The implementation is small and well-scoped: two new private helper functions in `src/backlog.rs`, a behavioral-neutral refactor of `generate_next_id()` to share parsing logic, and two call sites in `load()`.

## Approach

Extract a shared pure function `max_item_suffix()` from the ID-parsing chain currently inlined in `generate_next_id()`. Add a `warn_if_next_id_behind()` helper that loads config for the prefix, calls `max_item_suffix()`, compares the result against `next_item_id`, and logs a warning via `log_warn!` if behind. Wire the helper into `load()` before both `Ok(backlog)` return points (line 49 migration path and line 65 direct v3 path).

```
load() ──► parse YAML ──► migrate if needed ──► warn_if_next_id_behind() ──► Ok(backlog)
                                                       │
                                                       ├─ load_config() for prefix
                                                       ├─ max_item_suffix() (shared pure function)
                                                       ├─ compare max suffix vs next_item_id
                                                       └─ log_warn! if behind
```

**Patterns to follow:**

- `src/backlog.rs:108-125` — `generate_next_id()` for the `strip_prefix` + `parse::<u32>` + `filter_map` + `max` chain to extract into `max_item_suffix()`
- `src/log.rs:49-56` — `log_warn!` macro usage pattern
- `src/backlog.rs:7` — `load_config` import already present
- `tests/backlog_test.rs:844-871` — TempDir + `fs::write` pattern for load-path tests (see `backward_compatible_yaml_load_without_next_item_id`)

**Implementation boundaries:**

- Do not modify: `src/types.rs`, `src/config.rs`, `src/log.rs`, `Cargo.toml`
- Do not refactor: `load()` control flow beyond adding the two call sites
- Do not add: auto-correction of `next_item_id`, hard errors, CLI flags

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Extract shared helper & wire validation | Low | Extract `max_item_suffix()`, refactor `generate_next_id()`, add `warn_if_next_id_behind()`, wire into `load()`, write all tests |

**Ordering rationale:** This is a single-phase change — all pieces are small and tightly coupled. Splitting would create incomplete intermediate states (e.g., `max_item_suffix()` exists but nothing calls it).

---

## Phases

### Phase 1: Extract shared helper & wire validation

> Extract `max_item_suffix()`, refactor `generate_next_id()` to use it, add `warn_if_next_id_behind()`, wire into `load()`, and write tests

**Phase Status:** complete

**Complexity:** Low

**Goal:** Add a warning when `next_item_id` is behind the max item ID suffix on backlog load, with shared ID-parsing logic between validation and generation.

**Files:**

- `src/backlog.rs` — modify — Add `max_item_suffix()` pure function, refactor `generate_next_id()` to call it, add `warn_if_next_id_behind()` helper, add two call sites in `load()`
- `tests/backlog_test.rs` — modify — Add tests for the new validation warning behavior through `load()`

**Patterns:**

- Follow `src/backlog.rs:108-125` (`generate_next_id`) for the ID-parsing chain to extract
- Follow `tests/backlog_test.rs:844-871` for the TempDir + inline YAML load-path test pattern
- Follow `src/log.rs` for unit test module placement if adding `#[cfg(test)]` tests for `max_item_suffix()`

**Tasks:**

- [x] Add `max_item_suffix(items: &[BacklogItem], prefix: &str) -> u32` pure function in `src/backlog.rs` — extracts the `strip_prefix` + `parse::<u32>` + `filter_map` + `max` + `unwrap_or(0)` chain from `generate_next_id()`
- [x] Refactor `generate_next_id()` to call `max_item_suffix()` instead of inlining the parsing chain — behavior unchanged, all 10+ existing tests must pass
- [x] Add `warn_if_next_id_behind(backlog: &BacklogFile, path: &Path, project_root: &Path)` helper — loads config via `load_config(project_root).ok()`, calls `max_item_suffix()`, compares, logs warning if `next_item_id < max_suffix`. If config load fails (returns `None`), return immediately with no warning and no error.
- [x] Wire `warn_if_next_id_behind(&backlog, path, project_root)` immediately before `return Ok(backlog)` at line 49 (migration path, inside `if schema_version <= 2` block) in `load()`
- [x] Wire `warn_if_next_id_behind(&backlog, path, project_root)` immediately before `Ok(backlog)` at line 65 (direct v3 path) in `load()`
- [x] Add unit tests for `max_item_suffix()` in a `#[cfg(test)]` module in `src/backlog.rs`:
  - Empty items returns 0
  - Items with matching prefix — returns correct max suffix
  - Items with non-matching prefix filtered out — returns 0
  - Items with non-numeric suffixes filtered out
  - Mixed valid/invalid items — returns max from valid items only
  - Non-default prefix (e.g., `"PROJ"`) works correctly
- [x] Add integration tests in `tests/backlog_test.rs`:
  - Load with `next_item_id` behind max (e.g., `next_item_id=3`, items include `WRK-010`) — verify load succeeds
  - Load with `next_item_id == max_suffix` — verify load succeeds (boundary: no warning)
  - Load with `next_item_id > max_suffix` — verify load succeeds (normal case)
  - Load with empty items and `next_item_id=0` — verify load succeeds (no warning)
  - Load with empty items and nonzero `next_item_id` (archived items scenario) — verify load succeeds (no warning)
- [x] Verify all existing `generate_next_id()` tests still pass (regression — tests in `tests/backlog_test.rs` lines 182-245, 722-780)

**Verification:**

- [x] `cargo test` passes — all existing tests plus new tests
- [x] `cargo build` succeeds with no warnings
- [x] New `max_item_suffix()` unit tests cover: empty items, matching prefix, non-matching prefix, non-numeric suffixes, mixed items, non-default prefix
- [x] New load-path integration tests cover: behind case, boundary (equal) case, normal case, empty items (zero and nonzero `next_item_id`)
- [x] All existing `generate_next_id()` tests pass unchanged — behavioral equivalence after refactor (tests at `tests/backlog_test.rs` lines 182-245, 722-780)
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[WRK-031][P1] Feature: Add next_item_id validation warning on backlog load`

**Notes:**

- `max_item_suffix()` is private (`fn`, not `pub fn`) — unit tests go in an inline `#[cfg(test)]` module within `src/backlog.rs`
- Warning message format: `[backlog] next_item_id ({next_id}) is behind max item suffix ({max_suffix}) in {path}. Consider setting next_item_id to {max_suffix}.` — `{path}` uses `path.display()`, consistent with existing error messages in `load()`
- `warn_if_next_id_behind()` returns `()` — infallible by design. Config load failure (via `.ok()` converting `Err` to `None`) → early return with no warning and no error. Empty items → `max_suffix` is 0, condition `next_item_id < 0` is impossible for `u32`, so no warning. A nonzero `next_item_id` with no items is normal (items were archived).
- On the migration path, config may be loaded twice (once during migration, once for validation). This is acceptable — config loading is a small TOML parse and migration is rare.
- The `path` parameter in `warn_if_next_id_behind()` is the BACKLOG.yaml file path (for the warning message); the `project_root` parameter is used for `load_config()` to obtain the prefix. These are the same parameters already available in `load()`.
- Suffixes are parsed as `u32` via `str::parse::<u32>()`. Leading zeros are stripped during parsing (e.g., `WRK-001` → 1). Non-numeric suffixes and non-matching prefixes are filtered out by `filter_map`. This is identical to the existing `generate_next_id()` behavior.
- Warning output verification: `log_warn!` writes to stderr via `eprintln!`. The codebase does not capture stderr in tests. Integration tests verify that `load()` succeeds in all cases (behind, normal, empty). The pure function `max_item_suffix()` is tested directly via unit tests for correctness.

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
| 1 | Complete | `[WRK-031][P1] Feature: Add next_item_id validation warning on backlog load` | All 597 tests pass (89 backlog, 6 new unit, 5 new integration). Code review: no critical/high issues. |

## Followups Summary

### Critical

(none)

### High

(none)

### Medium

(none)

### Low

(none)

## Design Details

### Key Types

No new types introduced. Existing types used:

- `BacklogFile` (`src/types.rs:227-237`) — has `items: Vec<BacklogItem>` and `next_item_id: u32`
- `BacklogItem` (`src/types.rs`) — has `id: String` field used for suffix parsing
- `PhaseGolemConfig` (`src/config.rs`) — has `project.prefix: String` (default `"WRK"`)

### Architecture Details

Two new private functions in `src/backlog.rs`:

```rust
/// Compute the maximum numeric ID suffix across items matching the given prefix.
/// Returns 0 if no items match or the items slice is empty.
fn max_item_suffix(items: &[BacklogItem], prefix: &str) -> u32

/// Log a warning if next_item_id is behind the max item ID suffix.
/// Loads config for prefix. Skips silently if config loading fails.
fn warn_if_next_id_behind(backlog: &BacklogFile, path: &Path, project_root: &Path)
```

`generate_next_id()` is refactored to call `max_item_suffix()` — single source of truth for ID parsing logic.

### Design Rationale

- **Shared function over duplication:** Prevents the validation and generation logic from diverging if the ID format changes. `max_item_suffix()` becomes the single source of truth for "what is the highest item ID suffix?"
- **Helper function over inline:** DRY across two `load()` return paths; independently testable.
- **`()` return over `Result`:** Validation is advisory — must never block loading. The infallible return type enforces this at the type level. All internal failures (config load error, no items) are handled silently. The backlog is already loaded and usable regardless of validation outcome. If config is broken, it will surface through other code paths during normal operation (e.g., phase execution).
- **Config loaded inside helper:** Avoids threading config through `load()` or restructuring it. Redundant TOML parse on migration path is negligible. `load_config()` returns defaults when config file is missing (not an error); it only errors on parse failures of an existing file.

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
