# Tech Research: Gate preflight Phase 3 item validation on Phase 1 structural validation passing

**ID:** WRK-015
**Status:** Complete
**Created:** 2026-02-19
**PRD:** ./WRK-015_PRD.md
**Mode:** Light

## Overview

Researching how to gate Phase 3 (item validation) in `run_preflight` on Phase 1 (structural validation) passing. The key questions are: what pattern to use for the gate, where exactly to place it, and what test fixtures are needed to verify the behavior. This is a small, well-understood change that follows an existing pattern already in the codebase.

## Research Questions

- [x] What pattern should we use for the Phase 3 gate?
- [x] Where exactly does the snapshot variable go in the code?
- [x] What test helpers and fixtures exist for writing the new tests?
- [x] Are there any gotchas with the existing error accumulation pattern?

---

## External Research

### Landscape Overview

Multi-phase validation systems commonly handle phase dependencies through three mechanisms: (1) cascade modes that stop execution on failure, (2) conditional execution with guard clauses, and (3) explicit dependency graphs. For simple cases with 1-2 dependencies, guard clauses are the standard approach. Dependency graphs are appropriate when systems grow to 3+ phases with complex interdependencies.

### Common Patterns & Approaches

#### Pattern: Guard Clause (Conditional Execution)

**How it works:** Before executing a phase, check a boolean condition (e.g., "did Phase 1 pass?"). If false, skip the phase.

**When to use:** Simple cases with 1-2 conditional dependencies where the dependency logic is straightforward.

**Tradeoffs:**
- Pro: Simple to implement and understand, minimal overhead
- Pro: Easy to reason about at the call site
- Con: Doesn't scale to complex dependency graphs (not a concern here)

**Common technologies:** Used in FluentValidation (CascadeMode.Stop), CI/CD pipelines (GitLab `when:`, Azure Pipelines `dependsOn`), and custom validation frameworks.

**References:**
- [FluentValidation Cascade Mode](https://docs.fluentvalidation.net/en/latest/cascade.html) — Stop-on-failure pattern for sequential validators
- [Azure Pipelines Stages](https://learn.microsoft.com/en-us/azure/devops/pipelines/process/stages?view=azure-devops) — Dependency graph pattern in CI/CD
- [Concourse CI Gated Pipeline Patterns](https://concourse-ci.org/gated-pipeline-patterns.html/) — Sequential pipeline execution with gates

#### Pattern: Dependency Graph (DAG)

**How it works:** Phases declare explicit dependencies, execution engine runs phases only when dependencies succeed.

**When to use:** Complex pipelines with many phases and interdependencies. Overkill for our 5-phase linear pipeline with 2 simple gates.

**Tradeoffs:**
- Pro: Explicit, scalable, allows parallelization
- Con: More setup/configuration, harder to refactor

### Standards & Best Practices

- **Three-tier validation:** Structure (schema/types) -> Relational (cross-field) -> Semantic (business logic). Later tiers depend on earlier ones passing. Our Phase 1 -> Phase 3 dependency follows this exactly.
- **Fail fast, but log everything:** Use guards to stop execution quickly. Maintain visibility into what was skipped.
- **Track skip vs. fail:** Distinguish between phases that failed and phases that were skipped.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Gating on cumulative error state instead of specific phase | Gate becomes fragile to phase reordering; adding a new phase between 1 and 3 would change gate semantics | Use a snapshot variable captured immediately after the specific phase |
| Over-constraining the pipeline | Making all phases sequential blocks parallelization and hides independent errors | Only gate phases that have genuine semantic dependencies |
| Silent skips without documentation | Future maintainers don't understand why a phase was skipped | Comment the gate condition clearly |

### Key Learnings

- Guard clause pattern is the correct choice for this use case — simple, proven, already established in the codebase
- Snapshot variable is the right refinement over inline `errors.is_empty()` check — isolates gate to specific phase results
- No need for skip logging (consistent with existing Phase 2 gate behavior)

---

## Internal Research

### Existing Codebase State

The preflight validation system in `src/preflight.rs` runs 5 sequential phases that accumulate errors into a single `Vec<PreflightError>`. Phase 2 already implements the exact gate pattern needed for Phase 3. The change is a 3-line addition following this established pattern.

**Relevant files/modules:**

| File | Purpose | Relevance |
|------|---------|-----------|
| `src/preflight.rs` | Core validation engine with all 5 phases | **Primary change target.** Contains `run_preflight` (lines 38-68), `validate_structure` (lines 76-175), `validate_items` (lines 222-299), Phase 2 gate (lines 50-52) |
| `tests/preflight_test.rs` | Full test suite for all preflight phases | **Test additions needed.** 60+ existing tests, Phase 3 tests at lines 316-461, test helpers at lines 21-41 |
| `tests/common/mod.rs` | Shared test utilities | **Reusable.** Provides `make_item()`, `make_backlog()` helpers |
| `src/main.rs` | Entry point calling `run_preflight` | **Integration point** at line 579. No changes needed — same function signature |

**Existing patterns in use:**
- Phase 2 gate pattern: `if errors.is_empty() { errors.extend(probe_workflows(...)); }` at lines 50-52
- Error accumulation: all phases call `errors.extend()` on the same vector
- Final check: `if errors.is_empty() { Ok(()) } else { Err(errors) }` at lines 63-67

### Reusable Components

- **Phase 2 gate pattern** — identical pattern to replicate, with snapshot variable refinement
- **`feature_pipeline_no_workflows()`** (test helper, line 21) — creates a feature pipeline without workflow files, useful for testing Phase 3 in isolation
- **`default_config()`** (test helper, line 35) — wraps the no-workflows pipeline into a config
- **`make_item(id, status)`** (common helper) — creates BacklogItem with minimal defaults
- **`make_backlog(vec![...])`** (common helper) — wraps items into BacklogFile
- **`make_feature_item(id, status)`** (preflight test helper) — extends make_item with pipeline_type="feature"

### Constraints from Existing Code

- **Function signature immutable:** `run_preflight` returns `Result<(), Vec<PreflightError>>` — cannot change
- **Error type is flat:** `PreflightError` has no phase tag or severity — all errors are equivalent in the returned vector
- **No skip/info channel:** Only errors are returned, no mechanism for "Phase 3 was skipped" messages. Consistent with Phase 2 gate behavior (no skip message there either)
- **Phases 4-5 must remain ungated:** They operate on backlog item fields only, never dereference pipeline config

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Snapshot variable approach is correct | Confirmed — inline `errors.is_empty()` after Phase 2 would gate on Phase 1+2, not Phase 1 alone. Snapshot isolates correctly. | No concern — PRD is accurate |
| Phase 2 gate is the model to follow | Confirmed — same pattern, same style, same codebase conventions | No concern — PRD is accurate |
| Phases 4-5 should remain ungated | Confirmed — they operate on `id`, `dependencies`, `status` fields from backlog items, never pipeline config | No concern — PRD is accurate |

No PRD concerns found. The PRD is precise and well-aligned with the codebase reality.

---

## Critical Areas

### Snapshot Variable Placement

**Why it's critical:** The snapshot must be captured after Phase 1 (line 47) but before Phase 2 (line 50). If placed after Phase 2, the gate would be on Phase 1+2 combined, violating the PRD requirement that Phase 3 should still run when Phase 1 passes but Phase 2 fails.

**Why it's easy to miss:** A naive implementation might just add another `if errors.is_empty()` before Phase 3, but after Phase 2 has already run. This would gate Phase 3 on Phase 2 failures too, which is incorrect.

**What to watch for:** The snapshot line `let structural_ok = errors.is_empty();` must go on line 48 (after Phase 1, before Phase 2 gate check).

---

## Deep Dives

No deep dives needed — research was sufficient at light mode depth.

---

## Synthesis

### Open Questions

No open questions. The change is fully understood.

### Recommended Approaches

#### Gate Implementation

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Snapshot variable + guard clause | Isolates gate to Phase 1 only; robust to future phase reordering; follows PRD "Should Have" | One extra line of code (trivial) | Always — this is the recommended approach |
| Inline `errors.is_empty()` before Phase 3 | Slightly less code | Gates on Phase 1+2 combined, not Phase 1 alone; breaks if phases reorder | Never — incorrect for our requirements |

**Initial recommendation:** Snapshot variable approach. It's explicit, correct, and robust.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| `src/preflight.rs` lines 50-52 | Existing code | Phase 2 gate pattern — the model to follow |
| `tests/preflight_test.rs` lines 316-461 | Existing tests | Phase 3 test patterns to follow for new tests |
| [FluentValidation Cascade Mode](https://docs.fluentvalidation.net/en/latest/cascade.html) | External docs | Confirms guard/cascade pattern is standard practice |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-19 | Light mode: parallel internal + external research | Confirmed guard clause with snapshot variable is correct approach; identified exact code locations and test patterns |
| 2026-02-19 | PRD analysis | No concerns — PRD is precise and matches codebase reality |

## Assumptions

- **No human available for Q&A** — proceeding directly to finalization since the change is small, well-understood, and the PRD is precise. No deep dives needed.
- **Light mode is sufficient** — the change follows an established pattern with exact code precedent 3 lines above the insertion point. Heavy research would not yield additional value.
