# WRK-030: Add --target priority list and --only flag for orchestrator run

## Problem Statement

The orchestrator's `run` command currently supports `--target <ID>` for a single item (exits when that item is done/blocked) or unguided run (scheduler picks everything via advance-furthest-first). There's no way to say "prioritize these specific items first, then continue normally" or "only work on these items and stop."

Users frequently know what's most important and want to direct the orchestrator without micromanaging every phase.

## Proposed Approach

### 1. Extend `--target` to accept a comma-separated priority list

- Change CLI parsing: `target: Option<String>` stays as `Option<String>` but accepts comma-separated IDs (e.g., `--target WRK-029,WRK-026`)
- Parse into `Vec<String>` in `RunParams`
- Scheduler processes targets in order: schedule the first non-completed/non-blocked target, then fall through to normal scheduling for remaining capacity
- When all targets are completed/blocked, fall back to normal advance-furthest-first scheduling for the rest of the backlog

### 2. Add `--only` flag

- New CLI flag: `#[arg(long)] only: bool` (default false)
- Only meaningful when combined with `--target`
- When `--only` is set, the orchestrator exits after all targeted items are Done or Blocked instead of continuing with the rest of the backlog
- New halt reason variant: `TargetsCompleted` (distinct from existing `TargetCompleted` for single target)

### Files affected

1. **`orchestrator/src/main.rs`** (~15-20 lines) - CLI arg parsing, RunParams construction, summary display
2. **`orchestrator/src/scheduler.rs`** (~50-80 lines) - RunParams struct, action selection logic, halt condition checks, priority ordering
3. **`orchestrator/src/types.rs`** (~5 lines) - Possibly new HaltReason variants
4. **Tests in scheduler.rs** (~50-100 lines) - New test cases for multi-target priority and --only halt behavior

### Design decisions to consider

- **Target blocked behavior**: When one target in the list blocks, should the orchestrator skip to the next target or stop? Proposed: skip to next (log warning), only halt if `--only` and all targets are done/blocked.
- **Interaction with max_wip/max_concurrent**: Should targeted items bypass WIP limits? Currently single `--target` bypasses them. Multi-target should respect limits but prioritize targeted items within those limits.
- **Dependency ordering within targets**: If user specifies `--target WRK-003,WRK-001` but WRK-003 depends on WRK-001, the scheduler should handle this gracefully (existing `skip_for_unmet_deps` already handles this — WRK-003 would be skipped until WRK-001 completes).
- **Validation**: Warn (not error) if a target ID doesn't exist in the backlog, in case it's a typo.

## Assessment

| Dimension  | Rating | Rationale |
|------------|--------|-----------|
| Size       | Medium | 4-5 files, ~120-200 lines of changes including tests |
| Complexity | Medium | Design decisions around multi-target priority ordering, halt semantics, and interaction with existing scheduler constraints |
| Risk       | Low    | Default behavior (no --target) is completely unchanged; additive feature |
| Impact     | High   | Directly addresses a frequently desired workflow — users want to direct priorities without micromanaging |

## Assumptions

- The comma-separated format for `--target` is preferable to repeated `--target` flags for ergonomics
- `--only` without `--target` should be an error (or no-op with warning), not silently accepted
- Existing `select_targeted_actions` function can be extended rather than requiring a wholly new scheduling strategy
- No need to persist target priority across restarts — it's a per-run CLI option
