# Change: Inline trivial `run_workflows_sequentially` passthrough function

**Status:** Proposed
**Created:** 2026-02-19
**Author:** phase-golem (automated)

## Problem Statement

`run_workflows_sequentially` in `src/executor.rs` (lines 469-480) is a trivial passthrough function that delegates directly to `runner.run_agent(prompt, result_path, timeout).await` without adding any logic, transformation, or error handling. It has a single call site at line 395 (verified via grep — no other references exist in `src/` or `tests/`), and its existence adds an unnecessary layer of indirection that obscures the actual call being made.

This was identified as tech debt during the WRK-003 orchestrator pipeline engine v2 implementation (SPEC notes: "[P5] `run_skills_sequentially` is a trivial passthrough — inline or defer until multi-skill orchestration").

The function's comment suggests it exists as a placeholder for future multi-workflow orchestration, but that orchestration isn't needed yet — the prompt already encodes all workflows as a single agent invocation.

## User Stories / Personas

- **Codebase maintainer** — When reading `execute_phase`, wants to see the actual agent invocation directly rather than following a function indirection that adds no value.

## Desired Outcome

The `run_workflows_sequentially` function is removed and its single-line body is inlined at the call site in `execute_phase`. The code reads more directly, and there is no behavioral change.

## Success Criteria

### Must Have

- [ ] `run_workflows_sequentially` function is removed from `src/executor.rs`
- [ ] The `runner.run_agent(...)` call is inlined at the single call site in `execute_phase`
- [ ] Code compiles without errors or warnings (`cargo check`)
- [ ] All existing tests pass without modification (`cargo test`)
- [ ] No behavioral change — verified by test passage (no tests reference `run_workflows_sequentially` directly)

### Should Have

- [ ] The inlined call retains the existing comment from the function body (lines 476-478: "Currently workflows are encoded in the prompt...") placed above the `runner.run_agent()` call

## Scope

### In Scope

- Removing `run_workflows_sequentially` from `src/executor.rs`
- Inlining its body at the call site in `execute_phase`

### Out of Scope

- Implementing actual multi-workflow orchestration (future work if/when needed)
- Refactoring other passthrough functions or tech debt items from WRK-003
- Changes to `AgentRunner` trait or `run_agent` signature

## Constraints

- Must be a pure refactor with zero behavioral change
- The `tokio::select!` pattern at the call site must be preserved — the `runner.run_agent(prompt, &result_path, timeout).await` call replaces the `run_workflows_sequentially(runner, &prompt, &result_path, timeout)` call as the awaited expression in the select branch

## Dependencies

- **Depends On:** None
- **Blocks:** None

## Risks

- None — this is a mechanical inline of a single-line function with one call site

## Open Questions

None.

## Assumptions

- The backlog item title references "run_skills_sequentially" but the function is named `run_workflows_sequentially` in the current codebase (renamed during WRK-003 implementation). This PRD targets the function by its current name. The WRK-003 SPEC also references the old name.
- The function is private (`async fn`, not `pub`) — no external API impact from removal.

## References

- `src/executor.rs:469-480` — function definition
- `src/executor.rs:395` — sole call site
- `changes/WRK-003_orchestrator-pipeline-engine-v2/WRK-003_orchestrator-pipeline-engine-v2_SPEC.md` — original tech debt identification
