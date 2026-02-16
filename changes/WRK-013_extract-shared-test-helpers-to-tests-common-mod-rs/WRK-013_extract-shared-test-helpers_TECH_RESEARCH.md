# Tech Research: Extract Shared Test Helpers to tests/common/mod.rs

**ID:** WRK-013
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-013_extract-shared-test-helpers_PRD.md
**Mode:** Medium

## Overview

Research into patterns and approaches for extracting duplicated test helper functions from 14 integration test files into a shared `tests/common/mod.rs` module in a Rust project. The goal is to validate the PRD's approach, verify its duplication claims against actual code, identify pitfalls, and confirm the `tests/common/mod.rs` pattern is the right tool for this job.

## Research Questions

- [x] Is `tests/common/mod.rs` the right pattern for sharing test helpers in Rust integration tests?
- [x] What are the exact duplication patterns in the codebase? Does the PRD match reality?
- [x] What are the gotchas with this pattern (compilation cascade, visibility, naming conflicts)?
- [x] Should we use simple function extraction or a builder pattern?
- [x] What canonical signatures should the shared helpers use?
- [x] Are there timestamp-dependent assertions that would break if we standardize defaults?

---

## External Research

### Landscape Overview

Rust has a distinctive integration test model where each file in `tests/` compiles as a separate binary crate. This provides excellent isolation but creates challenges for sharing code. The community has converged on the `tests/common/mod.rs` pattern as the canonical solution, documented in the official Rust book. More sophisticated approaches exist (rstest fixtures, builder patterns, fake data generation) but are appropriate for larger or more complex test suites.

### Common Patterns & Approaches

#### Pattern: tests/common/mod.rs (Canonical)

**How it works:** Create `tests/common/` directory with `mod.rs` containing `pub` helper functions. Each test file imports via `mod common;` and calls `common::helper_name()`.

**When to use:** Default choice for any Rust project with 3+ integration test files sharing code.

**Tradeoffs:**
- Pro: Zero dependencies, official pattern, simple to understand, universally applicable
- Pro: Files in subdirectories of `tests/` are NOT compiled as separate test crates
- Con: Changes to `common/mod.rs` trigger recompilation of all importing test files
- Con: No advanced features like parameterization or dependency injection

**References:**
- [Test Organization - The Rust Programming Language](https://doc.rust-lang.org/book/ch11-03-test-organization.html)
- [Integration testing - Rust By Example](https://doc.rust-lang.org/rust-by-example/testing/integration_testing.html)

#### Pattern: Builder Pattern for Test Fixtures

**How it works:** Create builder structs with methods for setting fields, sensible defaults for all fields, and a `.build()` method. Tests only specify fields relevant to what they're testing.

**When to use:** When test data structures have 5+ fields and different tests need different combinations.

**Tradeoffs:**
- Pro: Extremely flexible, tests are self-documenting, reduces cognitive load
- Con: More upfront code, slight runtime overhead, overkill for simple cases

**References:**
- [Testing Rust: Using the builder pattern - Dan Munckton](https://dan.munckton.co.uk/blog/2018/03/01/testing-rust-using-the-builder-pattern-for-complex-fixtures/)

#### Pattern: rstest Fixture Framework

**How it works:** Use `#[rstest]` attribute for test functions, define reusable fixtures with `#[fixture]`, inject via function parameters.

**When to use:** Complex setup/teardown logic, parameterized tests, fixture dependency injection.

**Tradeoffs:**
- Pro: Very ergonomic, powerful parameterization
- Con: External dependency, attribute macro complexity

**References:**
- [rstest - GitHub](https://github.com/la10736/rstest)

### Standards & Best Practices

1. **Use `tests/common/mod.rs`** (directory pattern), NOT `tests/common.rs` — the file pattern would be compiled as a test crate showing "running 0 tests"
2. **All helpers must be `pub`** — test files are separate crates that import `common`
3. **Extraction threshold of 3+** — only extract helpers used by 3+ files to keep common module stable
4. **Document helper defaults** — explain why specific default values were chosen
5. **Treat common module as semi-stable API** — minimize churn since changes cascade

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Using `tests/common.rs` instead of `tests/common/mod.rs` | Cargo treats it as a test crate, runs it showing "0 tests" | Always use directory pattern |
| Forgetting `pub` on helpers | Functions default to private; test crates can't access them | Mark all shared helpers `pub` |
| Naming conflicts during migration | Local `make_item()` + `mod common;` requires disambiguation | Remove local definitions before adding `common::` calls, or use `common::` prefix |
| Compilation cascade on common changes | Every importing test file recompiles | Keep common module stable; only broadly-used helpers belong there |
| Inconsistent defaults across duplicates | Tests may pass locally but fail elsewhere depending on defaults | Single source of truth in common; audit assertions before standardizing |

### Key Learnings

- The `tests/common/mod.rs` pattern is well-established, zero-dependency, and fits this exact use case
- Builder pattern is overkill here — simple function extraction matches the existing helper style
- The compilation cascade (14 files recompiling on common change) is negligible at 0.6s build time
- The biggest risk is timestamp standardization breaking assertions — requires audit

---

## Internal Research

### Existing Codebase State

**Test suite:** 14 test files, ~9,946 total lines, no existing shared test infrastructure.

**Architecture:**
- Production code in `src/` with 14 public modules
- No `tests/common/` directory exists — clean slate
- Test dependency: `tempfile = "3"` (only test-specific dep in Cargo.toml)
- Import pattern: `use orchestrate::{module};`
- Error handling: `.unwrap()` / `.expect()` throughout (fail-fast)

### Complete Helper Catalog

Verified through direct code review of all 14 test files:

#### `make_item()` — Item builder (7 files)

**Variant 1: `(id: &str, status: ItemStatus)` — 4 files**

| File | Lines | pipeline_type | Timestamp | Other differences |
|------|-------|---------------|-----------|-------------------|
| `backlog_test.rs:19-44` | 26 | `None` | `2026-02-10T00:00:00+00:00` | — |
| `coordinator_test.rs:64-89` | 26 | `None` | `2026-02-10T00:00:00+00:00` | — |
| `executor_test.rs:67-92` | 26 | `Some("feature")` | `2026-02-10T00:00:00+00:00` | Sets pipeline_type |
| `preflight_test.rs:14-39` | 26 | `Some("feature")` | `2026-02-10T00:00:00+00:00` | Sets pipeline_type |

- backlog_test + coordinator_test are **exact duplicates**
- executor_test + preflight_test are **exact duplicates** of each other (add `pipeline_type: Some("feature")`)

**Variant 2: `(id: &str, title: &str, status: ItemStatus)` — 1 file**

| File | Lines | Timestamp | Other differences |
|------|-------|-----------|-------------------|
| `scheduler_test.rs:72-97` | 26 | `2026-01-01T00:00:00+00:00` | Different timestamp |

**Variant 3: `(id: &str, title: &str)` — 1 file**

| File | Lines | Timestamp | Other differences |
|------|-------|-----------|-------------------|
| `prompt_test.rs:23-48` | 26 | `2026-01-01T00:00:00Z` | Hardcodes `status: InProgress`, `phase: Some("prd")` — not a general-purpose builder |

**Note:** prompt_test's version hardcodes status to InProgress and phase to "prd" regardless of any parameter. This is intentional for prompt tests (all items are in-progress with a phase), not a bug.

#### `make_in_progress_item()` — InProgress item builder (5 files)

**Variant 1: `(id: &str, phase: &str)` based on 2-param make_item — 4 files**

| File | Lines | Sets phase_pool? | Sets pipeline_type? |
|------|-------|-----------------|---------------------|
| `backlog_test.rs:46-50` | 5 | No | No |
| `coordinator_test.rs:91-97` | 7 | Yes (`Main`) | Yes (`"feature"`) |
| `executor_test.rs:94-99` | 6 | Yes (`Main`) | No (inherits from make_item which sets it) |
| `scheduler_test.rs:99-105` (3-param variant) | 7 | Yes (`Main`) | No |

**Key difference:** backlog_test's version only sets phase. The other 3 also set `phase_pool = Main`. executor_test's `make_item` already sets `pipeline_type`, so its `make_in_progress_item` doesn't need to.

#### `setup_test_env()` / `setup_temp_repo()` — Git env setup (4 files)

| File | Name | Lines | Creates project dirs? |
|------|------|-------|-----------------------|
| `coordinator_test.rs:16-58` | `setup_test_env` | 43 | Yes (`_ideas`, `_worklog`, `changes`, `.orchestrator`) |
| `executor_test.rs:23-65` | `setup_test_env` | 43 | Yes |
| `scheduler_test.rs:24-66` | `setup_test_env` | 43 | Yes |
| `git_test.rs:7-45` | `setup_temp_repo` | 39 | **No** — only creates git repo + initial commit |

- The 3 `setup_test_env` copies are **exact duplicates**
- `setup_temp_repo` in git_test is a **subset** — same git setup but no project directory creation

#### `empty_backlog()` / `make_backlog()` — Backlog constructors (5 files)

| File | Function | Lines |
|------|----------|-------|
| `backlog_test.rs:52-58` | `empty_backlog()` | 7 |
| `coordinator_test.rs:106-112` | `empty_backlog()` | 7 |
| `executor_test.rs:108-114` | `make_backlog(items)` | 7 |
| `preflight_test.rs:41-47` | `make_backlog(items)` | 7 |
| `scheduler_test.rs:119-125` | `make_backlog(items)` | 7 |

All implementations are **identical** (same struct, same defaults). `empty_backlog()` is just `make_backlog(vec![])`.

#### `default_config()` — Config builder (3 files)

| File | Lines | Notes |
|------|-------|-------|
| `executor_test.rs:139-145` | 7 | Uses `OrchestrateConfig::default()` + insert feature pipeline |
| `scheduler_test.rs:127-135` | 9 | Same but checks `is_empty()` first |
| `preflight_test.rs:102-108` | 7 | Uses `feature_pipeline_no_workflows()` instead of `default_feature_pipeline()` |

**Not identical:** preflight_test uses a different pipeline (no workflows). executor_test and scheduler_test are near-duplicates.

#### `default_pipelines()` — Pipeline map builder (2 files)

| File | Lines |
|------|-------|
| `scheduler_test.rs:147-151` | 5 |
| `prompt_test.rs:60-67` | 8 |

Both create `HashMap` with single "feature" entry using `default_feature_pipeline()`. **Near-identical.**

#### `fixture_path()` / `fixtures_dir()` — Path helpers (3 files)

| File | Function | Returns |
|------|----------|---------|
| `backlog_test.rs:13-17` | `fixture_path(name)` | Full path to specific fixture file |
| `agent_test.rs:13-15` | `fixtures_dir()` | Path to fixtures directory |
| `migration_test.rs:11-13` | `fixtures_dir()` | Path to fixtures directory |

**Near-duplicates:** `fixture_path(name)` = `fixtures_dir().join(name)`.

#### Phase result builders (3 files)

| File | Functions | Lines |
|------|-----------|-------|
| `executor_test.rs` | `make_phase_result(item_id, phase, result)` | 13 |
| `scheduler_test.rs` | `phase_complete_result`, `failed_result`, `blocked_result`, `subphase_complete_result`, `triage_result_with_assessments` | ~80 total |
| `agent_test.rs` | `make_result(result_code, summary)` | 13 |

Different signatures but overlapping purpose. scheduler_test's specific builders could call a shared generic builder.

#### Single-use helpers (NOT candidates for extraction)

- `make_blocked_item` — coordinator_test only
- `make_ready_item` — scheduler_test only
- `make_test_item` — worklog_test only
- `make_snapshot` — scheduler_test only
- `run_params` — scheduler_test only
- `default_guardrails` — executor_test only
- `save_and_commit_backlog` — coordinator_test only
- `valid_result_json` — agent_test only
- `make_phase_config`, `default_prd_config`, `make_item_with_assessments` — prompt_test only
- `feature_pipeline_no_workflows` — preflight_test only
- Various config_test helpers — config_test only

### Existing Patterns

1. **No tests/common/ exists** — clean slate
2. **Import convention:** `use orchestrate::{module};` at top
3. **TempDir cleanup:** Auto-cleanup via `TempDir::new().unwrap()` (Drop trait)
4. **Helper section markers:** Files use `// --- Test helpers ---` or `// --- Helpers ---`
5. **Struct-literal construction:** All `make_item` variants construct `BacklogItem { ... }` with all 22 fields listed explicitly
6. **Mutation pattern:** Derived helpers (e.g., `make_in_progress_item`) call base helper then mutate fields

### Constraints

1. Each file in `tests/` compiles as a separate binary crate — must use `tests/common/mod.rs` pattern
2. All helpers in common must be `pub fn`
3. `BacklogItem` has 22 fields — all must be specified in struct literal (no `..Default::default()` unless BacklogItem implements Default)
4. No `#[cfg(test)]` needed in tests/ — all code is already test-only
5. Tests must continue to pass identically — pure refactoring

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| `make_item()` has 3 signatures used in 7 files | Confirmed: 4 files use `(id, status)`, 1 uses `(id, title, status)`, 1 uses `(id, title)` with hardcoded status/phase. But within the 4-file group, 2 set `pipeline_type` and 2 don't. | The canonical `(id, status)` signature with `pipeline_type: None` works for backlog_test + coordinator_test. executor_test + preflight_test need local wrappers that add `pipeline_type`. |
| `setup_test_env()` in 3 files, identical | Actually **4 files** — git_test.rs has `setup_temp_repo()` which is a subset (no project dirs). | Consider extracting the git-init core as one function, with `setup_test_env()` adding project directories on top. Or keep git_test's version local since it's a subset. |
| `make_in_progress_item()` in 5 files, some set pipeline_type | Confirmed 4 files (backlog_test doesn't set phase_pool; other 3 do). Scheduler has 3-param variant. | Canonical version should set `phase_pool = Main` (3 of 4 files). backlog_test keeps local wrapper without phase_pool. |
| `default_config()` in 3-4 files with ~20-30 lines | 3 files but only 5-9 lines each (not 20-30). preflight_test uses different pipeline. | Only executor_test + scheduler_test have compatible implementations. preflight_test's `default_config()` is domain-specific. |
| Line savings of 370+ | Verified: ~26 lines × 4 for make_item + ~6 × 4 for make_in_progress + 43 × 3 for setup_test_env + 7 × 5 for backlog + fixture_path ≈ 290+ lines from must-haves alone | PRD estimate is reasonable. Actual savings depend on how many should-have items are extracted. |

---

## Critical Areas

### Canonical make_item Signature Decision

**Why it's critical:** This is the most duplicated helper (7 files) and has the most variation. The canonical signature determines how many local wrappers are needed.

**Why it's easy to miss:** The 4 files using `(id, status)` aren't all identical — 2 set `pipeline_type: Some("feature")` and 2 set `pipeline_type: None`. This split needs to be handled.

**What to watch for:**
- Canonical version should use `pipeline_type: None` (the minimal default)
- executor_test and preflight_test create local wrappers that add pipeline_type
- prompt_test and scheduler_test keep their own `make_item` variants (different signatures entirely)

### Timestamp Audit Before Standardization

**Why it's critical:** Changing default timestamps could silently break assertion-dependent tests.

**Why it's easy to miss:** Timestamp values appear in struct construction but may also appear in test assertions comparing serialized output.

**What to watch for:**
- Search for `2026-01-01` and `2026-02-10` in assertion lines (not just helper definitions)
- scheduler_test uses `2026-01-01` — check if any of its assertions depend on this value
- prompt_test uses `2026-01-01T00:00:00Z` (different format) — format matters for string comparison

### make_in_progress_item Variation

**Why it's critical:** The backlog_test version is simpler (no phase_pool, no pipeline_type) than the other 3 files.

**Why it's easy to miss:** If the canonical version sets `phase_pool = Main`, backlog_test's tests might start passing for wrong reasons or behave differently.

**What to watch for:**
- Decide: should canonical set `phase_pool = Main` (majority pattern) or not?
- If yes, audit backlog_test assertions that might depend on `phase_pool` being `None`

---

## Deep Dives

### git_test.rs setup_temp_repo vs setup_test_env

**Question:** Should git_test.rs use the shared `setup_test_env()` or keep its own `setup_temp_repo()`?

**Summary:** `setup_temp_repo()` creates a git repo without project directories (`_ideas`, `_worklog`, `changes`, `.orchestrator`). The git tests only need a bare git repo, not a full orchestrate project structure. Adding unnecessary directories wouldn't break git tests, but it's not semantically correct.

**Implications:** Two options:
1. Extract `setup_git_repo()` (git-only) and `setup_test_env()` (git + project dirs, calls `setup_git_repo()` internally) — cleaner but more complex
2. Keep git_test.rs's `setup_temp_repo()` local (only 1 file uses it) and extract `setup_test_env()` for the 3 identical copies — simpler, pragmatic

Recommend option 2: keep git_test local, extract the 3-file `setup_test_env()`.

### prompt_test.rs make_item — Hardcoded status

**Question:** Is it a bug that prompt_test's `make_item(id, title)` ignores the status parameter and hardcodes `InProgress`?

**Summary:** prompt_test's `make_item` takes `(id, title)` — it doesn't take a status parameter at all. It hardcodes `status: InProgress` and `phase: Some("prd")` because all prompt tests operate on in-progress items with a phase. This is intentional and domain-specific, not a bug.

**Implications:** prompt_test should keep its local `make_item` since it has completely different semantics than the canonical `(id, status)` version. It could be renamed to `make_prompt_item` for clarity, but that's optional.

---

## Synthesis

### Open Questions

| Question | Why It Matters | Resolution |
|----------|----------------|------------|
| Should canonical `make_item` set `pipeline_type`? | 2 of 4 using-files set it, 2 don't | **No** — use `None` as default. Files needing it add local wrapper. Matches minimal-default principle. |
| Should canonical `make_in_progress_item` set `phase_pool`? | 3 of 4 files set `PhasePool::Main` | **Yes** — majority pattern. backlog_test can override to `None` if needed (audit first). |
| Should `setup_temp_repo` (git_test) use shared helper? | It's a subset of `setup_test_env` | **No** — keep local. Only 1 file uses it, and it has different semantics (no project dirs). |
| Should `default_config` be extracted? | 3 files but preflight uses different pipeline | **Partially** — extract executor+scheduler version. preflight keeps local. |
| Should timestamps be standardized in this change? | Risk of breaking assertions | **Yes, with audit** — verify no assertion depends on specific timestamp values before changing. |

### Recommended Approaches

#### Helper Organization: Flat vs Sub-modules

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Single `common/mod.rs` | Simplest, single `mod common;` import | Can grow unwieldy | ≤20 helpers (our case) |
| Sub-modules (`common/builders.rs`, `common/fixtures.rs`) | Better organization | More import ceremony | 20+ helpers |

**Initial recommendation:** Single flat `common/mod.rs`. We're extracting ~10-12 helpers — sub-modules would be over-engineering.

#### make_item Canonical Signature

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `(id, status)` with `pipeline_type: None` | Minimal default, matches 2 of 4 files exactly | executor/preflight need wrappers | Want simplest possible default |
| `(id, status)` with `pipeline_type: Some("feature")` | Matches executor/preflight exactly | backlog/coordinator need to override to None | Most callers need pipeline_type |

**Initial recommendation:** `pipeline_type: None`. The canonical helper should be the most general. Files needing `pipeline_type` add it.

#### make_in_progress_item Canonical

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Set `phase_pool = Main` | Matches 3/4 files, represents real orchestrator behavior | backlog_test may need local override | Real items are always in a pool |
| Don't set `phase_pool` | Simpler, matches backlog_test | 3 files need to add it | Minimal default |

**Initial recommendation:** Set `phase_pool = Some(PhasePool::Main)` — this matches the majority pattern and represents how in-progress items actually work in the orchestrator.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Test Organization - Rust Book](https://doc.rust-lang.org/book/ch11-03-test-organization.html) | Official docs | Canonical reference for `tests/common/mod.rs` pattern |
| [Integration testing - Rust By Example](https://doc.rust-lang.org/rust-by-example/testing/integration_testing.html) | Official docs | Concise examples |
| [Testing Rust: Helper Functions - Dan Munckton](https://dan.munckton.co.uk/blog/2018/02/25/testing-rust-helper-functions/) | Blog | Practical guide to organizing helpers |
| [Testing Rust: Builder Pattern - Dan Munckton](https://dan.munckton.co.uk/blog/2018/03/01/testing-rust-using-the-builder-pattern-for-complex-fixtures/) | Blog | Builder pattern for future reference (not needed now) |
| [Skeleton And Principles For A Maintainable Test Suite - Luca Palmieri](https://lpalmieri.com/posts/skeleton-and-principles-for-a-maintainable-test-suite/) | Blog | Test suite architecture principles |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | External research: Rust test organization patterns | Confirmed `tests/common/mod.rs` is canonical; identified 4 patterns; documented pitfalls |
| 2026-02-12 | Internal research: Complete helper catalog across 14 test files | Cataloged all helpers; verified PRD claims; found nuances in make_item variations |
| 2026-02-12 | PRD accuracy check | PRD 95% accurate; minor corrections to default_config line counts and setup_test_env file count |
| 2026-02-12 | Deep dive: git_test.rs setup vs setup_test_env | Decided to keep git_test local (different semantics) |
| 2026-02-12 | Deep dive: prompt_test make_item hardcoded status | Confirmed intentional — domain-specific helper, not a bug |
| 2026-02-12 | Synthesis and recommendations | Flat common/mod.rs, minimal defaults, phase_pool=Main for in_progress |
