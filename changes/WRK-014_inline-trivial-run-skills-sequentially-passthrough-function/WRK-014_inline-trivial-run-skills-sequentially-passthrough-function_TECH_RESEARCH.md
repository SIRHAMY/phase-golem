# Tech Research: Inline trivial `run_workflows_sequentially` passthrough function

**ID:** WRK-014
**Status:** Complete
**Created:** 2026-02-19
**PRD:** ./WRK-014_inline-trivial-run-skills-sequentially-passthrough-function_PRD.md
**Mode:** Light

## Overview

Researching whether the `run_workflows_sequentially` function in `src/executor.rs` can be safely inlined at its single call site within a `tokio::select!` block. Key questions: Is it truly a passthrough? Are there cancellation safety implications? Does the compiler already inline it?

## Research Questions

- [x] Is `run_workflows_sequentially` a true passthrough with no added logic?
- [x] Are there cancellation safety concerns when inlining within `tokio::select!`?
- [x] Does the Rust/LLVM compiler already inline this automatically?
- [x] Are there any borrow lifetime issues with direct inlining?

---

## External Research

### Landscape Overview

This refactor is the classic "Inline Method" pattern from Fowler's refactoring catalog. In Rust, the additional considerations are: (1) LLVM's aggressive inlining of private, single-call-site functions already handles this at the machine code level, so the value is purely source-level readability; and (2) `tokio::select!` cancellation safety depends on `.await` point topology, not function wrapping depth.

### Common Patterns & Approaches

#### Pattern: Inline Method (Fowler)

**How it works:** Replace a function call with the function's body at the call site, then delete the function.

**When to use:** When "a method body is more obvious than the method itself" — single call site, no added logic, name adds no information beyond what the inlined code already communicates.

**Tradeoffs:**
- Pro: Eliminates unnecessary cognitive indirection
- Con: Loses a named abstraction point (irrelevant here — the name is arguably misleading since it doesn't actually run multiple workflows sequentially)

**References:**
- [Inline Method - refactoring.guru](https://refactoring.guru/inline-method) — classic pattern description

#### Pattern: LLVM Automatic Inlining

**How it works:** LLVM's inlining heuristics aggressively inline small private functions within the same crate, especially those with a single call site, at `-O1` and above.

**When to use:** Understanding this confirms the manual inline has no performance implications — it's purely a readability change.

**Tradeoffs:**
- Pro: Confirms no performance regression from either keeping or removing the wrapper
- Con: N/A

**References:**
- [Inline In Rust - matklad](https://matklad.github.io/2021/07/09/inline-in-rust.html) — comprehensive `#[inline]` analysis
- [Inlining - The Rust Performance Book](https://nnethercote.github.io/perf-book/inlining.html) — official performance guidance

### Standards & Best Practices

- **Rust Performance Book:** "The best candidates for inlining are functions that are very small or have a single call site, and the compiler will often inline these itself."
- **matklad:** Apply `#[inline]` reactively based on profiling, not proactively. The main use is cross-crate inlining, irrelevant for private functions.
- **Tokio `select!` docs:** Cancellation safety depends on `.await` point structure, not function wrapping depth.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Changing `.await` topology in `select!` | Adding/reordering `.await` points changes cancellation behavior | Verify identical `.await` count before and after (both have exactly one) |
| Inlining polymorphic methods | Would break trait dispatch | N/A — this is a private free function |
| Accidentally changing borrow lifetimes | Could cause compilation errors | Verify all borrows at call site match function parameters (they do) |

### Key Learnings

- The `.await` topology is unchanged: one `.await` on `runner.run_agent(...)` before and after inlining
- LLVM already inlines this at the machine code level; the refactor is purely source-level
- No borrow lifetime issues — all parameters are already available as borrows at the call site

---

## Internal Research

### Existing Codebase State

The executor module drives phase execution through a retry loop with `tokio::select!` for cancellation support. The architecture separates data structures (`types.rs`), behavior (pure functions in `executor.rs`), and agent execution (trait-based via `AgentRunner` in `agent.rs`).

**Relevant files/modules:**
- `src/executor.rs` — Contains both the function definition (lines 469-480) and the single call site (line 395)
- `src/agent.rs` — Defines `AgentRunner` trait (lines 115-123) with `run_agent` method
- `tests/executor_test.rs` — 1,059 lines of tests, none referencing `run_workflows_sequentially` directly

**Function definition (lines 469-480):**
```rust
/// Run all workflows for a phase sequentially. If any workflow fails, the phase fails.
async fn run_workflows_sequentially(
    runner: &impl AgentRunner,
    prompt: &str,
    result_path: &Path,
    timeout: Duration,
) -> Result<PhaseResult, String> {
    // Currently workflows are encoded in the prompt, and a single agent run
    // executes them all. Multi-workflow phases run as a single agent invocation
    // (the prompt lists all workflow files).
    runner.run_agent(prompt, result_path, timeout).await
}
```

**Call site (lines 393-397):**
```rust
        // Run workflows sequentially
        let workflow_result = tokio::select! {
            result = run_workflows_sequentially(runner, &prompt, &result_path, timeout) => result,
            _ = cancel.cancelled() => return PhaseExecutionResult::Cancelled,
        };
```

**Existing patterns in use:**
- Trait-based abstraction (`AgentRunner`) for testability
- `tokio::select!` for cancellation racing
- `Result<PhaseResult, String>` error handling
- Pure function composition for helpers

### Reusable Components

- `runner.run_agent()` — direct trait method call, no wrapper needed
- `MockAgentRunner` — existing test infrastructure handles all scenarios
- Cancellation token pattern — already established in `execute_phase`

### Constraints from Existing Code

- The `tokio::select!` pattern must be preserved exactly
- The function is private (`async fn`, not `pub`) — no external API impact
- Single call site confirmed by grep — only reference is `src/executor.rs:395`
- No tests reference `run_workflows_sequentially` directly

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Function is a trivial passthrough | Confirmed — body is `runner.run_agent(prompt, result_path, timeout).await` with no logic | No concerns |
| Single call site at line 395 | Confirmed by grep across entire codebase | No concerns |
| tokio::select! pattern must be preserved | Cancellation safety is unchanged — identical `.await` topology | No concerns |
| No tests reference the function | Confirmed — all 1,059 test lines go through `execute_phase` | No concerns |

No divergences between PRD assumptions and research findings.

---

## Critical Areas

None identified. This is a mechanical inline of a private, single-call-site, zero-logic passthrough function with no cancellation safety implications.

---

## Deep Dives

No deep dives needed for light-mode research on this trivial refactor.

---

## Synthesis

### Open Questions

None. All research questions are resolved.

### Recommended Approaches

#### Inlining Strategy

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Direct inline with comment preservation | Clearer code, preserves domain context, eliminates indirection | Loses named abstraction point | Function is trivial passthrough (this case) |

**Initial recommendation:** Direct inline with comment preservation. This is the only viable approach for a zero-logic passthrough.

**Inlined code should look like:**
```rust
        // Currently workflows are encoded in the prompt, and a single agent run
        // executes them all. Multi-workflow phases run as a single agent invocation
        // (the prompt lists all workflow files).
        let workflow_result = tokio::select! {
            result = runner.run_agent(&prompt, &result_path, timeout).await => result,
            _ = cancel.cancelled() => return PhaseExecutionResult::Cancelled,
        };
```

Then delete lines 469-480 (the `run_workflows_sequentially` function).

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Inline Method - refactoring.guru](https://refactoring.guru/inline-method) | Article | Classic pattern description |
| [Inline In Rust - matklad](https://matklad.github.io/2021/07/09/inline-in-rust.html) | Article | Confirms LLVM handles inlining for private functions |
| [Inlining - Rust Performance Book](https://nnethercote.github.io/perf-book/inlining.html) | Docs | Official guidance on inlining |
| [tokio::select! docs](https://docs.rs/tokio/latest/tokio/macro.select.html) | Docs | Cancellation safety semantics |
| [Async Rust gotcha: tokio::select! cancellation](https://biriukov.dev/posts/async-rust-gocha-tokio-cancelation-select-future-then/) | Article | Cancellation safety deep dive |

---

## Assumptions

- Running autonomously without human input. No questions arose that required human judgment — the research confirms all PRD assumptions.

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-19 | Light external research on Rust inlining and tokio::select! cancellation safety | Confirmed no concerns |
| 2026-02-19 | Light internal research on function definition, call site, and test coverage | Confirmed all PRD assumptions |
| 2026-02-19 | PRD analysis | No divergences found |
