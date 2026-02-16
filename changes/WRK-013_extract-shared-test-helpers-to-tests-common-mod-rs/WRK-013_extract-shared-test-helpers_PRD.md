# Change: Extract Shared Test Helpers to tests/common/mod.rs

**Status:** Proposed
**Created:** 2026-02-12
**Author:** Claude (autonomous)

## Problem Statement

The `orchestrate` Rust project has 14 integration test files totaling ~9,946 lines, with an estimated 370+ lines of duplicated helper functions across 11 of those files. The same item builders, backlog constructors, configuration factories, and git environment setup functions are copied between test files with variations in function signatures, default field values, and naming conventions.

Specific duplication observed:

| Helper | Files duplicated in | Lines per copy | Key variations |
|--------|-------------------|----------------|----------------|
| `make_item()` | 7 files | ~20-25 | 3 different signatures: `(id, status)`, `(id, title, status)`, `(id, title)` |
| `make_in_progress_item()` | 5 files | ~6-10 | Some set `pipeline_type`, others don't |
| `setup_test_env()` | 3 files | ~58 | Identical implementations |
| `empty_backlog()` / `make_backlog()` | 5 files | ~5-6 | Consistent |
| `default_config()` / `default_pipelines()` | 3-4 files | ~20-30 | Varying pipeline definitions |
| `fixture_path()` | 1 file | ~3-5 | Only in `backlog_test.rs`, useful broadly |
| Timestamps | 7+ files | n/a | `"2026-02-10T00:00:00+00:00"` (4 files) vs `"2026-01-01T00:00:00+00:00"` (2 files) vs `"2026-01-01T00:00:00Z"` (1 file) |

This duplication creates three concrete problems:

1. **Maintenance burden**: When `BacklogItem` (the core work item struct with ~23 fields) gains a new field, the `make_item()` helper must be updated in 7+ separate files. Missing one causes a compile error that requires hunting across files.
2. **Inconsistent test data**: Different files use different default values for the same struct fields, which can mask bugs or make test behavior harder to reason about.
3. **Friction for new tests**: Writing a new test file requires finding and copying helpers from existing files, with no single source of truth for test data construction.

Rust has a standard pattern for sharing code between integration tests: `tests/common/mod.rs`. In Rust, each file in `tests/` compiles as a separate crate; `mod common;` is the idiomatic way to share helper code across them. This change extracts helpers appearing in 3+ files into that shared module.

## User Stories / Personas

- **Project maintainer** - Wants to update `BacklogItem` struct fields without hunting through 7+ test files to fix compilation errors. Wants consistent test data across the suite.
- **Future contributor** - Wants a single, discoverable location for test helpers when writing new integration tests, rather than copying helpers from random existing test files.

## Desired Outcome

After this change:
- A `tests/common/mod.rs` file exists containing shared test helper functions.
- Test files that previously defined their own copies of these helpers instead import from `common` via `mod common;` and call `common::make_item(...)`.
- All existing tests continue to pass (`cargo test` succeeds with no regressions).
- New test files can add `mod common;` and immediately access standard item builders, backlog constructors, and environment setup helpers.

## Success Criteria

### Must Have

- [ ] `tests/common/mod.rs` exists with shared helper functions
- [ ] Item creation helpers extracted with canonical signature `make_item(id: &str, status: ItemStatus) -> BacklogItem` (auto-generates title as `"Test item {id}"`), used by 3+ test files. Files needing `(id, title, status)` or `(id, title)` signatures keep local wrappers that call `common::make_item()` and override the title field.
- [ ] `make_in_progress_item(id: &str, phase: &str) -> BacklogItem` extracted, used by 3+ test files
- [ ] Backlog construction helpers (`empty_backlog`, `make_backlog`) extracted and used by 3+ test files
- [ ] All existing tests pass (`cargo test` succeeds with no regressions)
- [ ] No duplicate definitions remain for extracted helpers — verified by confirming each consuming test file has `mod common;` and no local definition of the same function name
- [ ] Fixture path helper (`fixture_path(name: &str) -> PathBuf`) available in the shared module. Currently only defined in `backlog_test.rs` but useful for any test that reads from `tests/fixtures/`.

### Should Have

- [ ] Configuration builders (`default_config`, `default_pipelines`) extracted where signatures are compatible across files
- [ ] Git test environment setup (`setup_test_env`) extracted from the 3 files that share identical implementations (`coordinator_test.rs`, `executor_test.rs`, `scheduler_test.rs`)
- [ ] Phase result builders (`phase_complete_result`, `failed_result`, `blocked_result`) extracted where used by 2+ files. A phase result (PhaseResult) is the structured output from running a workflow phase, containing result code, summary, and optional context.
- [ ] Default timestamp standardized to `"2026-02-10T00:00:00+00:00"` (used by 4 of 7 files). Before applying: audit all test assertions to confirm none depend on specific timestamp values; if any do, those tests keep local overrides.

### Nice to Have

- [ ] Extended item builders (`make_scoping_item`, `make_ready_item`, `make_blocked_item`) extracted for less-frequently-used variants
- [ ] Doc comments on shared helpers explaining purpose, parameters, and default values chosen

## Scope

### In Scope

- Creating `tests/common/mod.rs` with extracted helper functions
- Updating existing test files to add `mod common;` and replace local helper calls with `common::` prefixed calls
- Standardizing default values where no test assertions depend on the specific values being changed
- Removing the now-redundant local helper definitions from test files
- Adding local wrapper functions in test files that need non-canonical signatures (e.g., a `make_item_with_title()` that calls `common::make_item()` and overrides the title)

### Out of Scope

- Modifying production code in `src/`
- Adding or removing test cases (functional test coverage stays the same)
- Introducing a builder pattern or other API design changes (simple function extraction only — the existing helpers are plain functions, not builder objects)
- Modifying test fixtures in `tests/fixtures/`
- Adding `#[cfg(test)]` guards to `MockAgentRunner` in production code (existing tech debt, separate concern)
- Refactoring test logic or assertions (beyond updating helper call sites)
- Changing Cargo.toml dependencies

## Non-Functional Requirements

- **Compilation time:** Changes to `common/mod.rs` trigger recompilation of all test files that import it. Current full test build is ~0.6s. Expected impact is negligible, but incremental rebuilds after editing `common/mod.rs` will recompile all importing test crates.
- **Maintainability:** Single source of truth for test data construction. Future `BacklogItem` field additions require updating one file instead of 7+.

## Constraints

- Must use the standard Rust `tests/common/mod.rs` pattern. In Rust's test organization, each file in `tests/` compiles as an independent binary crate. The `tests/common/` directory with a `mod.rs` file is the standard way to share code, imported via `mod common;` in each test file.
- Helper functions in `common/mod.rs` must be `pub` to be accessible from test files.
- Cannot use `#[cfg(test)]` in `tests/` directory (all code there is already test-only).
- Must preserve existing test behavior — this is a pure refactoring. Test assertions, coverage, and outcomes must remain identical. Standardizing default values is acceptable only where no assertions depend on the specific values being changed.

## Dependencies

- **Depends On:** Nothing. This is a self-contained refactoring of the test suite.
- **Blocks:** Nothing directly, but improves velocity for any future work that adds or modifies tests.

## Risks

- [ ] **Function signature variations**: `make_item()` has 3 different signatures across 7 files. The `(id, status)` 2-param version is used in 4 files (backlog_test, coordinator_test, executor_test, preflight_test) and is the canonical choice. Files using `(id, title, status)` (scheduler_test) or `(id, title)` (prompt_test) will keep local wrappers. Mitigation: canonical signature in `common`, local wrappers where needed.
- [ ] **Timestamp default inconsistency**: 3 different timestamp strings across files. Changing defaults could cause assertion failures. Mitigation: audit all assertions that reference timestamps before standardizing; any test asserting on a specific timestamp value keeps a local override.
- [ ] **Compilation cascade**: Changes to `common/mod.rs` trigger recompilation of all 14 test files. Mitigation: acceptable trade-off given ~0.6s build time. Keep the common module stable — only broadly-used helpers belong there.
- [ ] **Naming conflicts during migration**: If a test file defines `make_item()` locally AND imports `mod common;`, Rust will require disambiguation. Mitigation: remove local definitions before adding `common::make_item()` calls, or use the `common::` prefix at call sites during migration.

## Open Questions

(None remaining — all resolved during self-critique. See Assumptions for decisions made.)

## Assumptions

Decisions made without human input (autonomous mode):

1. **Mode: medium** — This is a well-understood refactoring with clear scope, but warrants moderate exploration to identify all duplication patterns and risks.
2. **No builder pattern** — Simple function extraction is sufficient. The existing helpers are plain functions (not builder objects), and a builder pattern would add complexity without proportional benefit at this scale.
3. **`tests/common/mod.rs` pattern** — Using the directory-based module pattern per Rust convention for integration test shared code.
4. **Extraction threshold** — Must-have: helpers appearing in 3+ files. Should-have: helpers appearing in 2+ files. Helpers used by only 1 file remain local.
5. **Canonical `make_item()` signature: `(id: &str, status: ItemStatus)`** — Used in 4 of 7 files. Auto-generates title as `"Test item {id}"`. Files needing different signatures keep local wrappers.
6. **Canonical timestamp: `"2026-02-10T00:00:00+00:00"`** — Used by 4 of 7 files. Will audit assertions before applying; tests with timestamp-dependent assertions keep local overrides.
7. **`setup_test_env()` in Should Have** — High line savings (58 lines × 3 files = 174 lines) but involves git process spawning, making it slightly higher complexity than pure data builders. Included in Should Have rather than Must Have.
8. **`fixture_path()` extraction despite single current user** — Currently only in `backlog_test.rs`, but it's a broadly useful utility (any test reading fixtures benefits). Including in Must Have to establish the pattern.

## References

- [The Rust Programming Language §11.3 - Test Organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html)
- Orchestrator project: `.claude/skills/changes/orchestrator/`
- Test files: `.claude/skills/changes/orchestrator/tests/`
