# Design: Gate preflight Phase 3 item validation on Phase 1 structural validation passing

**ID:** WRK-015
**Status:** Complete
**Created:** 2026-02-19
**PRD:** ./WRK-015_PRD.md
**Tech Research:** ./WRK-015_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add a guard clause to `run_preflight` that skips Phase 3 (item validation) when Phase 1 (structural validation) has produced errors. The gate uses a snapshot variable captured immediately after Phase 1, before Phase 2 runs, so the gate is precisely "Phase 1 passed" — not "all prior phases passed." This follows the identical pattern already established by the Phase 2 gate at lines 50-52 of `src/preflight.rs`, with the addition of a snapshot variable to isolate the gate scope.

---

## System Design

### High-Level Architecture

No new components. The change adds two lines to the existing `run_preflight` function in `src/preflight.rs`:

1. A snapshot variable (`let structural_ok = errors.is_empty();`) captured after Phase 1 completes but before Phase 2 begins
2. A conditional guard (`if structural_ok { ... }`) around the Phase 3 call

The rest of the validation pipeline — Phases 1, 2, 4, and 5 — is unchanged.

### Component Breakdown

#### `run_preflight` (modified)

**Purpose:** Orchestrates 5 sequential validation phases, accumulating errors into a single vector.

**Change:** Insert snapshot capture after Phase 1, gate Phase 3 on that snapshot.

**Current flow (lines 44-67):**
```
let mut errors = Vec::new();
Phase 1: errors.extend(validate_structure(config));
Phase 2: if errors.is_empty() { errors.extend(probe_workflows(...)); }
Phase 3: errors.extend(validate_items(config, backlog));          // ← UNCONDITIONAL (bug)
Phase 4: errors.extend(validate_duplicate_ids(&backlog.items));
Phase 5: errors.extend(validate_dependency_graph(&backlog.items));
return Ok(()) or Err(errors)
```

**Proposed flow:**
```
let mut errors = Vec::new();
Phase 1: errors.extend(validate_structure(config));
let structural_ok = errors.is_empty();                             // ← NEW: snapshot
Phase 2: if errors.is_empty() { errors.extend(probe_workflows(...)); }
Phase 3: if structural_ok { errors.extend(validate_items(...)); }  // ← CHANGED: gated
Phase 4: errors.extend(validate_duplicate_ids(&backlog.items));
Phase 5: errors.extend(validate_dependency_graph(&backlog.items));
return Ok(()) or Err(errors)
```

**Interfaces:** Unchanged. Same function signature, same return type.

**Dependencies:** None new.

### Data Flow

1. Phase 1 runs `validate_structure(config)` → returns `Vec<PreflightError>`
2. Errors are extended into the accumulator
3. **NEW:** `structural_ok` captures whether the accumulator is empty (i.e., Phase 1 passed)
4. Phase 2 gate checks `errors.is_empty()` (same as before — this is equivalent to `structural_ok` at this point, but uses the existing pattern for consistency)
5. Phase 2 may add errors
6. **CHANGED:** Phase 3 gate checks `structural_ok` (not `errors.is_empty()`, which would now reflect Phase 2 errors too)
7. Phases 4 and 5 run unconditionally

### Key Flows

#### Flow: Preflight with structural errors present

> When Phase 1 finds config problems, Phase 3 is skipped to avoid misleading secondary errors.

1. **Phase 1 runs** — Finds structural error (e.g., pipeline has no main phases)
2. **Snapshot captured** — `structural_ok = false`
3. **Phase 2 skipped** — Existing gate: `errors.is_empty()` is false
4. **Phase 3 skipped** — New gate: `structural_ok` is false
5. **Phases 4-5 run** — Unconditional; check backlog-level issues independent of config structure
6. **Return** — Only Phase 1 (and possibly Phase 4/5) errors returned

**Edge cases:**
- Phase 1 fails + Phase 2 would also fail → Phase 2 already skipped by existing gate, Phase 3 now also skipped. Only Phase 1 errors shown.
- Phase 1 fails + item has both structural and item-level issues → Only structural errors shown. User fixes structural issues, re-runs, then sees item-level errors.

#### Flow: Preflight with Phase 2 errors only

> When Phase 1 passes but Phase 2 fails (missing workflow files), Phase 3 still runs.

1. **Phase 1 runs** — No errors found
2. **Snapshot captured** — `structural_ok = true`
3. **Phase 2 runs** — Finds missing workflow file, adds error
4. **Phase 3 runs** — `structural_ok` is true, so Phase 3 executes normally
5. **Phases 4-5 run** — Unconditional
6. **Return** — Phase 2 and Phase 3 errors (if any) both present

This is correct because Phase 3 has a semantic dependency on config structure (Phase 1) but not on workflow file existence (Phase 2). Item validation looks up pipeline and phase names, not workflow files.

---

## Technical Decisions

### Key Decisions

#### Decision: Snapshot variable vs. inline `errors.is_empty()`

**Context:** Phase 3's gate must check whether Phase 1 passed, not whether all prior phases passed. After Phase 2 runs and potentially adds errors, `errors.is_empty()` reflects Phase 1+2 combined.

**Decision:** Use a snapshot variable `let structural_ok = errors.is_empty();` captured after Phase 1 but before Phase 2.

**Rationale:** The snapshot isolates the gate to Phase 1 results specifically. If a future change adds a new phase between 1 and 3, the snapshot remains correct. The inline approach would silently change behavior.

**Consequences:** One additional `let` binding. The intent is explicit and self-documenting.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Reduced output in multi-failure scenarios | User with both structural and item errors sees only structural errors first | Clean, non-misleading error output; "fix one layer at a time" UX | Matches Phase 2 gate behavior; secondary errors are misleading, not helpful |

---

## Alternatives Considered

### Alternative: Inline `errors.is_empty()` check before Phase 3

**Summary:** Instead of a snapshot variable, check `errors.is_empty()` inline before Phase 3, after Phase 2 has run.

**How it would work:**
- No snapshot variable needed
- `if errors.is_empty() { errors.extend(validate_items(...)); }` before Phase 3

**Pros:**
- One fewer line of code

**Cons:**
- Gates Phase 3 on Phase 1+2 combined, not Phase 1 alone
- Violates PRD requirement: Phase 3 should run when Phase 1 passes but Phase 2 fails
- Fragile to future phase reordering

**Why not chosen:** Semantically incorrect. Phase 3 depends on config structure (Phase 1) but not workflow file existence (Phase 2). This approach would suppress valid Phase 3 errors when the only problem is a missing workflow file.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Snapshot variable name unclear to future maintainers | Low — minor readability concern | Low | Variable name `structural_ok` is descriptive; existing code comment on Phase 1 provides context |

---

## Integration Points

### Existing Code Touchpoints

- `src/preflight.rs:47-55` — Insert snapshot variable after line 47, wrap Phase 3 call (line 55) in conditional. Two lines added, one line modified.

### External Dependencies

None. No new crates, no new I/O, no new types.

---

## Open Questions

None. The design is fully specified.

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
| 2026-02-19 | Initial design draft (light mode) | Snapshot variable + guard clause; single alternative noted and rejected |
| 2026-02-19 | Self-critique (7 agents) | No issues found — design is minimal and follows established pattern |
