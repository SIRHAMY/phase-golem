# Change: Gate preflight Phase 3 item validation on Phase 1 structural validation passing

**Status:** Proposed
**Created:** 2026-02-19
**Author:** Claude (autonomous)

## Problem Statement

The `run_preflight` function in `src/preflight.rs` runs five sequential validation phases. Phase 2 (workflow probe) is correctly gated on Phase 1 (structural validation) passing — it only runs when `errors.is_empty()`. However, Phase 3 (item validation) runs unconditionally, even when Phase 1 has already found structural errors in the pipeline configuration.

Phase 3 (`validate_items`) looks up pipelines and phases by name from the config — the same structures that Phase 1 validates. When Phase 1 finds that a pipeline has no entries in its `phases` array (main phases, as opposed to `pre_phases`), has duplicate phase names, or other structural defects, Phase 3 proceeds to validate active items (those with status `InProgress` or `Scoping`) against those broken pipelines and produces secondary errors that are confusing and misleading. A user sees errors like "pipeline has no phases" alongside "item references unknown phase in that pipeline" when only the first is a root cause and the second is a consequence.

This is a correctness gap in the validation ordering: Phase 3 has a semantic dependency on Phase 1 that is not enforced in code.

## User Stories / Personas

- **Orchestrator User** — When editing `phase-golem.toml` and introducing a structural error, wants to see only the root-cause errors from preflight, not confusing secondary errors from downstream validation phases that ran against known-broken data.

## Desired Outcome

When Phase 1 structural validation finds errors, Phase 3 item validation is skipped entirely. The user sees only the structural errors, fixes them, and on the next run gets clean Phase 1 results followed by meaningful Phase 3 item validation. The error output is always actionable and never contains misleading secondary errors caused by upstream structural failures.

Phases 4 (duplicate ID validation) and 5 (dependency graph validation) continue to run unconditionally. They operate only on backlog item fields (`id`, `dependencies`, `status`) from `BACKLOG.yaml`, never dereference pipeline config, and detect real errors (duplicate IDs, broken dependency chains) regardless of whether the pipeline config is structurally sound.

## Success Criteria

### Must Have

- [ ] Phase 3 (`validate_items`) is skipped when Phase 1 (`validate_structure`) produces any error — the gate is on all `validate_structure` errors, not selectively on specific error types
- [ ] Phase 3 still runs when Phase 1 passes but Phase 2 (`probe_workflows`, which verifies workflow files exist on disk) fails — the gate is on Phase 1 only, not on all prior phases
- [ ] Phases 4 and 5 continue to run unconditionally regardless of Phase 1 results
- [ ] A test verifies Phase 3 is suppressed when Phase 1 fails (e.g., pipeline has no main phases) and an `InProgress` item references that broken pipeline — only the Phase 1 error appears, not a secondary Phase 3 error
- [ ] A test verifies Phase 3 still runs when Phase 1 passes but Phase 2 fails (missing workflow file) with an `InProgress` item present
- [ ] Existing tests pass without modification
- [ ] `cargo clippy` produces no new warnings

### Should Have

- [ ] The gate uses a snapshot variable (e.g., `let structural_ok = errors.is_empty()`) captured after Phase 1 but before Phase 2, rather than checking `errors.is_empty()` inline after Phase 2 — this ensures the gate scope is precisely "Phase 1 passed" and is robust against future phase reordering

### Nice to Have

- [ ] Doc comment on `run_preflight` updated to note that Phase 3 is gated on Phase 1 passing

## Scope

### In Scope

- Adding a gate condition in `run_preflight` to skip Phase 3 when Phase 1 fails
- Using a snapshot variable to isolate the gate to Phase 1 results only
- Adding test coverage for the new gating behavior

### Out of Scope

- Gating Phase 4 (duplicate IDs) or Phase 5 (dependency graph) — they have no dependency on config structure
- Gating Phase 3 on Phase 2 — missing workflow files do not invalidate pipeline/phase name lookups
- Adding a "Phase 3 skipped" diagnostic message to the output — there is no current mechanism for informational messages in `run_preflight`, only errors
- Restructuring the phase runner into a general-purpose pipeline with typed gates
- Selective gating (only skip Phase 3 for specific Phase 1 error types) — the blunt gate is simpler, consistent with Phase 2's approach, and sufficient

## Non-Functional Requirements

- **Performance:** No measurable impact. The change skips a pure-computation validation function when it would produce meaningless results, which is marginally faster in the error case.

## Constraints

- Must use the same gating pattern established by Phase 2 (conditional on prior errors) for consistency
- Must not change the `run_preflight` function signature or return type
- The gate must be specifically on Phase 1 results, not on the cumulative error state after Phase 2

## Dependencies

- **Depends On:** Nothing — standalone fix
- **Blocks:** Nothing directly, but improves error UX for any user encountering config structural errors during active work

## Risks

- [ ] **Minimal risk:** The change is a single conditional guard clause following an established pattern already used for Phase 2. The validation logic itself is unchanged.
- [ ] **Reduced diagnostic output in multi-failure scenarios:** A user with both a structural error and an independent item error will now only see the structural error first. This is the intended "fix one layer at a time" behavior and matches how Phase 2 already works.

## Assumptions

- The gate should be on Phase 1 specifically, not on "all prior phases passed." This is because Phase 3 has a semantic dependency on config structure (Phase 1) but not on workflow file existence (Phase 2). A snapshot variable captured after Phase 1 achieves this cleanly.
- Phases 4 and 5 should remain ungated. They operate only on backlog item fields (`id`, `dependencies`, `status`) and never dereference pipeline config. Their errors are real correctness issues regardless of whether the pipeline config is structurally sound.
- In production, `config::validate()` runs during config loading before `run_preflight` is called, so Phase 1 errors are rare in normal operation. The gate's primary value is preventing cascading secondary errors and ensuring all reported errors are actionable root causes, particularly in test contexts or when configs bypass `load_config`.
- `run_preflight` is stateless — each invocation creates a fresh error vector. Errors are not carried over between runs, so after a user fixes Phase 1 errors and re-runs, Phase 3 will execute normally.
- No "Phase 3 skipped" informational message is needed. The Phase 2 gate does not emit a skip message, and consistency with that precedent is preferred over adding new output mechanisms.

## References

- `src/preflight.rs` — `run_preflight` (lines ~38-68), `validate_structure` (lines ~76-175), `validate_items` (lines ~222-299), Phase 2 gate pattern (lines ~50-52). Line numbers are approximate; see actual function definitions.
- `tests/preflight_test.rs` — existing test coverage for all phases
