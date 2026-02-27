# SPEC: Document Safe tg CLI Operations for Phase-Golem Items

**ID:** tg-8a8f7
**Status:** Ready
**Created:** 2026-02-27
**PRD:** ./tg-8a8f7_document-safe-tg-cli-operations-for-phase-golem-items_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

Phase-golem recently migrated its storage layer to task-golem (WRK-076), making `.task-golem/` the shared persistence layer. The `tg` CLI can operate directly on this store, but phase-golem decorates items with `x-pg-*` extension fields (currently 15, defined in `src/pg_item.rs:14-28`) that `tg` is unaware of. No documentation exists classifying which `tg` commands are safe to run against a phase-golem-managed store. This SPEC implements the documentation file designed in the Design doc.

## Approach

Create a single markdown file at `docs/tg-cli-safety.md` that classifies all `tg` CLI commands into three tiers (Safe, Caution, Dangerous) based on their effect on phase-golem's extended state. The document contains 21 entries covering 19 unique subcommands (`doctor` and `init` each appear in two tiers due to their `--fix` and `--force` flag variants having different safety profiles). Add a one-line discoverability link from the project README.

The document uses a tier-grouped structure (Safe first, escalating to Dangerous) with a quick-reference summary table, dual-status mapping table, per-tier command details, extension field reference, and recovery guidance. Content is sourced from the tech research document, which already contains pre-built tables for commands, extension fields, and status mapping. These will be expanded with the narrative and detail specified in the Design.

**Patterns to follow:**

- `README.md` — existing section structure for determining where to add the documentation link

**Implementation boundaries:**

- Do not modify any Rust source files
- Do not modify any configuration files
- Do not add tests (documentation-only change)
- Do not modify the existing README structure beyond adding a "Documentation" section and updating the Project Layout tree

## Open Questions

None — the scope, structure, and content are fully defined by the PRD, Design, and tech research.

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Create Safety Documentation | Low | Write `docs/tg-cli-safety.md` with all 8 sections and add README discoverability link |

**Ordering rationale:** Single phase — the entire deliverable is one documentation file plus a one-line README update. No dependencies between phases because there is only one.

---

## Phases

### Phase 1: Create Safety Documentation

> Write `docs/tg-cli-safety.md` with all 8 sections and add README discoverability link

**Phase Status:** complete

**Complexity:** Low

**Goal:** Create the complete tg CLI safety guide covering all tg subcommands across 3 tiers (21 entries covering 19 unique subcommands), with extension field reference, dual-status mapping, and recovery guidance. Add a discoverability link from README.md.

**Files:**

- `docs/tg-cli-safety.md` — create — primary deliverable; 8-section safety guide
- `README.md` — modify — add one-line link to `docs/tg-cli-safety.md` for discoverability

**Tasks:**

- [x] Create `docs/` directory
- [x] Gather current metadata for "last verified against" marker: date, phase-golem HEAD commit SHA, task-golem version (from `Cargo.toml` dependency declaration or `Cargo.lock`; fall back to `../task-golem/Cargo.toml` if needed), `src/pg_item.rs` line reference for extension field constants
- [x] Write Section 1: Header & Introduction — title, framing paragraph, "safe relative to phase-golem" statement, "last verified against" marker with all 4 fields (date, commit SHA, task-golem version, pg_item.rs line reference). Include a note for commands not listed: "If you encounter a command not listed here, consult `tg --help` or the task-golem CLI source."
- [x] Write Section 2: Quick-Reference Summary Table — 21 entries (19 unique subcommands, with `doctor`/`doctor --fix` and `init`/`init --force` as separate rows) with columns: Command | Tier | Description | Recommendation
- [x] Write Section 3: Dual-Status Mapping Table — narrative paragraph explaining the reading rule with concrete example, then 6-row mapping table (Phase-Golem Status | Task-Golem Status | x-pg-status Value | Notes)
- [x] Write Section 4: Safe Tier — 7 commands (`list`, `show`, `dump`, `next`, `ready`, `doctor`, `completions`), each with: name, what it does, why it's safe (1-2 sentences per command). Include warning callout on `doctor`: "If `tg doctor` reports issues, do NOT run `tg doctor --fix` without reading the Dangerous tier section first." Include note that operators remain responsible for how they use information from safe commands.
- [x] Write Section 5: Caution Tier — 5 commands (`do`, `done`, `todo`, `block`, `unblock`), each with: name, what it does, which specific phase-golem logic is bypassed, phase-golem equivalent if one exists (2-4 sentences per command)
- [x] Write Section 6: Dangerous Tier — 9 entries (`add`, `edit`, `rm`, `init`, `init --force`, `doctor --fix`, `dep add`, `dep rm`, `archive`), each with: name, what it does, specific data integrity risk, phase-golem equivalent or "avoid" warning (2-4 sentences per command). Special callouts for `init --force` (destroys all items and extensions) and `rm` (orphans change folder artifacts). Note for `init`: safe when creating a new store on a fresh directory, but categorized as Dangerous because it writes store files.
- [x] Write Section 7: Extension Field Reference — table listing all `x-pg-*` fields from `src/pg_item.rs:14-28` (currently 15) with columns: Field | Type | Purpose | If Missing/Corrupted. Source reference to `src/pg_item.rs` as authoritative. Note on enum case-sensitivity. Verification hint for operators.
- [x] Write Section 8: Recovery Guidance — 2-3 brief recovery scenarios (heading + 1-2 sentence description + 1-3 step remediation each): git history recovery for `.task-golem/tasks.jsonl`, `tg doctor --fix` limitations (what it can/cannot repair)
- [x] Add a discoverability link to `README.md` — add a "Documentation" section after "Project Layout" containing a link to `docs/tg-cli-safety.md`. Also add `docs/` entry to the Project Layout directory tree.

**Verification:**

- [x] All tg CLI subcommands are listed and classified into correct tiers per PRD categorization rules (Safe = no writes, Caution = status-only writes, Dangerous = create/modify/delete). Tier counts: Safe=7, Caution=5, Dangerous=9, total=21 entries covering 19 unique subcommands.
- [x] All `x-pg-*` extension fields from `src/pg_item.rs:14-28` are listed with name, type, purpose, and consequence if missing (verify count matches source)
- [x] Dual-status mapping table has 6 rows matching the mapping in `src/pg_item.rs:79-114`
- [x] Quick-reference summary table has 21 entries matching the tier detail sections
- [x] Caution and Dangerous commands explain which specific phase-golem logic is bypassed
- [x] `doctor` (Safe) and `doctor --fix` (Dangerous) are separate entries; Safe-tier `doctor` includes warning callout directing readers to Dangerous tier for `--fix`
- [x] `init` and `init --force` are separate entries with appropriate risk levels
- [x] Safe tier includes note that operators remain responsible for how they use information from safe commands
- [x] "Last verified against" marker includes all 4 fields: date, commit SHA, task-golem version, `src/pg_item.rs` line reference
- [x] Recovery Guidance section exists with at least 2 recovery scenarios
- [x] Introduction includes a note about commands not listed in the table
- [x] `docs/tg-cli-safety.md` exists at the correct path and README.md link resolves to it
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[tg-8a8f7][P1] Docs: Create tg CLI safety guide for phase-golem stores`

**Notes:**

Content for the command tables, extension field reference, and dual-status mapping is pre-built in the tech research document (`tg-8a8f7_feature_TECH_RESEARCH.md`). The implementation task is primarily about expanding this into the narrative structure specified in the Design, adding the "last verified against" marker with current metadata, and ensuring all PRD success criteria are met.

**Followups:**

- Code review H2: `tg archive` has dual behavior (recovery without flags, prune with `--before`). Current doc covers both modes but the Dangerous classification applies mainly to `--before`. A future refinement could split this into two entries (like `doctor`/`doctor --fix`) if the command set grows.

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met (must-have, should-have, nice-to-have)
- [x] No regressions introduced
- [x] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1 | Complete | pending | All 8 sections written, all verification items pass. Code review found 2 High issues (archive behavior inaccuracy, archive dual-mode) and 1 Medium (README init tree accuracy) — all fixed. |

## Followups Summary

### Critical

*(none)*

### High

*(none)*

### Medium

*(none)*

### Low

- [ ] Consider splitting `tg archive` into two entries (no-flag recovery mode vs. `--before` prune mode) similar to `doctor`/`doctor --fix`, if the command set grows — current single entry documents both modes but the Dangerous classification applies mainly to `--before` (Phase 1 code review)
- [ ] Consider adding a CI check or pre-commit hook that warns when `src/pg_item.rs` extension field count diverges from `docs/tg-cli-safety.md` — prevents silent staleness

## Design Details

### Content Sources

The tech research document contains pre-built tables that serve as starting content:

| Section | Tech Research Location | What to Expand |
|---------|----------------------|----------------|
| Command classification | Lines 97-133 (3 tables) | Add narrative per command: bypass details, phase-golem equivalents, risk descriptions |
| Extension field reference | Lines 136-157 | Already complete; verify against current `src/pg_item.rs:14-28` |
| Dual-status mapping | Lines 158-171 | Add narrative paragraph before table explaining the reading rule |

### PRD Success Criteria Mapping

| PRD Criterion | SPEC Coverage |
|---------------|---------------|
| **Must: All tg CLI commands covered** | Phase 1 tasks: Sections 2, 4, 5, 6 (21 entries covering 19 unique subcommands across 3 tiers) |
| **Must: Each command categorized** | Phase 1 tasks: Sections 4, 5, 6 with tier definitions |
| **Must: Caution/Dangerous explain bypassed logic** | Phase 1 tasks: Sections 5, 6 specify bypass details per command |
| **Must: Dangerous explain data integrity issues** | Phase 1 task: Section 6 specifies risk descriptions |
| **Must: Extension fields listed with details** | Phase 1 task: Section 7 (all fields from `src/pg_item.rs:14-28`) |
| **Should: Phase-golem equivalents suggested** | Phase 1 tasks: Sections 5, 6 include equivalents |
| **Should: Quick-reference summary table** | Phase 1 task: Section 2 |
| **Should: Dual-status mapping table** | Phase 1 task: Section 3 |
| **Should: `init --force` noted** | Phase 1 task: Section 6 (separate entry with callout) |
| **Nice: Corruption scenario examples** | Phase 1 tasks: Sections 5, 6 (inline with tier entries) |
| **Nice: `doctor --fix` behavior guidance** | Phase 1 tasks: Sections 6, 8 |
| **Nice: Recovery guidance** | Phase 1 task: Section 8 |

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?

## Assumptions

- **Autonomous execution:** No human available. Decisions made based on PRD, Design, and tech research.
- **Single phase appropriate:** The entire deliverable is one markdown file plus a one-line README edit. Splitting into multiple phases would create artificial boundaries with no verification benefit.
- **No Product Vision exists** for this project, so SPEC decisions are grounded in PRD requirements and Design specifications.
- **`docs/` directory creation is trivial:** Treated as a task within Phase 1 rather than a separate phase.
