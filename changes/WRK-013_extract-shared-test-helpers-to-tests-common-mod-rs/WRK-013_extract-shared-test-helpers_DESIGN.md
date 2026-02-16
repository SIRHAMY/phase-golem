# Design: Extract Shared Test Helpers to tests/common/mod.rs

**ID:** WRK-013
**Status:** In Review
**Created:** 2026-02-12
**PRD:** ./WRK-013_extract-shared-test-helpers_PRD.md
**Tech Research:** ./WRK-013_extract-shared-test-helpers_TECH_RESEARCH.md
**Mode:** Medium

## Overview

Extract duplicated test helper functions from 11 integration test files into a single shared `tests/common/mod.rs` module using Rust's standard integration test sharing pattern. The design uses a flat module with simple `pub fn` helpers — no builder pattern, no sub-modules. Each test file adds `mod common;` and calls `common::helper_name()`, replacing local copies. The canonical helpers use minimal defaults (e.g., `pipeline_type: None`, `phase_pool: None`) so that files needing richer defaults use lightweight local wrappers that mutate the result.

---

## System Design

### High-Level Architecture

The change introduces one new file (`tests/common/mod.rs`) and modifies 8-11 existing test files. No production code is touched.

```
tests/
├── common/
│   └── mod.rs          ← NEW: shared helpers (~120 lines)
├── backlog_test.rs     ← MODIFIED: removes local helpers, adds mod common;
├── coordinator_test.rs ← MODIFIED
├── executor_test.rs    ← MODIFIED
├── preflight_test.rs   ← MODIFIED
├── scheduler_test.rs   ← MODIFIED
├── agent_test.rs       ← MODIFIED (fixtures_dir)
├── migration_test.rs   ← MODIFIED (fixtures_dir)
├── prompt_test.rs      ← MODIFIED (fixture_path only; keeps local make_item)
├── git_test.rs         ← UNCHANGED (keeps local setup_temp_repo — different semantics)
├── config_test.rs      ← UNCHANGED
├── lock_test.rs        ← UNCHANGED
├── types_test.rs       ← UNCHANGED
├── worklog_test.rs     ← UNCHANGED
└── agent_integration_test.rs ← UNCHANGED
```

### Component Breakdown

#### Component: `tests/common/mod.rs`

**Purpose:** Single source of truth for test helper functions used by 3+ integration test files.

**Responsibilities:**
- Provide canonical item builders (`make_item`, `make_in_progress_item`)
- Provide backlog constructors (`make_backlog`, `empty_backlog`)
- Provide fixture path utilities (`fixtures_dir`, `fixture_path`)
- Provide git environment setup (`setup_test_env`)
- Provide config builders (`default_config`)

**Interfaces:**
- Input: Simple parameters (id strings, status enums, item vectors)
- Output: Fully constructed test data structs (`BacklogItem`, `BacklogFile`, `OrchestrateConfig`, `TempDir`, `PathBuf`)

**Dependencies:** `orchestrate` crate (types, config modules), `std`, `tempfile`

#### Component: Local Wrappers (in consuming test files)

**Purpose:** Adapt the canonical helpers to file-specific needs without polluting the shared module.

**Responsibilities:**
- Call `common::make_item()` and override fields (e.g., adding `pipeline_type`)
- Provide domain-specific helpers that don't belong in common (e.g., prompt_test's hardcoded InProgress builder)

**Decision criteria for local wrappers vs inline mutation:**
- **1-2 call sites** needing the same override in a file: use inline mutation (e.g., `item.pipeline_type = Some(...)`)
- **3+ call sites** needing the same override in a file: create a local wrapper function
- **Different semantics** (e.g., prompt_test's hardcoded InProgress): keep an independent local helper

**Interfaces:**
- Input: Same parameters as their canonical equivalents
- Output: Mutated versions of canonical structs

**Dependencies:** `common` module

### Data Flow

1. Test file declares `mod common;` as the first line (before all `use` statements)
2. Test function calls `common::make_item("WRK-001", ItemStatus::New)`
3. `common::make_item` constructs a `BacklogItem` with all 22 fields using minimal defaults
4. Test function optionally mutates specific fields for its test case
5. Test proceeds with constructed data

### Key Flows

#### Flow: Basic Item Construction

> Test creates a BacklogItem with specific id and status

1. **Call common** — `let item = common::make_item("WRK-001", ItemStatus::New);`
2. **Receive defaults** — Item has `pipeline_type: None`, `phase_pool: None`, timestamp `"2026-02-10T00:00:00+00:00"`, title `"Test item WRK-001"`
3. **Use directly** — Most tests use the item as-is

#### Flow: Item with Pipeline Type (executor_test, preflight_test)

> Tests that need `pipeline_type: Some("feature")` on their items

1. **Call common** — `let mut item = common::make_item("WRK-001", ItemStatus::New);`
2. **Override locally** — `item.pipeline_type = Some("feature".to_string());`
3. **Use item** — Item now has pipeline_type set

**Alternative:** These files define a local `make_item_with_pipeline` wrapper:
```rust
fn make_item_with_pipeline(id: &str, status: ItemStatus) -> BacklogItem {
    let mut item = common::make_item(id, status);
    item.pipeline_type = Some("feature".to_string());
    item
}
```

**Decision:** Whether to use inline mutation or a local wrapper depends on call site count per the criteria above. If 3+ calls in a file needing the same override, use a wrapper.

#### Flow: In-Progress Item with Phase

> Test creates an item that's in-progress with a phase assignment

1. **Call common** — `let item = common::make_in_progress_item("WRK-001", "prd");`
2. **Receive defaults** — Item has `status: InProgress`, `phase: Some("prd")`, `phase_pool: None`, `pipeline_type: None`
3. **Override if needed** — Files needing `phase_pool: Some(PhasePool::Main)` add: `item.phase_pool = Some(PhasePool::Main);` (coordinator_test, executor_test, scheduler_test)

This follows the "minimal defaults" principle consistently — callers explicitly opt into pool assignment rather than having to opt out.

#### Flow: Test Environment Setup

> Tests that need a git repo with project directory structure

1. **Call common** — `let dir = common::setup_test_env();`
2. **Receive TempDir** — Contains:
   - Initialized git repo with `user.name = "Test"`, `user.email = "test@test.com"`
   - Initial commit (single file)
   - Project directories: `_ideas/`, `_worklog/`, `changes/`, `.orchestrator/`
3. **Use path** — `dir.path()` for all file operations in test
4. **Cleanup** — Automatic via `TempDir::drop()` when variable goes out of scope

**Error handling:** All git operations use `.expect()` for fail-fast. If git is unavailable or an operation fails, the test panics immediately. No partial setup state to worry about.

**Edge case:** git_test.rs needs only a bare git repo without project directories. It keeps its local `setup_temp_repo()` function (1 file, different semantics — not a candidate for extraction).

#### Flow: Fixture File Access

> Test reads a fixture file from tests/fixtures/

1. **Call common** — `let path = common::fixture_path("sample_backlog.yaml");`
2. **Receive PathBuf** — `{CARGO_MANIFEST_DIR}/tests/fixtures/sample_backlog.yaml`
3. **Use path** — `fs::read_to_string(&path)` or pass to loading function

**Note:** `fixture_path(name)` is a convenience wrapper: `fixtures_dir().join(name)`. Both are provided because some tests need the directory path (e.g., to list files), while most need a specific file path. Neither validates that the path exists — callers handle errors at the point of use.

---

## Technical Decisions

### Key Decisions

#### Decision: Flat `common/mod.rs` vs Sub-modules

**Context:** Could organize helpers into `common/builders.rs`, `common/fixtures.rs`, etc.

**Decision:** Single flat `common/mod.rs` file.

**Rationale:** We're extracting ~10-12 helpers totaling ~120 lines. Sub-modules add import ceremony (`mod common; use common::builders::make_item;` vs `common::make_item()`) without organizational benefit at this scale.

**Consequences:** If helpers grow past ~20 functions, should split into sub-modules. For now, a single file keeps imports simple.

#### Decision: Canonical `make_item` Signature — `(id: &str, status: ItemStatus)`

**Context:** 7 files define `make_item` with 3 different signatures: `(id, status)` in 4 files, `(id, title, status)` in 1, `(id, title)` in 1.

**Decision:** Canonical is `make_item(id: &str, status: ItemStatus) -> BacklogItem` with auto-generated title `"Test item {id}"`.

**Rationale:** Used by the most files (4). Title is rarely important in tests — most tests care about id and status. Files needing a specific title can mutate `item.title` directly.

**Consequences:** scheduler_test keeps its local 3-param `make_item(id, title, status)` wrapper (used at many call sites — converting to mutation would add noise). prompt_test keeps its completely different local `make_item(id, title)` (hardcodes InProgress status and "prd" phase — domain-specific semantics, not a general-purpose builder).

#### Decision: Minimal Defaults for All Helpers

**Context:** Different files use different values for optional fields like `pipeline_type` and `phase_pool`. Two possible principles: "minimal defaults" (None/empty for optional fields) vs "realistic defaults" (values matching production behavior).

**Decision:** Consistently use minimal defaults — `None` for all optional fields unless the parameter is part of the function signature.

**Rationale:** A single consistent principle avoids confusion about which defaults are "safe" vs "opinionated." Callers explicitly set the fields they care about, making test intent visible at the call site. This is safer for refactoring — callers won't silently inherit defaults they didn't expect.

**Consequences:**
- `make_item`: `pipeline_type: None`, `phase_pool: None`
- `make_in_progress_item`: `phase_pool: None` — coordinator_test, executor_test, and scheduler_test add `item.phase_pool = Some(PhasePool::Main)` (or use a local wrapper if 3+ call sites)
- executor_test and preflight_test add `item.pipeline_type = Some("feature".to_string())` where needed

#### Decision: Timestamp Default — `"2026-02-10T00:00:00+00:00"`

**Context:** 3 different timestamps across files: `"2026-02-10T00:00:00+00:00"` (4 files), `"2026-01-01T00:00:00+00:00"` (2 files), `"2026-01-01T00:00:00Z"` (1 file — note different format suffix).

**Decision:** Use `"2026-02-10T00:00:00+00:00"` as the canonical default.

**Rationale:** Used by the majority (4 files). The specific timestamp value rarely matters — tests care about item relationships and status transitions, not specific dates.

**Pre-implementation requirement:** Before migrating any test file to use canonical timestamps, audit **all migrating test files** for assertions that compare timestamp strings (including serialized JSON/YAML output). Files with timestamp-dependent assertions keep local overrides. Pay special attention to prompt_test which uses a different format suffix (`Z` vs `+00:00`).

#### Decision: Keep `git_test.rs` Local

**Context:** git_test.rs has `setup_temp_repo()` which is a subset of `setup_test_env()` (no project directories).

**Decision:** git_test.rs keeps its local helper.

**Rationale:** Only 1 file uses it. The semantics differ (bare repo vs full project env). Adding it to common would create confusion about which setup to use.

**Consequences:** git_test.rs is unchanged by this refactoring.

#### Decision: `default_config` Extraction Scope

**Context:** 3 files define `default_config()` but with different pipeline configurations: executor_test and scheduler_test use `default_feature_pipeline()`, while preflight_test uses `feature_pipeline_no_workflows()`.

**Decision:** Extract only the executor_test/scheduler_test version to common. preflight_test keeps its local `default_config()`.

**Rationale:** Only compatible implementations should be extracted. Parameterizing the shared version to handle both pipeline types would add complexity without clear benefit.

**Consequences:** preflight_test retains its own `default_config()`. If preflight_test's config aligns with common's in the future, it can migrate then.

#### Decision: scheduler_test Handling

**Context:** scheduler_test uses 3-param variants of both `make_item(id, title, status)` and `make_in_progress_item(id, phase, title)`, plus a different timestamp (`"2026-01-01T00:00:00+00:00"`).

**Decision:** scheduler_test keeps its local `make_item` and `make_in_progress_item` variants. It migrates to common for `setup_test_env`, `make_backlog`, and `empty_backlog` only.

**Rationale:** The 3-param variants are used extensively in scheduler_test. Converting each call site to `common::make_item() + title override` would add noise without reducing duplication. The timestamp difference is another reason to keep local item builders.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Compilation cascade | Any change to `common/mod.rs` recompiles all importing test files | Single source of truth for test data | Build time is ~0.6s — cascade is negligible |
| Minimal defaults require overrides | Some files need 1-line mutation for pipeline_type or phase_pool | Consistent, safe defaults that don't assume test needs | Makes test intent explicit at call site |
| `common::` prefix at call sites | Slightly more verbose: `common::make_item(...)` vs `make_item(...)` | No naming conflicts during migration; clear provenance | Standard Rust pattern; muscle memory forms quickly |
| Some files keep local item builders | scheduler_test, prompt_test retain their own `make_item` | Cleaner code than forced adapter pattern | Domain-specific needs don't belong in shared module |

---

## Alternatives Considered

### Alternative: Builder Pattern for BacklogItem

**Summary:** Create a `BacklogItemBuilder` with fluent API: `BacklogItemBuilder::new("WRK-001").status(New).pipeline_type("feature").build()`.

**How it would work:**
- Define `BacklogItemBuilder` struct in `common/mod.rs` with all fields defaulted
- Each field has a setter method returning `&mut Self`
- `.build()` produces the final `BacklogItem`

**Pros:**
- Tests only specify fields they care about — very readable
- Adding new fields to BacklogItem only requires updating the builder's defaults
- Self-documenting test intent

**Cons:**
- ~60-80 additional lines of boilerplate for the builder struct + methods
- BacklogItem has 22 fields — 22 setter methods
- Over-engineering for this scale — the existing plain-function pattern works
- Tests already construct items via mutation when needed

**Why not chosen:** The PRD explicitly scopes this as "simple function extraction only — not builder objects." The existing helpers are plain functions. The builder pattern would be a design change, not a refactoring. If BacklogItem grew past ~30 fields or test complexity increased, this would become the right approach.

### Alternative: Sub-module Organization

**Summary:** Split `common/` into `common/builders.rs`, `common/env.rs`, `common/fixtures.rs`.

**How it would work:**
- `common/mod.rs` re-exports from sub-modules
- `common/builders.rs`: item and backlog constructors
- `common/env.rs`: `setup_test_env()`
- `common/fixtures.rs`: `fixture_path()`, `fixtures_dir()`

**Pros:**
- Better organization if helpers grow
- Separate concerns — builders vs environment vs fixtures

**Cons:**
- More files for ~120 lines of code
- More complex imports (unless re-exported from mod.rs)
- Over-organized for current scale

**Why not chosen:** With ~10-12 helpers totaling ~120 lines, sub-modules add structural overhead without readability gains. Can split later if common grows past ~20 helpers.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Timestamp standardization breaks assertions | Tests fail on string comparison | Low | **Pre-implementation:** grep all migrating test files for timestamp string literals in assertions. Files with matches keep local overrides. Pay attention to format differences (`Z` vs `+00:00`). |
| Naming conflict during migration | Compile error from duplicate definitions | Low | Migration order per file: (1) add `mod common;`, (2) update call sites to `common::` prefix, (3) remove local helper definition. Never have both a local definition and unqualified calls during transition. |
| BacklogItem struct changes | New required fields break `make_item` in common | Medium | `common::make_item()` must be updated when BacklogItem gains new fields. This is the intended benefit — update once instead of 7+ times. Run `cargo test` after any BacklogItem change. |

---

## Integration Points

### Existing Code Touchpoints

- `tests/backlog_test.rs` — Remove `make_item`, `make_in_progress_item`, `empty_backlog`, `fixture_path`; add `mod common;`; update call sites to `common::` prefix
- `tests/coordinator_test.rs` — Remove `setup_test_env`, `make_item`, `make_in_progress_item`, `empty_backlog`; add `mod common;`; update call sites
- `tests/executor_test.rs` — Remove `setup_test_env`, `make_item`, `make_in_progress_item`, `make_backlog`, `default_config`; add `mod common;`; add local wrapper for pipeline_type if 3+ call sites need it; update call sites
- `tests/preflight_test.rs` — Remove `make_item`, `make_backlog`; add `mod common;`; add local wrapper for pipeline_type if 3+ call sites need it; keep local `default_config` (uses different pipeline); update call sites
- `tests/scheduler_test.rs` — Remove `setup_test_env`, `make_backlog`, `empty_backlog`; add `mod common;`; **keep** local `make_item` and `make_in_progress_item` variants (3-param signatures, different timestamp); update backlog/env call sites only
- `tests/agent_test.rs` — Remove `fixtures_dir`; add `mod common;`; update `fixtures_dir()` calls to `common::fixtures_dir()`
- `tests/migration_test.rs` — Remove `fixtures_dir`; add `mod common;`; update calls
- `tests/prompt_test.rs` — Add `mod common;` for `fixture_path` if needed; **keep** local `make_item` (hardcoded InProgress semantics — domain-specific, not a general-purpose builder)
- `tests/git_test.rs` — **UNCHANGED**. Keeps local `setup_temp_repo()` (bare git repo without project directories — different semantics from `setup_test_env()`)

### External Dependencies

None. This change is entirely within the test suite and depends only on crate-internal types.

---

## Shared Module API

The canonical helpers in `tests/common/mod.rs`:

```rust
#![allow(dead_code)]  // Not all helpers are used by all test files

use orchestrate::{...};  // types needed
use std::path::PathBuf;
use tempfile::TempDir;

// --- Item Builders ---

/// Create a BacklogItem with minimal defaults.
/// Title auto-generated as "Test item {id}".
/// Timestamp: "2026-02-10T00:00:00+00:00".
/// All optional fields (pipeline_type, phase, phase_pool, etc.): None/empty.
/// Callers needing non-default values should mutate the returned item.
pub fn make_item(id: &str, status: ItemStatus) -> BacklogItem

/// Create an in-progress item with phase assignment.
/// Sets status=InProgress, phase=Some(phase).
/// phase_pool and pipeline_type default to None (callers set if needed).
pub fn make_in_progress_item(id: &str, phase: &str) -> BacklogItem

// --- Backlog Constructors ---

/// Create a BacklogFile from a list of items.
/// schema_version=2, next_item_id=0.
pub fn make_backlog(items: Vec<BacklogItem>) -> BacklogFile

/// Create an empty BacklogFile. Equivalent to make_backlog(vec![]).
pub fn empty_backlog() -> BacklogFile

// --- Fixture Paths ---

/// Path to the tests/fixtures/ directory.
pub fn fixtures_dir() -> PathBuf

/// Path to a specific fixture file: fixtures_dir().join(name).
/// Does NOT verify the file exists — callers handle errors at point of use.
pub fn fixture_path(name: &str) -> PathBuf

// --- Environment Setup ---

/// Create a temp directory with an initialized git repo and project directories.
/// Git config: user.name="Test", user.email="test@test.com"
/// Initial commit: single file committed
/// Directories created: _ideas/, _worklog/, changes/, .orchestrator/
/// All git operations use .expect() — panics on failure (fail-fast).
/// Cleanup: automatic via TempDir::drop()
pub fn setup_test_env() -> TempDir

// --- Config Builders ---

/// Create an OrchestrateConfig with default settings and a "feature" pipeline
/// using default_feature_pipeline(). Only compatible with executor_test and
/// scheduler_test patterns. preflight_test keeps its own local default_config().
pub fn default_config() -> OrchestrateConfig
```

---

## Open Questions

None — all resolved during PRD, tech research, and self-critique phases.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain
- [x] Default value principles are consistent across all helpers

---

## Assumptions

Decisions made without human input (autonomous mode):

1. **Mode: medium** — Well-understood refactoring, but warrants documenting alternatives and tradeoffs.
2. **Migration strategy: file-by-file** — Each test file is migrated independently: add `mod common;`, update call sites, remove local helpers, verify compilation. This allows incremental progress and easy rollback.
3. **Local wrappers over parameter proliferation** — Rather than adding optional parameters to canonical helpers (e.g., `make_item(id, status, pipeline_type)`), files use 1-line mutation. Keeps the common API stable and minimal.
4. **`#[allow(dead_code)]` on common module** — Rust integration tests compile each file as a separate crate. The common module is compiled into each, but not all functions are used by all files. A module-level `#![allow(dead_code)]` prevents unused function warnings.
5. **scheduler_test keeps local item builders** — Its 3-param `make_item(id, title, status)` and `make_in_progress_item(id, phase, title)` variants are used extensively. Converting all to `common::` + mutation would add noise. It migrates only `setup_test_env`, `make_backlog`, and `empty_backlog`.
6. **Consistent minimal defaults** — All canonical helpers use `None`/empty for optional fields. This avoids the inconsistency of some helpers being "minimal" (pipeline_type: None) while others are "realistic" (phase_pool: Some(Main)). Callers explicitly set what they need.
7. **preflight_test keeps local default_config** — Its config uses a different pipeline type (`feature_pipeline_no_workflows`), incompatible with the common version.
8. **prompt_test keeps local make_item** — Its version hardcodes `status: InProgress` and `phase: Some("prd")` because all prompt tests operate on in-progress items. This is domain-specific, not a variant of the general-purpose builder.
9. **Timestamp audit required before migration** — All migrating test files must be checked for timestamp-dependent assertions before switching to canonical defaults.

---

## Self-Critique Summary

The initial design was reviewed by 7 internal critique agents. Key improvements applied:

**Auto-fixes applied:**
1. **Resolved contradictory default principles** — Changed `make_in_progress_item` from `phase_pool: Some(Main)` to `phase_pool: None` to match the "minimal defaults" principle used for `pipeline_type: None`. Consistency matters more than matching majority usage.
2. **Clarified scheduler_test migration scope** — Explicitly stated it keeps local `make_item` AND `make_in_progress_item` (both have 3-param signatures), migrating only backlog/env helpers.
3. **Clarified default_config extraction boundary** — Explicitly stated preflight_test keeps its local version due to incompatible pipeline type.
4. **Broadened timestamp audit scope** — Changed from "audit scheduler_test and prompt_test" to "audit all migrating test files."
5. **Fixed #[allow(dead_code)] explanation** — Removed contradictory statement about warnings. Clarified that module-level `#![allow(dead_code)]` is the solution.
6. **Added git_test.rs to Integration Points** — Marked as UNCHANGED with rationale.
7. **Documented fixture_path/fixtures_dir relationship** — Clarified one wraps the other.
8. **Added setup_test_env postconditions** — Git user, commit content, error handling, cleanup.
9. **Added mod common; placement convention** — First line before `use` statements.
10. **Added migration criteria for local wrapper vs inline mutation** — Clear rules based on call site count.
11. **Added BacklogItem struct change risk** — New risk for when BacklogItem gains fields.
12. **Removed unused import warnings risk** — Superseded by `#![allow(dead_code)]` decision.

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Initial design draft | Flat common/mod.rs with 8 helpers, minimal defaults, local wrappers for overrides |
| 2026-02-12 | Self-critique (7 agents) | 12 auto-fixes applied; resolved default principle inconsistency; clarified migration scope for scheduler_test, preflight_test, prompt_test |
