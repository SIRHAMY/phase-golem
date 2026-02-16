# Change: Add --target Priority List and --only Flag for Orchestrator Run

**Status:** Proposed
**Created:** 2026-02-12
**Author:** AI (autonomous PRD creation)

## Problem Statement

The orchestrator currently offers two extremes for controlling which items get processed: run everything (default scheduler picks items using advance-furthest-first priority, which processes items closest to completion before others), or run exactly one item (`--target WRK-005`). There is no middle ground for users who know which 2-5 items are most important and want the orchestrator to focus on those in a specific order, or for users who want to process a subset of their backlog filtered by attributes like impact, size, or tags.

This forces users into inefficient workarounds that require manual intervention:
- **Sequential single-target runs:** Run `orchestrate run --target WRK-005`, wait for completion, then run `--target WRK-010`, etc. Requires human-in-the-loop between each item, defeating autonomous orchestration.
- **Metadata manipulation:** Manually edit BACKLOG.yaml to set `impact: high` on desired items. Indirect, pollutes metadata with temporary priorities, and only affects the sort order when promoting items from Ready to InProgress status.
- **Accepting default scheduling:** Let the scheduler pick items, even when the user knows which items matter most right now due to user context (deadlines, blocking dependencies, strategic focus) that the automated scheduler cannot infer from backlog state alone.

With a backlog of 50+ items, the lack of granularity between "all" and "one" makes it impractical to efficiently prioritize work.

## User Stories / Personas

- **Solo developer (primary)** - Has a growing backlog and frequently knows which items are urgent based on context the scheduler can't see (external deadlines, blocking issues, current focus area). Wants to say "process these 3 items in this order, then stop" without babysitting each one.

- **Batch processor** - Wants to clear out a category of work (all small bug fixes, all high-impact items, all items tagged "sprint-1") without manually listing every ID. Needs attribute-based filtering.

## Desired Outcome

Users can control orchestrator execution at a granularity between "everything" and "one item":

1. **Multi-target with priority order:** `orchestrate run --target WRK-005 --target WRK-010 --target WRK-003` processes these items sequentially in the specified order, moving to the next target when the current one completes or blocks.

2. **Attribute-based filtering:** `orchestrate run --only impact=high` restricts the scheduler to only consider items matching the filter criteria, using normal scheduling logic (advance-furthest-first) within that subset.

A person watching the terminal sees clear output indicating which targets are queued, which is active, and progress through the list. When all targets complete (or block), the orchestrator halts with an appropriate halt reason (the enum code the orchestrator outputs when it stops execution, e.g., `TargetCompleted`, `TargetBlocked`).

## Success Criteria

### Must Have

- [ ] `--target` accepts multiple item IDs (repeated flag: `--target WRK-005 --target WRK-010`)
- [ ] Targets are processed in the order specified (first target gets exclusive focus until Done or Blocked)
- [ ] When current target completes, orchestrator automatically advances to next target
- [ ] When current target blocks, orchestrator halts with `TargetBlocked` reason (MVP behavior; auto-advance to next target planned as Should Have enhancement)
- [ ] All target IDs validated at startup: must match format `{PREFIX}-{NNN}` (alphanumeric + hyphen only), must exist in backlog; error with list of invalid IDs if validation fails
- [ ] Duplicate target IDs are rejected at startup with an error listing the duplicates
- [ ] If a target is already Done at startup, it is skipped with a log message; orchestrator advances to next target
- [ ] `--only` accepts a single key=value filter criterion (e.g., `--only impact=high`)
- [ ] `--only` filters the backlog snapshot before the scheduler selects actions (filter applied once per scheduler cycle on the snapshot, consistent with existing snapshot-based design)
- [ ] Supported filter fields: `status`, `impact`, `size`, `risk`, `complexity`, `tag`, `pipeline_type`
- [ ] Invalid filter field names error at startup with message listing supported fields (e.g., "Unknown filter field: foo. Supported: status, impact, size, risk, complexity, tag, pipeline_type")
- [ ] Malformed filter syntax errors at startup with clear message (e.g., "Filter must be in format KEY=VALUE, got: ...")
- [ ] Invalid filter values for enum fields error at startup with message listing valid values (e.g., "Invalid value 'gigantic' for field 'size'. Valid values: small, medium, large")
- [ ] Filter matching is case-insensitive for enum-valued fields: `status`, `impact`, `size`, `risk`, `complexity` (e.g., `impact=HIGH` matches stored value `High`). Tag and `pipeline_type` matching is case-sensitive.
- [ ] Tag filtering matches if the item's `tags` list contains the specified tag (items with empty tags list never match tag filters)
- [ ] Terminal output includes a config line showing target list and current active target with position (e.g., `[config] Targets: WRK-005 (active, 1/3), WRK-010, WRK-003`)
- [ ] Terminal output shows filter criteria and number of matching items (e.g., `[config] Filter: impact=high — 5 items match (from 47 total)`)
- [ ] Backward compatible: existing single `--target WRK-005` continues to work identically
- [ ] `--target` and `--only` are mutually exclusive; error if both provided: "Cannot combine --target and --only flags. Use one or the other."
- [ ] `--only` with no matching items halts immediately with halt reason `NoMatchingItems` and message: "No items match filter criteria: impact=high"
- [ ] When `--only` is used, orchestrator halts with `FilterExhausted` when all matching items reach Done or Blocked status
- [ ] Halt reason `TargetCompleted` fires when ALL targets in the list are Done (or skipped because already Done)

### Should Have

- [ ] When current target blocks, orchestrator auto-advances to next target instead of halting (controlled by `--auto-advance` flag)
- [ ] Multiple `--only` flags combine with AND logic: `--only impact=high --only size=small`
- [ ] Comma-separated values within `--only` for OR within same field: `--only status=ready,in_progress`
- [ ] Run summary shows per-target completion status (e.g., "WRK-005: Done, WRK-010: Blocked, WRK-003: Queued")

### Nice to Have

- [ ] `orchestrate status --only impact=high` to preview which items match a filter without running
- [ ] Negation filters: `--only-not tag=skip`

## Scope

### In Scope

- CLI argument parsing for multi-target and `--only` filter using existing `clap` framework
- Extending `select_targeted_actions()` (the scheduler function that selects items when a target is specified) to accept and iterate through an ordered list of targets
- Tracking current target index in the orchestrator runner (not in the pure scheduler function — statefulness managed by the runner, scheduler remains pure)
- Pre-scheduler snapshot filtering for `--only`
- New `filter.rs` module with filter parsing, validation, and matching logic
- Validation of target IDs and filter syntax at startup (fail-fast)
- Terminal output showing target progress and filter status
- New halt reasons: `FilterExhausted`, `NoMatchingItems` added to existing `HaltReason` enum
- Unit tests for new scheduling logic and filter parsing

### Out of Scope

- Complex query language or boolean expressions for filters (no `impact=high AND (size=small OR risk=low)`)
- Interactive target selection (TUI/prompts)
- Filter presets/profiles saved to config
- Per-target configuration overrides (e.g., different timeouts per target)
- Target dependency inference ("auto-include WRK-003 because WRK-005 depends on it")
- Dynamic priority recalculation or weighted scoring
- Regex or pattern matching for filter values
- Filter persistence across runs (filters are ephemeral CLI args)
- Combining `--target` and `--only` in the same command (potential Phase 2 work)
- Resume/checkpoint for partially completed target lists

## Non-Functional Requirements

- **Performance:** Filter application and target validation complete in <10ms for a 500-item backlog (single pass over items, O(n) where n = number of backlog items)
- **Observability:** Target list progress and filter match counts logged at info level

## Constraints

- Must use existing `clap` argument parsing; multi-target uses clap's repeated flag via `action = append` (e.g., `#[arg(long, action = clap::ArgAction::Append)]`)
- Must preserve existing single-target `--target WRK-005` behavior exactly
- Must work with the pure `select_actions()` / `select_targeted_actions()` function design (no side effects in scheduling logic); target iteration state is managed by the runner, not the scheduler
- Filter operates on snapshot, not live state (consistent with existing scheduler design; filter is re-evaluated each scheduler cycle on the fresh snapshot)
- Tags field (`Vec<String>`) already exists on `BacklogItem` but is currently unused in scheduling; tags are user-populated in BACKLOG.yaml

## Dependencies

- **Depends On:** None — all required infrastructure (clap CLI, scheduler, backlog types with tags) already exists
- **Blocks:** None directly, but enables more sophisticated orchestrator control patterns

## Risks

- [ ] **Blocked-target semantics:** Current single-target halts on block. Multi-target MVP preserves this (halts when current target blocks). This may frustrate users with long target lists where the first item blocks. Mitigated by implementing auto-advance as a Should Have (`--auto-advance` flag).
- [ ] **Filter field evolution:** If new fields are added to BacklogItem in the future, the filter module needs to be updated. Mitigated by keeping filter parsing centralized in a single `filter.rs` module. The MVP supports exactly these 7 filter fields; adding new fields requires a code change.

## Open Questions

- [ ] Should blocked targets auto-advance to next target by default for multi-target runs, or require an explicit `--auto-advance` flag? (Leaning: require explicit flag for MVP to preserve backward-compatible halt-on-block behavior. Auto-advance as default can be reconsidered after user feedback.)
- [ ] Should `--only` without `--target` halt when all filtered items are Done/Blocked? (Leaning: yes, halt with `FilterExhausted` — this is the natural "only these items" semantic.)
- [ ] For tag filtering, should the CLI field name be `tag` (singular, more natural CLI usage) or `tags` (matching the `BacklogItem` field name)? (Leaning: `tag` for CLI ergonomics, mapped internally to the `tags` field.)

## Assumptions

Decisions made without human input during autonomous PRD creation:

1. **Medium mode selected** — Based on the item assessment (WRK-030 categorized as medium size, medium complexity, low risk), a moderate-scope exploration phase was used.
2. **Mutually exclusive `--target` and `--only`** — For MVP simplicity, these two flags cannot be combined. This avoids complex interaction semantics. Combined mode is a candidate for Phase 2.
3. **Sequential target processing** — Multi-target processes items one at a time (first target gets exclusive scheduler focus), not concurrently. This aligns with existing single-target behavior and is simpler to reason about.
4. **Halt on block for MVP** — Current single-target behavior halts when target blocks. Multi-target MVP preserves this (halts when current target blocks). Auto-advance is a Should Have enhancement behind a flag.
5. **Single `--only` filter for MVP** — Start with one key=value pair. Multiple filters with AND logic is a Should Have.
6. **Error on invalid target/filter** — Fail fast at startup rather than warn-and-skip. A typo in a target ID or filter field should not silently proceed.
7. **`--only` halts when filtered set exhausted** — When all items matching the filter are Done or Blocked, the orchestrator halts with `FilterExhausted`. This is the natural "only" semantic.
8. **Orchestrator is single-process** — Only one orchestrator run executes at a time. No concurrent modification of the backlog by external processes during a run. This matches existing design.

## References

- Existing scheduler: `.claude/skills/changes/orchestrator/src/scheduler.rs` — contains `select_actions()` (normal scheduling) and `select_targeted_actions()` (single-target mode), `HaltReason` enum
- CLI entry point: `.claude/skills/changes/orchestrator/src/main.rs` — `clap` argument definitions and `handle_run()` function
- Backlog types: `.claude/skills/changes/orchestrator/src/types.rs` — `BacklogItem` struct, `ItemStatus` enum, `SizeLevel`, `DimensionLevel` enums
- Orchestrator config: `orchestrate.toml` — execution settings (max_wip, max_concurrent, phase_timeout); no changes required for this feature
- Similar patterns: cargo `-p`/`--package` for multi-target selection, pytest `-k` for filtering
