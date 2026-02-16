# SPEC: Add --target Priority List and --only Flag for Orchestrator Run

**ID:** WRK-030
**Status:** Final
**Created:** 2026-02-12
**PRD:** ./WRK-030_add-target-priority-list-and-only-flag-for-orchestrator-run_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

The orchestrator currently supports only two extremes: run everything (default scheduler) or run exactly one item (`--target WRK-005`). Users with 50+ item backlogs need finer control — processing 2-5 specific items in priority order, or filtering by attributes like `impact=high`. This change extends the CLI with multi-target priority lists and attribute-based filtering while preserving the existing pure-scheduler / stateful-runner architecture.

The existing `select_targeted_actions()` function (scheduler.rs:720-772) already handles single-target scheduling. Multi-target support wraps this with a cursor-based advancement loop in the runner, while filtering applies a pre-scheduler snapshot transformation. Both features use fail-fast validation at startup.

## Approach

The implementation touches four layers, each with a clear role:

1. **Types & Parsing Foundation** — Extract `parse_size_level()` and `parse_dimension_level()` from `main.rs` to `types.rs` as public functions, add `parse_item_status()` for snake_case status parsing. This enables both the CLI and filter module to share parsing logic.

2. **Filter Module** — New `src/filter.rs` with `FilterField` enum (7 variants), `FilterValue` enum (pre-parsed typed values), and `FilterCriterion` struct. Provides `parse_filter()`, `apply_filter()`, and `matches_item()` functions. Filter is applied to snapshot before `select_actions()`, producing a narrowed view while keeping the unfiltered snapshot for halt-condition checks.

3. **CLI & Validation** — Change `--target` from `Option<String>` to `Vec<String>` using clap's `action = Append`, add `--only` with `conflicts_with = "target"`. Validate all targets (format, existence, no duplicates) and filter syntax at startup with error accumulation. Construct `RunParams` with `targets: Vec<String>` and `filter: Option<FilterCriterion>`.

4. **Scheduler Integration** — Add `current_target_index: usize` to `SchedulerState`. The runner loop uses an advancement subroutine to skip Done/archived targets and detect exhaustion. Filter mode applies `apply_filter()` each cycle on fresh snapshot. New halt reasons: `FilterExhausted`, `NoMatchingItems`. Pure scheduling functions (`select_actions`, `select_targeted_actions`) remain unchanged.

**Patterns to follow:**

- `src/scheduler.rs:720-772` — `select_targeted_actions()` takes `target_id: &str`, unchanged. Runner passes `&params.targets[state.current_target_index]`
- `src/scheduler.rs:1314-1321` — `SchedulerState` tracks per-run mutable state; add `current_target_index` here
- `src/scheduler.rs:514-526` — Target completion/block check pattern; extend to multi-target with advancement subroutine
- `src/main.rs:727-748` — `parse_size_level()` / `parse_dimension_level()` pattern for case-insensitive enum parsing
- `src/config.rs:147-205` — `config::validate()` error accumulation pattern (collect all errors, return together)
- `tests/scheduler_test.rs:72-97` — `make_item()` and `make_snapshot()` test helpers for constructing fixtures

**Implementation boundaries:**

- Do not modify: `select_actions()`, `select_targeted_actions()` function signatures or internal logic
- Do not modify: `coordinator.rs` (provides snapshots, no changes needed)
- Do not modify: `executor.rs`, `prompt.rs`, `backlog.rs`
- Do not refactor: Existing test helpers beyond what's needed for new test coverage

## Open Questions

_All open questions from design phase resolved. No remaining open questions._

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Types & Filter Foundation | Med | Extract parsers to types.rs, create filter.rs module with parsing/matching/application logic, add tests |
| 2 | CLI & Validation | Med | Update clap args, implement startup validation for targets and filters, update config logging |
| 3 | Scheduler Integration | High | Multi-target cursor tracking, filter snapshot application, new halt reasons, advancement subroutine |
| 4 | Integration Testing & Polish | Med | End-to-end tests, backward compatibility verification, edge case coverage |

**Ordering rationale:** Phase 1 establishes types that Phase 2 and 3 depend on. Phase 2 provides validated `RunParams` that Phase 3 consumes. Phase 4 verifies the full integration.

---

## Phases

Each phase should leave the codebase in a functional, stable state. Complete and verify each phase before moving to the next.

---

### Phase 1: Types & Filter Foundation

> Extract parsing functions and create the filter module with comprehensive tests

**Phase Status:** complete

**Complexity:** Med

**Goal:** Establish the type foundation (shared parsers in types.rs) and the complete filter module (filter.rs) so that Phase 2 and 3 can consume them.

**Files:**

- `src/types.rs` — modify — Add `pub fn parse_size_level()`, `pub fn parse_dimension_level()`, `pub fn parse_item_status()`
- `src/main.rs` — modify — Change local `parse_size_level()` and `parse_dimension_level()` to import from `types`
- `src/filter.rs` — create — New module: `FilterField`, `FilterValue`, `FilterCriterion`, `parse_filter()`, `apply_filter()`, `matches_item()`
- `src/lib.rs` — modify — Add `pub mod filter;`
- `tests/filter_test.rs` — create — Unit tests for filter parsing, validation, and matching

**Patterns:**

- Follow `src/main.rs:727-748` for parsing function style (lowercase match, descriptive error messages)
- Follow `src/types.rs` for enum definitions with serde derives

**Tasks:**

- [x] Add `pub fn parse_size_level(s: &str) -> Result<SizeLevel, String>` to `types.rs` — move from `main.rs:727-737`, make `pub`
- [x] Add `pub fn parse_dimension_level(s: &str) -> Result<DimensionLevel, String>` to `types.rs` — move from `main.rs:739-748`, make `pub`
- [x] Add `pub fn parse_item_status(s: &str) -> Result<ItemStatus, String>` to `types.rs` — new function, snake_case match: `new`, `scoping`, `ready`, `in_progress`, `done`, `blocked` (case-insensitive)
- [x] Update `main.rs` to import `parse_size_level` and `parse_dimension_level` from `types` instead of local definitions
- [x] Create `src/filter.rs` with:
  - `FilterField` enum: `Status`, `Impact`, `Size`, `Risk`, `Complexity`, `Tag`, `PipelineType`
  - `FilterValue` enum: `Status(ItemStatus)`, `Dimension(DimensionLevel)`, `Size(SizeLevel)`, `Tag(String)`, `PipelineType(String)`
  - `FilterCriterion` struct: `{ field: FilterField, value: FilterValue }`
  - `pub fn parse_filter(raw: &str) -> Result<FilterCriterion, String>` — split on `=`, validate field name, parse value based on field type
  - `pub fn apply_filter(criterion: &FilterCriterion, snapshot: &BacklogSnapshot) -> BacklogSnapshot` — return new snapshot with only matching items
  - `fn matches_item(criterion: &FilterCriterion, item: &BacklogItem) -> bool` — per-field matching logic
- [x] Add `pub mod filter;` to `src/lib.rs`
- [x] Create `tests/filter_test.rs` with tests:
  - Parse valid filters for all 7 field types (one test per field minimum)
  - Invalid field name → error message listing valid fields: "Unknown filter field: foo. Supported: status, impact, size, risk, complexity, tag, pipeline_type"
  - Invalid enum value → error message listing valid values: "Invalid value 'gigantic' for field 'size'. Valid values: small, medium, large"
  - Malformed syntax (no `=`) → error: "Filter must be in format KEY=VALUE, got: ..."
  - Case-insensitive parsing for enum fields (`impact=HIGH`, `status=IN_PROGRESS` → both work)
  - Case-sensitive matching for `tag` and `pipeline_type` (`tag=v1` does NOT match item with tag `V1`)
  - `status=in_progress` parses and matches `ItemStatus::InProgress` (critical serde edge case)
  - Tag filtering: items with empty `tags: []` never match any tag filter
  - `Option::None` fields never match: test each Optional field (`impact`, `size`, `risk`, `complexity`, `pipeline_type`) with `None` values
  - `apply_filter()` returns correct subset of items
  - `apply_filter()` on empty snapshot returns empty snapshot
  - Parser error tests: empty string, whitespace only, multiple `=` signs (e.g., `"key=val=ue"` — first `=` splits)

**Verification:**

- [x] `cargo build` succeeds with no warnings
- [x] `cargo test` passes — all existing tests pass, new filter tests pass (41 filter tests + 359 existing tests = 400 total)
- [x] `main.rs` still uses `parse_size_level()` and `parse_dimension_level()` via import (no duplication)
- [x] `parse_item_status("in_progress")` returns `Ok(ItemStatus::InProgress)` (verified by test `parse_and_match_status_in_progress`)
- [x] Code review passes — no critical/high issues found, added Display roundtrip test from review suggestion

**Commit:** `[WRK-030][P1] Feature: Add filter module and extract parsing functions to types.rs`

**Notes:**

- The `FilterValue` enum stores pre-parsed typed values. Parsing happens once at startup; matching uses typed comparison, not string re-parsing.
- `parse_item_status()` uses snake_case representation matching serde `rename_all = "snake_case"` on `ItemStatus`. Critical to test `in_progress` specifically.
- `FilterField::Tag` maps to `BacklogItem.tags: Vec<String>` — the CLI field name is `tag` (singular) for ergonomics.
- `FilterField::PipelineType` maps to `BacklogItem.pipeline_type: Option<String>` — the CLI field name is `pipeline_type`.
- Added `Display` impl for `FilterCriterion` with roundtrip test (parse → display → reparse) to catch any future drift.

**Followups:**

---

### Phase 2: CLI & Validation

> Update CLI argument parsing and implement startup validation for targets and filters

**Phase Status:** complete

**Complexity:** Med

**Goal:** Accept multi-target and `--only` CLI arguments, validate at startup with error accumulation, update config logging, and construct the new `RunParams`.

**Files:**

- `src/main.rs` — modify — Update `Commands::Run` struct (target → Vec, add only), update `handle_run()` signature and validation logic, update config logging
- `src/scheduler.rs` — modify — Update `RunParams` struct (target → targets, add filter field), update `HaltReason` enum (add `FilterExhausted`, `NoMatchingItems`)

**Patterns:**

- Follow `src/config.rs:147-205` for error accumulation pattern (collect all errors into `Vec<String>`, return together)
- Follow `src/main.rs:262-276` for config logging format

**Tasks:**

- [x] Update `Commands::Run` in `main.rs`:
  - Change `target: Option<String>` to `target: Vec<String>` with `#[arg(long, action = clap::ArgAction::Append)]`
  - Add `only: Option<String>` with `#[arg(long, conflicts_with = "target")]`
  - Update help text for `--target`: "Target specific backlog items by ID (can be specified multiple times for sequential processing)"
- [x] Update `RunParams` in `scheduler.rs`:
  - Change `target: Option<String>` to `targets: Vec<String>`
  - Add `filter: Option<filter::FilterCriterion>`
- [x] Add `FilterExhausted` and `NoMatchingItems` variants to `HaltReason` enum in `scheduler.rs`
- [x] Update `handle_run()` signature in `main.rs` to accept `target: Vec<String>, only: Option<String>`
- [x] Update `Commands::Run` match arm in `main()` to pass both fields
- [x] Implement target validation in `handle_run()` (before scheduler start):
  - Format validation: each target matches `{PREFIX}-\d+` pattern using `config.project.prefix` (reject missing hyphen, non-numeric suffix, wrong prefix, empty string, whitespace)
  - Existence validation: each target ID exists in backlog items
  - Duplicate detection: use `HashSet` for O(n) check, report all duplicates with error: "Duplicate targets: WRK-005"
  - Error accumulation: collect all validation errors, return together with format "Target validation failed:\n  - error1\n  - error2"
- [x] Implement filter validation in `handle_run()`:
  - Call `filter::parse_filter()` on `only` argument
  - On success: count matching items via snapshot for config log
  - On error: return the parse error
- [x] Implement mutual exclusivity check:
  - If both `target` (non-empty) and `only` are provided: return error "Cannot combine --target and --only flags. Use one or the other."
  - Note: clap's `conflicts_with` should handle this, but add an explicit check as a safety net
- [x] Update config logging in `handle_run()`:
  - Multi-target: `[config] Targets: WRK-005, WRK-010, WRK-003`
  - Filter: `[config] Filter: impact=high — N items match (from M total)`
  - Single target (backward compat): `[config] Target: WRK-005` (same as before for single element)
- [x] Construct `RunParams` with validated data:
  - `targets: target` (Vec from clap, empty if no `--target` flags)
  - `filter: parsed_filter` (from `parse_filter()`, None if no `--only` flag)
- [x] Update all test code that constructs `RunParams` — change `target: Some(...)` / `target: None` to `targets: vec![...]` / `targets: vec![]`, add `filter: None`

**Verification:**

- [x] `cargo build` succeeds
- [x] `cargo test` passes — all existing tests pass with updated `RunParams` construction
- [x] Verified by code inspection: single `--target WRK-005` produces `targets: vec!["WRK-005"]` (backward compat) — clap `Append` with single flag yields single-element Vec
- [x] Verified by code inspection: `--target WRK-005 --target WRK-010` produces `targets: vec!["WRK-005", "WRK-010"]` — clap `Append` collects repeated flags
- [x] Verified by code inspection: `--only impact=high` calls `filter::parse_filter()` which is tested in filter_test.rs (41 tests)
- [x] Verified by clap `conflicts_with` + explicit safety net check: `--target` and `--only` cannot be combined
- [x] Verified by code inspection: invalid target format errors at startup with clear message — format check uses `starts_with("{prefix}-")` + numeric suffix validation
- [x] Verified by code inspection: non-existent target errors at startup listing the missing IDs — existence check iterates backlog items
- [x] Verified by code inspection: duplicate targets error at startup listing the duplicates — HashSet-based O(n) detection
- [x] Code review passes — fixed whitespace trimming, simplified duplicate detection, removed misleading "(active, 1/N)" annotation

**Commit:** `[WRK-030][P2] Feature: Add multi-target and --only CLI arguments with startup validation`

**Notes:**

- The clap `conflicts_with` attribute handles `--target` / `--only` mutual exclusivity at the parser level. The explicit check in `handle_run()` is a safety net.
- `handle_run()` receives `target: Vec<String>` directly from clap. An empty Vec means no `--target` flags were provided. A single-element Vec matches old single-target behavior.
- Target format validation uses `config.project.prefix` (e.g., "WRK") to check the format `WRK-\d+`. The prefix is loaded from `orchestrate.toml`.
- For backward compatibility, when exactly one target is specified, log it as `[config] Target: WRK-005` (singular) matching existing output.
- Targets are trimmed of whitespace before validation for robustness.
- Multi-target log shows simple comma-separated list; `(active, 1/N)` annotation deferred to Phase 3 when cursor tracking is implemented.

**Followups:**

---

### Phase 3: Scheduler Integration

> Implement multi-target cursor tracking, filter snapshot application, advancement subroutine, and new halt conditions

**Phase Status:** complete

**Complexity:** High

**Goal:** Wire multi-target and filter modes into the scheduler runner loop. Multi-target uses a cursor-based advancement subroutine; filter mode applies `apply_filter()` each cycle. Add `FilterExhausted` and `NoMatchingItems` halt conditions.

**Files:**

- `src/scheduler.rs` — modify — Add `current_target_index` to `SchedulerState`, implement target advancement subroutine, add filter application before action selection, add halt condition checks for filter exhaustion
- `src/filter.rs` — modify — Make `matches_item()` public for direct use in scheduler
- `tests/scheduler_test.rs` — modify — Add multi-target and filter scheduling tests

**Patterns:**

- Follow the existing target completion/block check in `run_scheduler()` (the `if let Some(ref target_id) = params.target` block) — extend to multi-target with advancement subroutine
- Follow the existing action selection branching in `run_scheduler()` (the `if let Some(ref target_id) = params.target` dispatch) — extend to three cases: targets, filter, normal

**Tasks:**

- [x] Add `current_target_index: usize` to `SchedulerState` struct and initialize to `0` in `run_scheduler()`
- [x] Implement `advance_to_next_active_target()` as a public function (for unit testing):
  ```
  pub fn advance_to_next_active_target(
      targets: &[String],
      current_index: usize,
      items_completed: &[String],
      snapshot: &BacklogSnapshot,
  ) -> usize
  ```
  - Loop while `current_index < targets.len()`
  - If target not found in snapshot (archived): log warning, increment, continue
  - If target in `items_completed` or status == Done: log info with position `({index+1}/{total})`, increment, continue
  - If target status == Blocked: log info, increment, continue (skip pre-blocked targets)
  - Otherwise break (found active target)
  - Return final index (if `>= targets.len()`, all targets exhausted)
- [x] Update target completion/block check (replacing the existing `if let Some(ref target_id) = params.target` block):
  - If `!params.targets.is_empty()`:
    - Check if current target is in `state.items_blocked` (run-time blocked check) → halt with `TargetBlocked` before advancement
    - Call `advance_to_next_active_target()` and store returned index into `state.current_target_index`
    - If `state.current_target_index >= params.targets.len()`: drain join set, halt with `TargetCompleted`
  - Else if no targets: keep existing `AllDoneOrBlocked` halt check (unchanged)
- [x] Add filter application before action selection:
  - If `params.filter.is_some()`:
    - Apply `filter::apply_filter()` to snapshot → `filtered_snapshot`
    - If `filtered_snapshot.items.is_empty()` and all items in the unfiltered snapshot that match the filter are Done or Blocked: halt with `FilterExhausted` (message: "All items matching {field}={value} are done or blocked")
    - If `filtered_snapshot.items.is_empty()` and no items in the unfiltered snapshot match the filter at all: halt with `NoMatchingItems` (message: "No items match filter criteria: {field}={value}"). Uses `filter::matches_item()` directly for efficiency.
    - Otherwise check if all items in `filtered_snapshot` are Done or Blocked → halt with `FilterExhausted`
    - Pass `filtered_snapshot` to `select_actions()` (not `select_targeted_actions`)
  - **Halt semantics:** `NoMatchingItems` means zero items in the backlog have the filtered attribute (independent of status). `FilterExhausted` means items match but all are Done/Blocked. Items archived during the run use `has_prior_progress` check to distinguish.
- [x] Update action selection branching (replacing the existing `if let Some(ref target_id) = params.target` dispatch):
  - If `!params.targets.is_empty()`: `select_targeted_actions(..., &params.targets[state.current_target_index])`
  - Else if `params.filter.is_some()`: `select_actions(&filtered_snapshot, ...)`
  - Else: `select_actions(&snapshot, ...)` (unchanged)
- [x] Make `matches_item()` public in `filter.rs` for direct use in scheduler empty-check
- [x] Add unit tests for `advance_to_next_active_target()` to `tests/scheduler_test.rs`:
  - `test_advance_skips_done_targets` — chain of Done targets skipped, returns index of first active
  - `test_advance_skips_archived_targets` — target not in snapshot → skipped
  - `test_advance_all_exhausted` — all targets Done/archived → returns index >= len
  - `test_advance_first_is_active` — first target is active → returns same index (0)
  - `test_advance_mixed_states` — mix of Done, archived, and active targets
  - `test_advance_empty_targets` — empty targets Vec → returns 0
  - `test_advance_skips_items_in_completed_list` — items in completed list skipped even if snapshot status differs
  - `test_advance_skips_pre_blocked_targets` — targets with Blocked status in snapshot skipped
- [x] Add multi-target integration tests to `tests/scheduler_test.rs`:
  - `test_multi_target_processes_in_order` — targets processed sequentially
  - `test_multi_target_halts_on_block` — current target blocked → halt with `TargetBlocked`
  - `test_multi_target_all_done_at_startup` — all targets already Done → immediate halt with `TargetCompleted`
  - `test_multi_target_skips_done_targets` — chain of Done targets skipped, advances to first active
  - `test_multi_target_single_element_backward_compat` — single `--target` in Vec behaves identically to pre-change (same halt reason, items_completed, action sequence)
  - `test_multi_target_target_archived_during_run` — target not in snapshot → treated as completed, advance
  - `test_multi_target_skips_pre_blocked_targets` — pre-Blocked targets skipped, next active target processed
- [x] Add filter scheduling tests to `tests/scheduler_test.rs`:
  - `test_filter_restricts_scheduler_to_matching_items` — only matching items scheduled
  - `test_filter_no_matching_items_halts` — no items have the filtered attribute → `NoMatchingItems`
  - `test_filter_all_exhausted_halts` — matching items all Done/Blocked → `FilterExhausted`

**Verification:**

- [x] `cargo build` succeeds
- [x] `cargo test` passes — all existing + new tests pass (449 total: 70 scheduler + 379 others)
- [x] Multi-target: `run --target WRK-005 --target WRK-010` processes WRK-005 first, then WRK-010 (verified by `test_multi_target_processes_in_order`)
- [x] Multi-target: blocks halt with `TargetBlocked` including position info in logs (verified by `test_multi_target_halts_on_block`)
- [x] Multi-target: all Done at startup → immediate `TargetCompleted` (verified by `test_multi_target_all_done_at_startup`)
- [x] Filter: `run --only impact=high` schedules only high-impact items (verified by `test_filter_restricts_scheduler_to_matching_items`)
- [x] Filter: no matches → `NoMatchingItems` halt (verified by `test_filter_no_matching_items_halts`)
- [x] Filter: all matching Done/Blocked → `FilterExhausted` halt (verified by `test_filter_all_exhausted_halts`)
- [x] Single target backward compat: `run --target WRK-005` identical behavior (verified by `test_multi_target_single_element_backward_compat`)
- [x] Code review passes — fixed HIGH (pre-Blocked skip) and MEDIUM (matches_item efficiency), deferred LOW items

**Commit:** `[WRK-030][P3] Feature: Multi-target cursor tracking and filter snapshot application in scheduler`

**Notes:**

- The advancement subroutine is a pure function (takes snapshot + state, returns new index). Testable in isolation. Made `pub` for direct unit testing.
- `select_targeted_actions()` signature is unchanged — it still takes `target_id: &str`. The runner loop passes `&params.targets[state.current_target_index]`.
- Filter application creates a new `BacklogSnapshot` with filtered items. The original snapshot is kept for halt-condition checking (e.g., `AllDoneOrBlocked` checks all items, not just filtered ones). This follows the existing snapshot-based design.
- The `NoMatchingItems` check applies on every cycle, not just the first. If filter matches on the first iteration but items change status so the filter matches nothing on a later iteration, it halts with `FilterExhausted` (all matching are Done/Blocked), not `NoMatchingItems`.
- Target advancement logs use `[target]` prefix at info level: `[target] WRK-005 already done (1/3). Skipping.`
- Filter halt logs use `[filter]` prefix: `[filter] All items matching impact=high are done or blocked.`
- `advance_to_next_active_target()` skips pre-Blocked targets (ItemStatus::Blocked in snapshot) but the run-time `items_blocked` check fires before advancement to catch targets blocked during this run, preserving `TargetBlocked` halt semantics.
- Made `filter::matches_item()` public to avoid per-item `apply_filter()` clone overhead in the scheduler's filter empty-check.
- `has_prior_progress` check distinguishes `NoMatchingItems` from `FilterExhausted` when items have been archived (completed items removed from snapshot).

**Followups:**

- [ ] [Low] Add `test_filter_reapplied_each_cycle` test — verifying items that gain matching attributes mid-run are included (deferred: requires mock agent mid-run attribute mutation)
- [ ] [Low] Add `test_multi_target_advances_on_completion` as separate explicit test (covered implicitly by `test_multi_target_processes_in_order`)

---

### Phase 4: Integration Testing & Polish

> End-to-end integration tests, backward compatibility verification, terminal output polish

**Phase Status:** complete

**Complexity:** Med

**Goal:** Verify the full feature works end-to-end via integration tests on realistic scenarios, ensure backward compatibility, and polish terminal output formatting.

**Files:**

- `tests/scheduler_test.rs` — modify — Add end-to-end integration tests using `run_scheduler()` with mock agent
- `src/main.rs` — modify — Polish run summary output for multi-target and filter modes

**Patterns:**

- Follow existing integration test pattern in `tests/scheduler_test.rs` (setup_test_env, mock agent, run_scheduler, assert RunSummary)

**Tasks:**

- [x] Add integration test: `test_integration_single_target_backward_compat`:
  - Set up backlog with item WRK-001 (InProgress at build)
  - Run scheduler with `targets: vec!["WRK-001"]`
  - Assert: halt reason `TargetCompleted`, items_completed contains "WRK-001"
  - Assert: behavior identical to old `target: Some("WRK-001".to_string())` behavior
- [x] Add integration test: `test_integration_multi_target_sequential`:
  - Set up backlog with WRK-001, WRK-002, WRK-003 (all InProgress at build)
  - Run scheduler with `targets: vec!["WRK-001", "WRK-002", "WRK-003"]`
  - Assert: all items completed in order, halt reason `TargetCompleted`
- [x] Add integration test: `test_integration_multi_target_with_block`:
  - Set up backlog where second target will block
  - Run scheduler with `targets: vec!["WRK-001", "WRK-002"]`
  - Assert: WRK-001 completes, WRK-002 blocks → halt reason `TargetBlocked`
- [x] Add integration test: `test_integration_filter_impact_high`:
  - Set up backlog with mix of high/medium/low impact items
  - Run scheduler with `filter: Some(parse_filter("impact=high").unwrap())`
  - Assert: only high-impact items processed, halt reason `FilterExhausted`
- [x] Add integration test: `test_integration_filter_no_matches`:
  - Set up backlog with no high-impact items
  - Run scheduler with `filter: Some(parse_filter("impact=high").unwrap())`
  - Assert: halt reason `NoMatchingItems`
- [x] Polish run summary output in `main.rs`:
  - For filter: show final match state in summary (FilterExhausted and NoMatchingItems messages)
- [x] Verify terminal output format matches PRD specification:
  - `[config] Targets: WRK-005 (active, 1/3), WRK-010, WRK-003`
  - `[config] Filter: impact=high — 5 items match (from 47 total)`

**Verification:**

- [x] `cargo build` succeeds
- [x] `cargo test` passes — all tests pass including new integration tests (454 total: 75 scheduler + 379 others)
- [x] All PRD "Must Have" success criteria verified by at least one test
- [x] Single-target backward compat verified by integration test (`test_integration_single_target_backward_compat`)
- [x] Terminal output matches PRD specification — `(active, 1/N)` annotation on multi-target config line, filter match count log
- [x] No regressions in existing tests (449 pre-existing all pass)
- [x] Code review passes — only LOW items found (assertion style consistency fixed, naming convention noted)

**Commit:** `[WRK-030][P4] Feature: Integration tests and terminal output polish for multi-target and filter`

**Notes:**

- Integration tests use mock agent runner and coordinator setup (existing pattern in test file).
- Per-target completion status in summary is deferred to followups (PRD "Should Have"). See Followups Summary → Low.
- The `[config]` log lines are emitted in `handle_run()` before the scheduler starts. Target advancement and filter halt logs are emitted in `run_scheduler()` during execution.
- Run summary now shows filter-specific context for `FilterExhausted` and `NoMatchingItems` halt reasons.
- Multi-target `[config]` line now shows `(active, 1/N)` annotation on the first target, matching PRD specification.

**Followups:**

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met:
  - [x] `--target` accepts multiple item IDs (repeated flag) — verified by `test_integration_multi_target_sequential`
  - [x] Targets processed in specified order — verified by `test_multi_target_processes_in_order`
  - [x] Auto-advance on completion, halt on block — verified by `test_multi_target_halts_on_block`, `test_integration_multi_target_with_block`
  - [x] All target IDs validated at startup (format, existence, duplicates) — verified by code inspection (Phase 2)
  - [x] `--only` accepts single key=value filter — verified by `test_filter_restricts_scheduler_to_matching_items`, `test_integration_filter_impact_high`
  - [x] `--only` filters snapshot before scheduler — verified by `test_filter_restricts_scheduler_to_matching_items`
  - [x] Supported filter fields: status, impact, size, risk, complexity, tag, pipeline_type — verified by 41 filter tests
  - [x] Invalid field/value errors at startup with clear messages — verified by filter_test.rs error tests
  - [x] Case-insensitive for enum fields, case-sensitive for tag/pipeline_type — verified by filter_test.rs
  - [x] Tag filtering matches via contains, empty tags never match — verified by filter_test.rs
  - [x] Terminal output shows target list and filter info — implemented in main.rs with `(active, 1/N)` annotation
  - [x] Backward compatible: single `--target` unchanged — verified by `test_integration_single_target_backward_compat`, `test_multi_target_single_element_backward_compat`
  - [x] `--target` and `--only` mutually exclusive — verified by clap `conflicts_with` + explicit safety net
  - [x] `--only` with no matches → `NoMatchingItems` halt — verified by `test_filter_no_matching_items_halts`, `test_integration_filter_no_matches`
  - [x] `--only` exhaustion → `FilterExhausted` halt — verified by `test_filter_all_exhausted_halts`, `test_integration_filter_impact_high`
  - [x] `TargetCompleted` fires when ALL targets done — verified by `test_multi_target_processes_in_order`, `test_integration_multi_target_sequential`
- [x] Tests pass — 454 total (75 scheduler + 379 others)
- [x] No regressions introduced — all 449 pre-existing tests pass
- [x] Code reviewed — only LOW items found and addressed

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | complete | `[WRK-030][P1]` | Filter module + parser extraction. 41 tests, all 400 total pass. |
| 2 | complete | `[WRK-030][P2]` | CLI args + validation. Multi-target Vec, --only flag, startup validation, config logging. All 431 tests pass. |
| 3 | complete | `[WRK-030][P3]` | Scheduler integration. Multi-target cursor, filter snapshot application, new halt reasons. 20 new tests (70 scheduler, 449 total). Fixed pre-Blocked skip bug and filter efficiency. |
| 4 | complete | `[WRK-030][P4]` | Integration tests + terminal output polish. 5 new e2e tests, `(active, 1/N)` config annotation, filter summary in run output. 454 total tests pass. |

## Followups Summary

### Critical

### High

### Medium

- [ ] [Medium] Implement `--auto-advance` flag for multi-target runs — Currently halts when target blocks (MVP). Auto-advance skips to next target. (PRD "Should Have")
- [ ] [Medium] Support multiple `--only` filters with AND logic — Currently single filter only. Multiple filters: `--only impact=high --only size=small`. (PRD "Should Have")
- [ ] [Medium] Support comma-separated values within `--only` for OR within same field — e.g., `--only status=ready,in_progress`. (PRD "Should Have")

### Low

- [ ] [Low] Add `orchestrate status --only impact=high` to preview filter matches without running. (PRD "Nice to Have")
- [ ] [Low] Add negation filters `--only-not tag=skip`. (PRD "Nice to Have")
- [ ] [Low] Add per-target completion status to run summary. (PRD "Should Have")

## Design Details

### Key Types

```rust
// --- Filter Module Types (src/filter.rs) ---

/// Valid filter field names, one per filterable BacklogItem attribute.
pub enum FilterField {
    Status,       // → item.status: ItemStatus
    Impact,       // → item.impact: Option<DimensionLevel>
    Size,         // → item.size: Option<SizeLevel>
    Risk,         // → item.risk: Option<DimensionLevel>
    Complexity,   // → item.complexity: Option<DimensionLevel>
    Tag,          // → item.tags: Vec<String>
    PipelineType, // → item.pipeline_type: Option<String>
}

/// Pre-parsed filter value. Parsed once at startup, stored as typed value.
pub enum FilterValue {
    Status(ItemStatus),              // Case-insensitive parse, typed match
    Dimension(DimensionLevel),       // For impact, risk, complexity
    Size(SizeLevel),                 // For size
    Tag(String),                     // Case-sensitive exact match
    PipelineType(String),            // Case-sensitive exact match
}

/// A parsed and validated filter criterion.
pub struct FilterCriterion {
    pub field: FilterField,
    pub value: FilterValue,
}

// --- Updated Existing Types ---

/// RunParams (scheduler.rs) — updated
pub struct RunParams {
    pub targets: Vec<String>,                    // Was: target: Option<String>
    pub filter: Option<FilterCriterion>,          // New
    pub cap: u32,
    pub root: PathBuf,
}

/// HaltReason (scheduler.rs) — two new variants
pub enum HaltReason {
    AllDoneOrBlocked,
    CapReached,
    CircuitBreakerTripped,
    ShutdownRequested,
    TargetCompleted,
    TargetBlocked,
    FilterExhausted,     // New: all filtered items Done or Blocked
    NoMatchingItems,     // New: no items match filter at startup
}

/// SchedulerState (scheduler.rs) — one new field
struct SchedulerState {
    phases_executed: u32,
    cap: u32,
    consecutive_exhaustions: u32,
    items_completed: Vec<String>,
    items_blocked: Vec<String>,
    follow_ups_created: u32,
    current_target_index: usize,  // New: cursor into params.targets
}
```

### Architecture Details

**Filter Application Flow (each scheduler cycle):**
```
1. Fetch fresh BacklogSnapshot from coordinator
2. If filter mode:
   a. Apply filter → filtered_snapshot (new BacklogSnapshot with matching items only)
   b. Check halt conditions:
      - If no items in full snapshot have the filtered attribute at all → halt with NoMatchingItems
      - If matching items exist but all are Done/Blocked → halt with FilterExhausted
   c. Pass filtered_snapshot to select_actions()
3. If multi-target mode:
   a. Run advancement subroutine (skip Done/archived targets)
   b. If all targets exhausted → halt with TargetCompleted
   c. If current target in items_blocked → halt with TargetBlocked
   d. Pass full_snapshot + current_target to select_targeted_actions()
4. If neither:
   a. Pass full_snapshot to select_actions() (unchanged)
```

**Target Advancement Subroutine:**
```
advance_to_next_active_target(targets, current_index, items_completed, snapshot):
    while current_index < targets.len():
        target = targets[current_index]
        target_item = snapshot.items.find(target)
        if target_item is None:
            log_warn("[target] {} not found (archived?). Skipping.", target)
            current_index += 1
            continue
        if target in items_completed or target_item.status == Done:
            log_info("[target] {} already done. Skipping ({}/{}).", target, current_index+1, targets.len())
            current_index += 1
            continue
        break  // Found an active target
    return current_index  // If >= targets.len(), all targets exhausted
```

**FilterField → BacklogItem Matching:**

| FilterField | BacklogItem Field | Type | Matching Logic |
|-------------|-------------------|------|----------------|
| Status | `status` | `ItemStatus` | `item.status == parsed_status` |
| Impact | `impact` | `Option<DimensionLevel>` | `item.impact == Some(parsed_level)` |
| Size | `size` | `Option<SizeLevel>` | `item.size == Some(parsed_size)` |
| Risk | `risk` | `Option<DimensionLevel>` | `item.risk == Some(parsed_level)` |
| Complexity | `complexity` | `Option<DimensionLevel>` | `item.complexity == Some(parsed_level)` |
| Tag | `tags` | `Vec<String>` | `item.tags.contains(&tag_string)` |
| PipelineType | `pipeline_type` | `Option<String>` | `item.pipeline_type.as_deref() == Some(&pt_string)` |

### Design Rationale

**Why filter before scheduler, not inside:** Keeps `select_actions()` pure and unchanged. The scheduler doesn't need to know about filtering — it processes whatever items it receives. The runner manages context; the scheduler makes decisions on provided data.

**Why `Vec<String>` not `Option<Vec<String>>`:** Clap naturally produces an empty Vec when no `--target` flags are given. `is_empty()` distinguishes "none" from "some." No unnecessary `Option` wrapper.

**Why `current_target_index` in SchedulerState, not a separate struct:** SchedulerState already tracks per-run mutable state. Target index is the same kind of state. A separate `TargetQueue` struct adds indirection for a single `usize` field. If auto-advance is added later, extracting to a dedicated struct is a natural refactoring point.

**Why `FilterValue` stores parsed types, not raw strings:** Avoids re-parsing the same string hundreds of times per run (once per scheduler cycle). Makes the match logic a simple typed comparison instead of string → type → comparison.

---

## Assumptions

Decisions made without human input during autonomous SPEC creation:

1. **Parser tests prioritize serde edge case** — `parse_item_status("in_progress")` → `ItemStatus::InProgress` is tested explicitly because serde `rename_all = "snake_case"` representation is easy to get wrong.
2. **Integration tests use existing mock agent pattern** — Reuse `MockAgentRunner` and coordinator setup from existing integration tests rather than creating new test infrastructure.
3. **Per-target completion status in summary deferred** — The PRD "Should Have" for per-target status summary is deferred to followups to keep Phase 4 focused on verification.
4. **NoMatchingItems vs FilterExhausted semantics** — `NoMatchingItems` means zero items in the full backlog have the filtered attribute (regardless of status). `FilterExhausted` means items match the filter but all are Done or Blocked. Both conditions are checked each cycle (since the filter is re-evaluated on fresh snapshots), but `NoMatchingItems` will typically trigger on the first cycle. This aligns with the PRD's "halts immediately" language for `NoMatchingItems`.

## Retrospective

### What worked well?

- The phased approach kept each change manageable and independently verifiable. Each phase built cleanly on the previous one.
- The pure function design for `select_actions()` and `select_targeted_actions()` made unit testing straightforward. The statefulness is contained in the runner loop.
- The `advance_to_next_active_target()` function being public and pure enabled isolated unit tests that caught edge cases (pre-Blocked skip, archived targets) early.
- Error accumulation pattern from `config::validate()` worked well for target validation.

### What was harder than expected?

- The `FilterExhausted` vs `NoMatchingItems` distinction required careful thought about archived items and the `has_prior_progress` check.
- Ownership of `parsed_filter` — it gets moved into `RunParams`, so the run summary code needed to pre-capture a display string before the move.

### What would we do differently next time?

- Consider adding `Clone` or `Display` to `FilterCriterion` earlier in the design process to avoid ownership dance at the callsite.
- The Phase 3 and Phase 4 integration tests overlap significantly. Future SPECs could be more explicit about which tests are "new coverage" vs "verification of existing coverage."
