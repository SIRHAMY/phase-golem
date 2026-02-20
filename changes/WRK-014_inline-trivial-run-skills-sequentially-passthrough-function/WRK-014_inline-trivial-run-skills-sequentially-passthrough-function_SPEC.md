# SPEC: Inline trivial `run_workflows_sequentially` passthrough function

**ID:** WRK-014
**Status:** Complete
**Created:** 2026-02-19
**PRD:** ./WRK-014_inline-trivial-run-skills-sequentially-passthrough-function_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

`run_workflows_sequentially` in `src/executor.rs` (lines 469–480) is a private, single-call-site passthrough that delegates directly to `runner.run_agent()` with no added logic. It was identified as tech debt during WRK-003 (SPEC note: "trivial passthrough — inline or defer"). This SPEC covers its removal and inlining at the call site.

## Approach

Classic "Inline Method" refactor: replace the function call at the single call site (line 395) with the function's one-line body, preserve the explanatory comment from the function body at the call site, and delete the function definition. Zero behavioral change — the `tokio::select!` topology, cancellation safety, and `Result<PhaseResult, String>` propagation are all identical before and after.

**Patterns to follow:**

- `src/executor.rs:393-397` — existing `tokio::select!` pattern for cancellation racing (preserved as-is, only the awaited expression changes)
- `src/executor.rs:469-480` — the function being inlined (body becomes the replacement code)

**Implementation boundaries:**

- Do not modify: `src/agent.rs` (AgentRunner trait), `tests/executor_test.rs` (tests pass without changes)
- Do not refactor: any other functions in `executor.rs` beyond the inline target

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Inline and delete | Low | Replace call with `runner.run_agent()`, move comment, delete function |

**Ordering rationale:** Single phase — the change is atomic and indivisible.

---

## Phases

### Phase 1: Inline and delete

> Replace `run_workflows_sequentially` call with direct `runner.run_agent()` invocation and delete the function

**Phase Status:** complete

**Complexity:** Low

**Goal:** Remove the passthrough function and inline its body at the single call site, preserving the explanatory comment.

**Files:**

- `src/executor.rs` — modify — inline call at ~line 395, delete function at ~lines 469–480

**Tasks:**

- [x] Pre-implementation check: run `grep -n "run_workflows_sequentially" src/executor.rs` to confirm the exact line numbers of the call site and function definition, and verify there is exactly one call site
- [x] Replace the comment `// Run workflows sequentially` above the `tokio::select!` block with the three-line comment from the function body:
  ```
  // Currently workflows are encoded in the prompt, and a single agent run
  // executes them all. Multi-workflow phases run as a single agent invocation
  // (the prompt lists all workflow files).
  ```
- [x] In the `tokio::select!` block, replace `run_workflows_sequentially(runner, &prompt, &result_path, timeout)` with `runner.run_agent(&prompt, &result_path, timeout)`. **Important:** Do not add an explicit `.await` — `tokio::select!` awaits the future implicitly. Ensure parameters match: `&prompt`, `&result_path`, `timeout` (same references as the original call site)
- [x] Delete the `run_workflows_sequentially` function definition (doc-comment, signature, and body — approximately lines 469–480)

**Verification:**

- [x] `cargo check` passes with no errors or warnings
- [x] `cargo test` passes — all existing tests pass without modification (confirms zero behavioral change)
- [x] `grep -r "run_workflows_sequentially" src/ tests/` returns no results (function fully removed)
- [x] `grep -A 3 "Currently workflows are encoded" src/executor.rs` confirms the three-line comment appears directly above the `tokio::select!` block
- [x] Code review: confirm the `tokio::select!` block structure is preserved — same `.await` topology, same cancellation branch, same parameter references (`&prompt`, `&result_path`, `timeout`)

**Commit:** `[WRK-014][P1] Clean: Inline run_workflows_sequentially at call site`

**Notes:**

- Line numbers are approximate — the pre-implementation check task verifies actual positions before editing.
- The tech research doc's inlined code sample incorrectly shows `.await` inside `select!`. The existing code and Design doc correctly omit it. The task description above includes the correct guidance.

**Followups:**

None.

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met:
  - [x] `run_workflows_sequentially` removed from `src/executor.rs`
  - [x] `runner.run_agent(...)` inlined at the call site in `execute_phase`
  - [x] Code compiles without errors or warnings (`cargo check`)
  - [x] All existing tests pass without modification (`cargo test`)
  - [x] No behavioral change (verified by test passage)
  - [x] Inlined call retains the existing comment from the function body
- [x] Tests pass
- [x] No regressions introduced

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1: Inline and delete | complete | `[WRK-014][P1] Clean: Inline run_workflows_sequentially at call site` | All tasks done, all verification passed, code review clean |

## Followups Summary

### Critical

### High

### Medium

### Low

## Assumptions

- Running autonomously. No questions arose — the change is mechanical and fully specified by the PRD and Design.
- Line numbers (393–397, 469–480) are based on the current state of `src/executor.rs`. These will be verified before editing.
- The tech research doc's inlined code sample includes `.await` inside `tokio::select!`, but this is incorrect for how `select!` works (it awaits the future expression implicitly). The Design doc's code sample is correct and is what we follow.
