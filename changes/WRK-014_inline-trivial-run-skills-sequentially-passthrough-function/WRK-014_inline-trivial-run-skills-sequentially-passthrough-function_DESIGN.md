# Design: Inline trivial `run_workflows_sequentially` passthrough function

**ID:** WRK-014
**Status:** Complete
**Created:** 2026-02-19
**PRD:** ./WRK-014_inline-trivial-run-skills-sequentially-passthrough-function_PRD.md
**Tech Research:** ./WRK-014_inline-trivial-run-skills-sequentially-passthrough-function_TECH_RESEARCH.md
**Mode:** Light

## Overview

Inline the trivial `run_workflows_sequentially` passthrough function at its single call site in `execute_phase`, then delete the function. This is a mechanical "Inline Method" refactor that removes one layer of unnecessary indirection with zero behavioral change. The existing comment from the function body is preserved at the call site to retain domain context.

---

## System Design

### High-Level Architecture

No architectural changes. This refactor operates entirely within `src/executor.rs`, replacing a function call with the function's single-line body. The module's structure, trait boundaries, and data flow are unchanged.

### Component Breakdown

#### `execute_phase` (modified)

**Purpose:** Drives phase execution with retry logic and cancellation support.

**Change:** The `run_workflows_sequentially(runner, &prompt, &result_path, timeout)` call at line 395 is replaced with `runner.run_agent(&prompt, &result_path, timeout)` — the direct trait method call that was previously wrapped.

**Interfaces:** Unchanged. Same inputs, same outputs, same `.await` topology within `tokio::select!`.

#### `run_workflows_sequentially` (deleted)

**Purpose:** Was a passthrough wrapper around `runner.run_agent()`.

**Change:** Removed entirely — including the doc-comment (`/// Run all workflows for a phase sequentially...`), function signature, and body. The inline comment (three lines) is preserved at the call site; the doc-comment is deleted since the function no longer exists.

### Data Flow

Unchanged. The data flow before and after is identical:

1. `execute_phase` builds a prompt and result path
2. `runner.run_agent(prompt, result_path, timeout)` is awaited inside `tokio::select!` racing against cancellation
3. The result is matched and processed

### Key Flows

#### Flow: Phase Execution (happy path)

> Inline the agent invocation directly in the `tokio::select!` block.

**Before (lines 393-397):**
```rust
        // Run workflows sequentially
        let workflow_result = tokio::select! {
            result = run_workflows_sequentially(runner, &prompt, &result_path, timeout) => result,
            _ = cancel.cancelled() => return PhaseExecutionResult::Cancelled,
        };
```

**After:**
```rust
        // Currently workflows are encoded in the prompt, and a single agent run
        // executes them all. Multi-workflow phases run as a single agent invocation
        // (the prompt lists all workflow files).
        let workflow_result = tokio::select! {
            result = runner.run_agent(&prompt, &result_path, timeout) => result,
            _ = cancel.cancelled() => return PhaseExecutionResult::Cancelled,
        };
```

**Edge cases:**
- Cancellation — unchanged; `tokio::select!` still races the same `.await` against `cancel.cancelled()`
- Agent failure — unchanged; `Result<PhaseResult, String>` propagation is identical

---

## Technical Decisions

### Key Decisions

#### Decision: Preserve the function body comment at the call site

**Context:** The `run_workflows_sequentially` function contains a three-line comment explaining why a single agent invocation handles all workflows. This context is useful for future maintainers.

**Decision:** Move the comment to directly above the `tokio::select!` block, replacing the old `// Run workflows sequentially` comment.

**Rationale:** The comment explains a non-obvious design choice (single invocation for multi-workflow phases). Without it, a reader might wonder why there's no loop or sequential execution logic.

**Consequences:** Slightly longer comment block at the call site, but retains important domain context.

#### Decision: Drop the `// Run workflows sequentially` comment

**Context:** The old single-line comment at line 393 described what `run_workflows_sequentially` did. After inlining, this comment would be redundant with the more detailed comment moved from the function body.

**Decision:** Replace it with the more descriptive three-line comment from the function body.

**Rationale:** The function-body comment is strictly more informative.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Loss of named abstraction | `run_workflows_sequentially` no longer exists as a named function | Directness: reader sees the actual `run_agent` call without following an indirection | The name was misleading (no sequential execution of multiple workflows happens) and the function added no logic |

---

## Alternatives Considered

### Alternative: Keep the function and add `#[inline]`

**Summary:** Leave `run_workflows_sequentially` in place but annotate with `#[inline]` to hint the compiler.

**How it would work:**
- Add `#[inline]` attribute to the function
- No other changes

**Pros:**
- Preserves a named abstraction point for future multi-workflow orchestration
- Zero risk of introducing bugs

**Cons:**
- `#[inline]` is unnecessary — LLVM already inlines private single-call-site functions
- The function name is misleading (no sequential multi-workflow execution occurs)
- Keeps an unnecessary layer of indirection for readers

**Why not chosen:** The function adds no value — it doesn't abstract complexity, its name is misleading, and the compiler already handles inlining. This alternative would preserve a known tech debt item identified in WRK-003 without addressing it.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| None identified | — | — | This is a mechanical inline of a single-line private function with one call site. Verification: `cargo check` confirms compilation, `cargo test` confirms no behavioral change. Parameter matching (`&prompt`, `&result_path`, `timeout`) can be confirmed via code review diff. |

---

## Integration Points

### Existing Code Touchpoints

- `src/executor.rs:393-397` — Call site modified (inline the function body)
- `src/executor.rs:469-480` — Function definition deleted (doc-comment, signature, and body)

### External Dependencies

None.

---

## Open Questions

None.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-19 | Initial design draft (light mode) | Straightforward inline method refactor — single approach, one alternative briefly noted |
| 2026-02-19 | Self-critique (7 agents) | No critical or high issues. Auto-fixed: clarified doc-comment deletion, added verification strategy detail. All remaining items were quality-level and not applicable for this trivial refactor. |
