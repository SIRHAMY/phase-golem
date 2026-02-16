# Design: Add --target Priority List and --only Flag for Orchestrator Run

**ID:** WRK-030
**Status:** Refined
**Created:** 2026-02-12
**PRD:** ./WRK-030_add-target-priority-list-and-only-flag-for-orchestrator-run_PRD.md
**Tech Research:** ./WRK-030_add-target-priority-list-and-only-flag-for-orchestrator-run_TECH_RESEARCH.md
**Mode:** Medium

## Overview

Extend the orchestrator's `run` command with two mutually exclusive modes of subset control: multi-target priority lists (`--target WRK-005 --target WRK-010`) that process items sequentially in the specified order, and attribute-based filtering (`--only impact=high`) that restricts the normal scheduler to a matching subset. The design adds a new `filter` module for parsing/matching, extends `RunParams` from `Option<String>` to `Vec<String>` for targets, and manages target cursor state in the runner loop while keeping the pure scheduling functions unchanged.

---

## System Design

### High-Level Architecture

The feature touches four layers of the orchestrator, each with a clear role:

```
CLI Layer (main.rs)
  ├── Parse --target (Vec<String>) and --only (Option<String>)
  ├── Validate targets and filter at startup (fail-fast)
  └── Pass validated params to scheduler

Filtering Layer (filter.rs) — NEW
  ├── Parse "key=value" into FilterCriterion
  ├── Validate field names and values
  └── Apply filter to BacklogSnapshot → filtered BacklogSnapshot

Runner Layer (scheduler.rs::run_scheduler)
  ├── Track current_target_index in SchedulerState
  ├── Apply filter to snapshot before select_actions() (keep unfiltered for halt checks)
  ├── Advance target cursor on completion/skip (loop until non-Done target or exhausted)
  └── Check new halt conditions (FilterExhausted, NoMatchingItems)

Pure Scheduling Layer (scheduler.rs::select_actions / select_targeted_actions)
  └── UNCHANGED — receives pre-filtered snapshots, unaware of filter/multi-target
```

### Component Breakdown

#### CLI Argument Changes (main.rs)

**Purpose:** Accept multi-target and `--only` arguments, validate at startup, construct `RunParams`.

**Responsibilities:**
- Change `--target` from `Option<String>` to `Vec<String>` using clap's `action = Append`
- Add `--only` as `Option<String>` with `conflicts_with = "target"`
- Validate all target IDs: format (`{PREFIX}-\d+` where PREFIX comes from `config.project.prefix`), existence in backlog, no duplicates (use `HashSet` for O(n) duplicate detection)
- Accumulate all validation errors before returning (matching existing `config::validate()` pattern), so users see all issues at once
- Validate filter syntax: `key=value` format, valid field name, valid value for field type
- Log config summary:
  - Multi-target: `[config] Targets: WRK-005 (active, 1/3), WRK-010, WRK-003`
  - Filter: `[config] Filter: impact=high — 5 items match (from 47 total)`
- Construct `RunParams`: single `--target WRK-005` becomes `targets: vec!["WRK-005"]`; no `--target` becomes `targets: vec![]`

**Interfaces:**
- Input: CLI arguments from user
- Output: Validated `RunParams` with `targets: Vec<String>` and `filter: Option<FilterCriterion>`

**Dependencies:** `filter` module for filter parsing/validation, `backlog` for target existence check

#### Filter Module (filter.rs) — NEW

**Purpose:** Parse, validate, and apply `key=value` attribute-based filters to backlog snapshots.

**Responsibilities:**
- Parse a `"key=value"` string into a `FilterCriterion` (see type definition below)
- Validate field names against a `FilterField` enum (7 variants)
- Validate values for enum-valued fields using `parse_size_level()` and `parse_dimension_level()` (extracted from `main.rs` to `types.rs` as public functions, colocated with their enum definitions)
- Validate status values using a custom snake_case parser matching serde representation: `new`, `scoping`, `ready`, `in_progress`, `done`, `blocked`
- Apply a filter criterion to a `BacklogSnapshot`, producing a new snapshot with only matching items
- Report match count for terminal output
- Tag filtering returns false for items with empty `tags` Vec
- `Option<T>` fields (`impact`, `size`, `risk`, `complexity`, `pipeline_type`): `None` means no match

**Key Types:**

```rust
pub enum FilterField {
    Status,
    Impact,
    Size,
    Risk,
    Complexity,
    Tag,
    PipelineType,
}

/// Parsed and validated filter value. Parsing happens once at startup;
/// matching at runtime uses pre-parsed values to avoid re-parsing each cycle.
pub enum FilterValue {
    Status(ItemStatus),
    Dimension(DimensionLevel),    // For impact, risk, complexity
    Size(SizeLevel),
    Tag(String),                  // Case-sensitive
    PipelineType(String),         // Case-sensitive
}

pub struct FilterCriterion {
    pub field: FilterField,
    pub value: FilterValue,   // Parsed at startup, not raw string
}
```

**FilterField → BacklogItem Mapping:**

| FilterField | BacklogItem Field | Type | Matching Logic |
|-------------|-------------------|------|----------------|
| Status | `status` | `ItemStatus` | Compare pre-parsed `ItemStatus` with `==` |
| Impact | `impact` | `Option<DimensionLevel>` | Compare `Some(parsed) == item.impact`; `None` = no match |
| Size | `size` | `Option<SizeLevel>` | Compare `Some(parsed) == item.size`; `None` = no match |
| Risk | `risk` | `Option<DimensionLevel>` | Compare `Some(parsed) == item.risk`; `None` = no match |
| Complexity | `complexity` | `Option<DimensionLevel>` | Compare `Some(parsed) == item.complexity`; `None` = no match |
| Tag | `tags` | `Vec<String>` | `item.tags.contains(&value)` (case-sensitive); empty tags = no match |
| PipelineType | `pipeline_type` | `Option<String>` | `item.pipeline_type.as_deref() == Some(value)` (case-sensitive); `None` = no match |

**Case sensitivity rationale:** Enum fields (`status`, `impact`, `size`, `risk`, `complexity`) use case-insensitive parsing because they have a known, small set of valid values where case is an artifact of representation. Tag and `pipeline_type` are user-defined strings where case may carry meaning (e.g., tags `v1` vs `V1`).

**Interfaces:**
- `pub fn parse_filter(raw: &str) -> Result<FilterCriterion, String>` — parse and validate `"key=value"` string
- `pub fn apply_filter(criterion: &FilterCriterion, snapshot: &BacklogSnapshot) -> BacklogSnapshot` — return new snapshot with only matching items
- `pub fn match_count(criterion: &FilterCriterion, snapshot: &BacklogSnapshot) -> usize` — count matching items

**Dependencies:** `types` module for `BacklogItem`, `ItemStatus`, `SizeLevel`, `DimensionLevel`

#### SchedulerState Extension (scheduler.rs)

**Purpose:** Track target cursor position within the runner loop.

**Responsibilities:**
- Add `current_target_index: usize` field
- Provide method to get the current target ID from the targets list
- Advance to next target when current completes or is already Done

**Interfaces:**
- Input: `targets: Vec<String>` from `RunParams`
- Output: Current target ID for each loop iteration

**Dependencies:** `RunParams`

#### HaltReason Extension (scheduler.rs)

**Purpose:** Add new halt conditions for filter and multi-target modes.

**Responsibilities:**
- `FilterExhausted` — all items matching the filter are Done or Blocked
- `NoMatchingItems` — no items match the filter at startup

**Interfaces:**
- Part of `HaltReason` enum, returned in `RunSummary`

**Dependencies:** None

### Data Flow

1. **Startup:** CLI parses `--target` / `--only` args → validates targets against backlog or parses/validates filter → constructs `RunParams` with `targets: Vec<String>` and `filter: Option<FilterCriterion>`
2. **Runner loop entry:** `run_scheduler` initializes `SchedulerState` with `current_target_index: 0`
3. **Each loop iteration:**
   - Fetch snapshot from coordinator → `full_snapshot` (unchanged)
   - **If filter mode:** Apply filter to `full_snapshot` → `filtered_snapshot`. If `filtered_snapshot` is empty on first iteration → halt with `NoMatchingItems`. Check if all items in `filtered_snapshot` are Done/Blocked → halt with `FilterExhausted`. Pass `filtered_snapshot` to `select_actions()`.
   - **If multi-target mode:** Check if current target in `items_completed` or `items_blocked` → run advancement subroutine on `full_snapshot`. If all targets exhausted → halt with `TargetCompleted`. If current target blocked → halt with `TargetBlocked`. Pass `full_snapshot` + `&targets[current_target_index]` to `select_targeted_actions()`.
   - **If neither:** Pass `full_snapshot` to `select_actions()` (existing behavior).
4. **Phase completion:** Existing `handle_task_completion` adds to `items_completed`/`items_blocked` → next loop iteration detects target state change

### Key Flows

#### Flow: Multi-Target Run

> Process multiple targets sequentially in specified order.

1. **CLI Parsing** — User runs `orchestrate run --target WRK-005 --target WRK-010 --target WRK-003`. Clap collects into `Vec<String>`.
2. **Startup Validation** — Validate all IDs: format check (`{PREFIX}-{NNN}`), existence in backlog, duplicate detection. Log: `[config] Targets: WRK-005 (active, 1/3), WRK-010, WRK-003`.
3. **Skip Already-Done** — Before entering loop, run the advancement subroutine (see below). If all targets already Done, halt immediately with `TargetCompleted`.
4. **Normal Scheduling** — `select_targeted_actions()` is called with `targets[current_target_index]` as the target ID. The function signature is unchanged — it takes a single `target_id: &str`. The runner passes `&targets[current_target_index]`.
5. **Target Completes** — Current target reaches Done (detected via `items_completed` check at top of loop iteration, before fetching snapshot). Run the advancement subroutine.
6. **Target Blocks** — Current target blocks (detected via `items_blocked` check at top of loop iteration). Halt with `TargetBlocked`. Log: `[target] WRK-010 blocked (2/3). Halting.`
7. **All Complete** — If advancement subroutine exhausts the list, halt with `TargetCompleted`.

**Target Advancement Subroutine:**
```
fn advance_to_next_active_target(targets, current_index, items_completed, snapshot):
    while current_index < targets.len():
        target = targets[current_index]
        target_item = snapshot.items.find(target)
        if target_item is None:
            // Target was archived during run — treat as completed
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

**Edge cases:**
- Target blocks during in-flight phase — detected next loop iteration when `items_blocked` is checked
- All targets already Done at startup — advancement subroutine exhausts list, halt immediately with `TargetCompleted`
- Duplicate targets — rejected at startup validation
- Target doesn't exist in backlog — rejected at startup validation
- Target archived during run — advancement subroutine treats as completed, logs warning, advances to next

#### Flow: Filter Run

> Restrict scheduling to items matching an attribute filter.

1. **CLI Parsing** — User runs `orchestrate run --only impact=high`. Clap parses as `Option<String>`.
2. **Startup Validation** — Parse `"impact=high"` → `FilterCriterion { field: FilterField::Impact, value: "high" }`. Validate field name (7 allowed), validate value against `DimensionLevel` variants.
3. **Initial Match Count** — Apply filter to initial snapshot. If 0 matches → halt with `NoMatchingItems`, message: `No items match filter criteria: impact=high`. Otherwise log: `[config] Filter: impact=high — 5 items match (from 47 total)`.
4. **Each Loop Iteration** — Fetch fresh snapshot → apply filter → produce filtered snapshot. Pass filtered snapshot to `select_actions()`. Normal scheduling logic (advance-furthest-first) operates on the filtered subset.
5. **Exhaustion Check** — After filtering, check if all items in filtered snapshot are Done or Blocked. If so → halt with `FilterExhausted`.
6. **Normal Processing** — Actions returned by `select_actions()` operate on real items (IDs are valid in coordinator). Phase results flow through existing completion handling unchanged.

**Edge cases:**
- Items change status during run so they start/stop matching — filter is re-applied each cycle on fresh snapshot, so items that become `Done` naturally drop out; items that gain matching attributes are included
- Status filter for `in_progress` — uses serde representation (snake_case), parsed via lowercase match
- Tag filter — exact match against item's `tags` list; items with empty tags never match
- `pipeline_type` filter — exact, case-sensitive match against `Option<String>`; `None` means no match

---

## Technical Decisions

### Key Decisions

#### Decision: `Vec<String>` for targets instead of `Option<Vec<String>>`

**Context:** Need to represent "no targets", "one target", and "multiple targets."

**Decision:** Use `Vec<String>` directly. Empty vec means no targets specified.

**Rationale:** Clap naturally produces an empty `Vec` when no `--target` flags are given. `Option<Vec<String>>` adds an unnecessary `Option` layer since `is_empty()` distinguishes "none" from "some." This matches the clap maintainer guidance from tech research.

**Consequences:** All existing `if let Some(ref target_id) = params.target` patterns change to `if !params.targets.is_empty()` with index-based access. Backward compatible since a single `--target` produces a one-element Vec.

#### Decision: Target cursor in SchedulerState, not a separate struct

**Context:** Need to track which target is currently active across loop iterations.

**Decision:** Add `current_target_index: usize` to `SchedulerState`.

**Rationale:** `SchedulerState` already tracks per-run mutable state (`phases_executed`, `items_completed`, `items_blocked`). Target index is the same kind of state. A separate `TargetQueue` struct would add indirection for a single `usize` field.

**Consequences:** Target advancement logic lives in the runner loop alongside existing halt-condition checks. Simple to understand and test.

#### Decision: FilterField enum for field name validation

**Context:** Need to validate and dispatch on the filter field name in `--only key=value`.

**Decision:** Create a `FilterField` enum with 7 variants: `Status`, `Impact`, `Size`, `Risk`, `Complexity`, `Tag`, `PipelineType`.

**Rationale:** Compiler enforces exhaustive matching, preventing silent misses when new fields are added. Aligns with codebase convention of using enums for closed sets. String-based matching risks typos and silent failures.

**Consequences:** Adding a new filterable field requires adding an enum variant and a match arm. This is intentional — it forces a deliberate decision about how to filter on each field.

#### Decision: Filter applied to snapshot, not passed into scheduler

**Context:** The filter must restrict which items the scheduler considers. Two options: filter the snapshot before calling `select_actions()`, or pass the filter into `select_actions()`.

**Decision:** Filter the snapshot in the runner loop before calling `select_actions()`.

**Rationale:** Keeps `select_actions()` pure and unchanged. The scheduler doesn't need to know about filtering — it just processes whatever items it receives. This follows the existing pattern where the runner manages context and the scheduler makes decisions on provided data.

**Consequences:** A second snapshot reference (unfiltered) must be kept for halt-condition checks. Small additional allocation for the filtered snapshot each cycle, but O(n) clone is negligible per the NFR.

#### Decision: FilterCriterion stores parsed values, not raw strings

**Context:** Filter values need to be compared against `BacklogItem` fields each scheduler cycle. Two options: store the raw string and re-parse each cycle, or parse once at startup and store the typed value.

**Decision:** Parse values into a `FilterValue` enum at startup. Store pre-parsed typed values in `FilterCriterion`.

**Rationale:** Avoids re-parsing the same string hundreds of times per run. Eliminates the risk of parsing logic divergence between validation and matching. Makes the match logic a simple comparison rather than string → type → comparison.

**Consequences:** `FilterCriterion` is slightly more complex (typed enum vs String), but matching logic is simpler and faster.

#### Decision: Extract `parse_size_level()` and `parse_dimension_level()` to `types.rs`

**Context:** Filter module needs to parse `SizeLevel` and `DimensionLevel` values. These parsing functions currently live in `main.rs`.

**Decision:** Move `parse_size_level()` and `parse_dimension_level()` from `main.rs` to `types.rs` as public functions. Add a new `parse_item_status()` function to `types.rs` for parsing snake_case status values.

**Rationale:** Colocates parsing logic with type definitions. Both `main.rs` and `filter.rs` can import from `types`. Follows the principle of keeping type-related logic with the type.

**Consequences:** `main.rs` imports these functions from `types` instead of defining them locally. Minor refactoring.

#### Decision: Status filter values use serde (snake_case) representation

**Context:** `ItemStatus::InProgress` serializes to `in_progress` in YAML via `serde(rename_all = "snake_case")`. Users see `in_progress` in BACKLOG.yaml. The filter must accept what users see.

**Decision:** Parse status filter values using snake_case representation: `new`, `scoping`, `ready`, `in_progress`, `done`, `blocked`.

**Rationale:** Users write what they see in their YAML files. Matching `InProgress` would be confusing. Case-insensitive matching means `In_Progress` also works.

**Consequences:** Need a custom parser that matches snake_case status names, not the Rust variant names. Simple match statement on lowercased input.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Halt on block for multi-target | Users with long target lists must restart if first item blocks | Backward-compatible behavior; simpler state machine | MVP. Auto-advance is a clear Should Have enhancement behind `--auto-advance` flag |
| Single filter only | Can't combine `--only impact=high --only size=small` yet | Simpler parsing, no AND/OR logic to design | MVP. Multiple filters with AND logic is a Should Have. Single filter covers the most common case |
| Mutually exclusive --target and --only | Can't say "run these targets, but only if they're high impact" | No complex interaction semantics between two modes | Combined mode would require defining what happens when a target doesn't match the filter. Defer to Phase 2 |
| Filter re-clone each cycle | Allocates a new `Vec<BacklogItem>` each loop iteration | Consistent snapshot-based design; no mutation of coordinator state | O(n) clone on 500 items is microseconds. Snapshot is already cloned for coordinator communication |
| Closed FilterField enum | Adding new filterable fields requires code change | Compiler catches missing match arms; no silent failures | Only 7 fields. Changes are infrequent. Compiler enforcement is worth the maintenance cost |

---

## Alternatives Considered

### Alternative: Target Queue as Separate Struct

**Summary:** Create a dedicated `TargetQueue` struct that encapsulates the target list, current index, and advancement logic.

**How it would work:**
- `TargetQueue { targets: Vec<String>, current_index: usize }` with methods like `current()`, `advance()`, `is_exhausted()`
- Runner loop delegates target management to this struct
- SchedulerState holds a `TargetQueue` instead of raw index

**Pros:**
- Cleaner encapsulation of target iteration logic
- Methods make intent explicit (`.advance()` vs `index += 1`)

**Cons:**
- More indirection for a single `usize` field
- TargetQueue would need access to `items_completed`/`items_blocked` for advancement decisions, creating coupling
- Over-engineering for MVP complexity level

**Why not chosen:** The target iteration logic is ~10 lines in the runner loop. A dedicated struct adds more code than it saves. If auto-advance or more complex target policies are added later, this could be refactored then.

### Alternative: Pass Filter into select_actions()

**Summary:** Extend `select_actions()` to accept an optional filter parameter and apply filtering internally.

**How it would work:**
- Add `filter: Option<&FilterCriterion>` parameter to `select_actions()`
- Filter items inside the function before sorting/selecting
- Same for `select_targeted_actions()`

**Pros:**
- Filter logic is in one place (the scheduler)
- No need to create a separate filtered snapshot

**Cons:**
- Breaks the pure function contract — filter is a concern of the runner, not the scheduler
- Would need to update `select_targeted_actions()` too, adding parameter to an already-complex signature
- Harder to test filter and scheduling independently

**Why not chosen:** Violates the established architecture principle of keeping scheduling pure and stateless. The runner manages context; the scheduler makes decisions on provided data.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Target cursor logic introduces off-by-one errors | Skipped targets or infinite loops | Low | Unit tests for: all-done at startup, first-blocks, last-blocks, skip-chain of consecutive Done targets, single-target list, target archived during run. Advancement subroutine uses `while index < len` with explicit bounds check. |
| Serde representation mismatch for status filter | `status=in_progress` silently matches nothing | Medium | `parse_item_status()` in types.rs uses explicit snake_case match. Dedicated test: verify `in_progress` matches `ItemStatus::InProgress`. Add compile-time test comparing serde output for each status variant against parser expected values. |
| Breaking existing single-target behavior | Users relying on `--target WRK-005` see different behavior | Low | Single-element Vec behaves identically to current Option. Integration test: single `--target` run produces identical `HaltReason`, `items_completed`, and action sequence. |
| Filter criterion validation too strict/lenient | Users can't express valid filters, or invalid filters pass | Low | Error messages include valid values: `"Invalid value 'gigantic' for field 'size'. Valid values: small, medium, large"`. Error accumulation reports all issues at once. |

---

## Integration Points

### Existing Code Touchpoints

- `src/main.rs:38-53` — `Commands::Run` struct: change `target: Option<String>` to `target: Vec<String>` with `action = Append`, add `only: Option<String>` with `conflicts_with = "target"`.
- `src/main.rs:109` — `handle_run()` call site: update to pass `target` Vec and `only` Option.
- `src/main.rs:240-397` — `handle_run()`: add target validation (format, existence, duplicates with HashSet, error accumulation), filter validation via `filter::parse_filter()`, update config log output with formats from PRD, update `RunParams` construction (`target.map(|t| vec![t]).unwrap_or_default()` for backward compat, or pass Vec directly from clap).
- `src/main.rs:727-748` — Extract `parse_size_level()` and `parse_dimension_level()` to `src/types.rs` as public functions. `main.rs` imports from `types`.
- `src/types.rs` — Add `pub fn parse_size_level()`, `pub fn parse_dimension_level()`, `pub fn parse_item_status()` (snake_case match: `new`, `scoping`, `ready`, `in_progress`, `done`, `blocked`).
- `src/scheduler.rs:46-50` — `RunParams`: change `target: Option<String>` to `targets: Vec<String>`, add `filter: Option<FilterCriterion>`.
- `src/scheduler.rs:462-476` — `run_scheduler()`: initialize `current_target_index: 0` in `SchedulerState`.
- `src/scheduler.rs:514-526` — Target completion check: extend from single target to multi-target with advancement subroutine. Keep unfiltered snapshot for halt condition checks.
- `src/scheduler.rs:528-533` — Action selection: change from `if let Some(ref target_id) = params.target` to `if !params.targets.is_empty()`, pass `&params.targets[state.current_target_index]` to `select_targeted_actions()`.
- `src/scheduler.rs:720-772` — `select_targeted_actions()`: **signature unchanged** — still takes `target_id: &str`. Runner passes current target from vec.
- `src/scheduler.rs:36-43` — `HaltReason`: add `FilterExhausted`, `NoMatchingItems` variants.
- `src/scheduler.rs:1314-1321` — `SchedulerState`: add `current_target_index: usize`.
- `src/filter.rs` — New module with `FilterField`, `FilterValue`, `FilterCriterion`, `parse_filter()`, `apply_filter()`, `matches_item()`.
- `src/lib.rs` — Add `pub mod filter;`.
- `tests/scheduler_test.rs` — Add tests for multi-target scheduling, filter application, new halt reasons. Critical test: `status=in_progress` filter matches `ItemStatus::InProgress` (snake_case edge case). Integration test: single `--target WRK-005` produces identical behavior to pre-change.

### External Dependencies

None. All required infrastructure exists in the codebase.

---

## Open Questions

- [x] Should `current_target_index` live in `SchedulerState` or a new struct? → **Decision: SchedulerState** (simpler, avoids indirection)
- [x] How to parse `ItemStatus` filter values given `serde(rename_all = "snake_case")`? → **Decision: Custom match on snake_case lowercase** (matches what users see in YAML)
- [x] Should filter produce a new `BacklogSnapshot` or modify items in-place? → **Decision: New filtered snapshot** (keeps original for halt condition checking)
- [x] Should the runner log each target advance at `info` level or `debug`? → **Decision: `info` level** (target transitions are user-visible progress that users need to monitor, especially for long target lists)

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain (or they're flagged for spec phase)

---

## Assumptions

Decisions made without human input during autonomous design:

1. **Keep TargetQueue as future refactoring candidate** — 4/7 critique agents flagged this. For MVP, the ~15 lines of advancement logic in the runner loop are manageable. If auto-advance is added later, extracting to a dedicated struct is a natural refactoring point.
2. **Filter re-evaluated each cycle** — Some agents suggested one-shot filtering. Re-evaluation each cycle is simpler, matches snapshot-based design, and handles the desirable case where items gain matching attributes during the run (e.g., after triage sets `impact`).
3. **Keep mutual exclusivity of --target and --only** — Semantics of combining them are non-obvious for MVP. Phase 2 concern per PRD.
4. **All 7 filter fields included** — One agent suggested starting with 2-3. The incremental cost per field is one match arm; users shouldn't wait for fields visible in BACKLOG.yaml.
5. **Parser extraction to types.rs** — `parse_size_level()`, `parse_dimension_level()`, and new `parse_item_status()` colocated with their enum definitions in `types.rs`.
6. **FilterCriterion stores pre-parsed values** — Parse once at startup via `FilterValue` enum, not raw strings re-parsed each cycle.
7. **Target advancement logs at info level** — Users monitoring multi-target runs need to see target transitions.

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Initial design draft | Full design covering multi-target, filter, flows, decisions, and alternatives |
| 2026-02-12 | Self-critique (7 agents) and refinement | Added advancement subroutine pseudocode, parsed FilterValue type, error accumulation, parser extraction decision, terminal output formats, target-archived edge case, closed open questions |
