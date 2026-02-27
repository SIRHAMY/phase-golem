# Design: Document Safe tg CLI Operations for Phase-Golem Items

**ID:** tg-8a8f7
**Status:** Complete
**Created:** 2026-02-27
**PRD:** ./tg-8a8f7_document-safe-tg-cli-operations-for-phase-golem-items_PRD.md
**Tech Research:** ./tg-8a8f7_feature_TECH_RESEARCH.md
**Mode:** Light

## Overview

A single documentation file (`docs/tg-cli-safety.md`) that classifies every `tg` CLI command into Safe, Caution, or Dangerous tiers based on their effect on phase-golem's extended state. The document uses a tier-grouped structure (safe commands first, escalating to dangerous) with a quick-reference summary table, an extension field reference, and a dual-status mapping table. This approach was selected because it mirrors established safety documentation patterns (HTTP method safety, DCG three-tier classification) and directly satisfies all PRD requirements with minimal complexity.

---

## System Design

### High-Level Architecture

This is a documentation-only change. There are no software components, APIs, or runtime behaviors. The deliverable is a single markdown file.

```
docs/
└── tg-cli-safety.md    ← New file (primary deliverable)
README.md               ← Minimal update: add one line linking to docs/tg-cli-safety.md
```

The `docs/` directory will be created if it does not exist. The README update is a single line to satisfy the PRD's discoverability requirement.

### Document Structure

The document is organized into these sections, in order:

#### 1. Header & Introduction

**Purpose:** Frame the document's scope and define what "safe" means.

**Content:**
- Title: "tg CLI Safety Guide for Phase-Golem Stores"
- One-paragraph introduction explaining the phase-golem / tg relationship
- Explicit statement: safety is defined relative to phase-golem's extended state, not task-golem's native operation
- "Last verified against" marker with specific format: date, git commit SHA of this repo, task-golem version (from `Cargo.toml`), and `src/pg_item.rs` line reference. Example: "Last verified: 2026-02-27, commit abc1234, task-golem 0.3.x, src/pg_item.rs:14-28"

#### 2. Quick-Reference Summary Table

**Purpose:** At-a-glance lookup for any command.

**Content:**
- Columns: Command | Tier | Description | Recommendation
- All 19 commands listed
- Tier indicated by label (Safe / Caution / Dangerous)
- Recommendation column: "OK to use" for Safe; "Use phase-golem equivalent" or "Avoid" for others

#### 3. Dual-Status Mapping Table

**Purpose:** Explain how tg's 4 native statuses map to phase-golem's 6 statuses. This context is essential for understanding why Caution-tier commands are risky — they change the native status without updating the `x-pg-status` extension that disambiguates phase-golem's finer-grained states.

**Content:**
- Narrative paragraph before the table explaining the reading rule in plain language with a concrete example (e.g., "If an item's task-golem status is 'todo', look at `x-pg-status`: 'new' means not yet triaged, 'scoping' means pre-phases are running, 'ready' means it can begin main phases")
- Table with columns: Phase-Golem Status | Task-Golem Status | x-pg-status Value | Notes
- 6 rows (New, Scoping, Ready, InProgress, Done, Blocked)
- Explanation of the reading rule: native status is authoritative for Doing/Done/Blocked; for Todo, `x-pg-status` disambiguates; absent `x-pg-status` defaults to New

#### 4. Safe Tier (Read-Only Commands)

**Purpose:** Document commands that are always safe to run.

**Content:**
- Brief tier definition: "These commands only read from the store and cannot modify items or metadata."
- 7 commands: `list`, `show`, `dump`, `next`, `ready`, `doctor` (without `--fix`), `completions`
- Each entry: command name, what it does, why it's safe
- Note: information obtained could be misused in downstream actions; operators remain responsible
- Warning callout on `doctor`: "If `tg doctor` reports issues, do NOT run `tg doctor --fix` without reading the Dangerous tier section first — `--fix` modifies the store without awareness of phase-golem extensions"

#### 5. Caution Tier (Status Transition Commands)

**Purpose:** Document commands that change item status but bypass phase management logic.

**Content:**
- Brief tier definition: "These commands modify item status in the tg native model but bypass phase-golem's phase management logic."
- 5 commands: `do`, `done`, `todo`, `block`, `unblock`
- Each entry: command name, what it does, which phase-golem logic is bypassed (specific list), phase-golem equivalent if one exists
- Specific bypass details per command (e.g., `done` bypasses phase completion checks and commit tracking)

#### 6. Dangerous Tier (Mutation Commands)

**Purpose:** Document commands that create, modify, or delete items.

**Content:**
- Brief tier definition: "These commands create, modify, or delete items. They can produce items without required extensions, overwrite phase state, or permanently destroy metadata."
- 9 entries covering 7 subcommands (with `init`/`init --force` as separate entries due to different risk profiles, plus `doctor --fix` which is a flag variant of the Safe-tier `doctor`): `add`, `edit`, `rm`, `init`, `init --force`, `doctor --fix`, `dep add`, `dep rm`, `archive`
- Each entry: command name, what it does, specific data integrity risk, phase-golem equivalent or explicit "avoid" warning
- `init --force` gets special callout: reinitializes the store and destroys all items including their extension fields and all phase management state
- `rm` entry explicitly notes: deleting an item orphans any associated change folder under `changes/` (PRDs, specs, design docs); these must be manually cleaned up

#### 7. Extension Field Reference

**Purpose:** Document all `x-pg-*` fields so readers understand what's at stake.

**Content:**
- Table with columns: Field | Type | Purpose | If Missing/Corrupted
- All 15 extension fields (as of document creation) from `src/pg_item.rs:14-28`
- Source of truth reference: "See `src/pg_item.rs` for authoritative definitions"
- Note on enum-typed fields: "Enum-typed extensions (e.g., `x-pg-status`, `x-pg-size`, `x-pg-risk`) are case-sensitive and strict. Invalid values trigger warnings and fallback to defaults, which may cause unexpected phase-golem behavior without obvious errors."
- Verification hint for operators: "To check freshness, count extension field constants in `src/pg_item.rs` (search for `X_PG_`) and compare to this table"

#### 8. Recovery Guidance (Nice to Have)

**Purpose:** Help operators recover from common mistakes.

**Content:**
- Brief section with 2-3 recovery scenarios
- Primary guidance: check git history for `.task-golem/tasks.jsonl` to recover deleted/corrupted items
- Guidance on what `tg doctor --fix` can and cannot repair

### Data Flow

Not applicable — documentation-only change.

### Key Flows

#### Flow: Operator Looks Up Command Safety

> An operator wants to know if a specific `tg` command is safe to run against a phase-golem store.

1. **Open document** — Operator navigates to `docs/tg-cli-safety.md`
2. **Check quick-reference table** — Find the command in the summary table; see tier and recommendation
3. **Read tier details (if needed)** — Navigate to the tier section for bypass details and phase-golem equivalents
4. **Check extension fields (if needed)** — Consult the extension field table to understand what data is at risk

**Edge cases:**
- Command not in the table — Document includes a note: "This guide covers tg CLI commands as of the version listed in the header. If you encounter a command not listed here, consult `tg --help` or the task-golem CLI source. Report discrepancies as a documentation issue."

#### Flow: New Contributor Onboarding

> A new contributor discovers the `.task-golem/` directory and wants to understand the tg / phase-golem relationship.

1. **Discover document** — Find link from project README or `docs/` directory listing
2. **Read introduction** — Understand that phase-golem extends tg with `x-pg-*` fields
3. **Read dual-status mapping** — Understand the 4-status → 6-status mapping
4. **Scan quick-reference table** — Get an overview of what's safe vs. what requires phase-golem

---

## Technical Decisions

### Key Decisions

#### Decision: Tier-Grouped Document Structure

**Context:** The document could be organized alphabetically (easy lookup) or by tier (safety narrative).

**Decision:** Tier-grouped, with Safe first, then Caution, then Dangerous.

**Rationale:** Matches the DCG safe-first pattern recommended by tech research. Reads as an escalation ladder that communicates the safety model, not just individual command classifications. The quick-reference summary table at the top provides the alphabetical lookup for operators who already know what command they want.

**Consequences:** Commands are not alphabetically sorted within the main body, but the summary table covers that need.

#### Decision: Extension Fields Documented Inline + Source Reference

**Context:** Extension fields could be documented only inline, only by reference, or with both.

**Decision:** Include a full table in the document with a reference to `src/pg_item.rs` as the source of truth.

**Rationale:** Satisfies the PRD requirement to "list each field with name, type, purpose, consequence" while maintaining a path to the authoritative definitions. Operators who don't read Rust can still understand the fields; developers can verify against source.

**Consequences:** The inline table may drift from source over time. The source reference mitigates this, and the "last verified against" marker signals when the document needs updating.

#### Decision: File Location at `docs/tg-cli-safety.md`

**Context:** The document could live at the project root, in `docs/`, or as a README section.

**Decision:** `docs/tg-cli-safety.md` in a new `docs/` directory.

**Rationale:** Standard location for project documentation. Keeps the root clean. A standalone file is easier to reference, link to, and maintain than a section embedded in README.

**Consequences:** The `docs/` directory must be created. A minimal link will be added to the project README for discoverability (single line, in scope for this change).

#### Decision: `doctor` (Read-Only) vs. `doctor --fix` (Dangerous) as Separate Entries

**Context:** `tg doctor` is one command with an optional `--fix` flag, but the two modes have fundamentally different safety profiles.

**Decision:** List them as separate entries: `doctor` in Safe tier, `doctor --fix` in Dangerous tier.

**Rationale:** PRD explicitly requires this separation. The safety difference is significant enough to warrant distinct entries even though they share a command name.

**Consequences:** The command count in the document (19 entries) differs from the clap subcommand count because one subcommand appears in two tiers.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Staleness risk | Inline extension field table may drift from source | Self-contained document that doesn't require reading Rust source | Source reference + version marker provides a verification path; doc-only change means updates are cheap |
| Not exhaustive on recovery | Recovery section covers common cases, not all possible corruption | Keeping the document focused and not overwhelming | Git history of `tasks.jsonl` covers the worst cases; detailed recovery is out of scope per PRD |

---

## Alternatives Considered

### Alternative: Alphabetical Command Reference with Tier Badges

**Summary:** List all commands alphabetically, each tagged with a tier badge (Safe/Caution/Dangerous).

**How it would work:**
- Single flat list of all 19 commands, A-Z
- Each entry has a badge/tag indicating its tier
- Same detail level per entry

**Pros:**
- Easy to look up any specific command by name
- Familiar reference manual format

**Cons:**
- Loses the safety narrative — reader doesn't get the escalation model
- Tier definitions are separated from their commands
- Harder to scan for "what's dangerous?"

**Why not chosen:** The tier-grouped approach communicates the safety model more effectively, and the quick-reference summary table provides the alphabetical lookup need. For a safety-focused document, narrative structure matters more than alphabetical convenience.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Extension fields change without doc update | Document gives stale advice on field consequences | Medium | Reference `src/pg_item.rs` as source of truth; include "last verified against" marker |
| New tg CLI commands added without doc update | New commands not classified | Low | Reference tg CLI source; version marker signals staleness |
| Readers treat "Safe" commands as universally safe | Misunderstanding could lead to using safe command output in unsafe ways | Low | Introduction explicitly states safety is relative to phase-golem state; safe commands note about downstream responsibility |

---

## Integration Points

### Existing Code Touchpoints

- `README.md` — Add a single line linking to `docs/tg-cli-safety.md` for discoverability. This is a minimal addition (one line in an existing section or a new "Documentation" section if none exists).

### External Dependencies

- `src/pg_item.rs` — Source of truth for extension field definitions and status mapping. Referenced but not modified.
- `../task-golem/src/cli/args.rs` — Source of truth for tg CLI command set. Referenced but not modified.

---

## Open Questions

None — the scope, structure, and content are fully defined by the PRD and tech research.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements (all must-have, should-have, and nice-to-have items are covered by document sections)
- [x] Key flows are documented and make sense (operator lookup, new contributor onboarding)
- [x] Tradeoffs are explicitly documented and acceptable (staleness, recovery scope)
- [x] Integration points with existing code are identified (none — doc only)
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-27 | Initial design draft | Tier-grouped document structure with 8 sections; one alternative considered |
| 2026-02-27 | Self-critique (7 agents) + auto-fixes | Fixed: dangerous tier count, init --force wording, added enum case-sensitivity note, orphaned change folder risk, doctor warning callout, dual-status narrative context, "last verified" marker format, unknown command guidance. Directional: brought README link into scope. |

## Assumptions

- **Autonomous execution:** No human available. Decisions made based on PRD requirements, tech research findings, and codebase analysis.
- **Light mode appropriate:** Documentation-only change with well-defined scope requires minimal design iteration.
- **README link is in scope:** A minimal one-line link to `docs/tg-cli-safety.md` will be added to the project README to satisfy the PRD's discoverability requirement. This is a documentation-only addition consistent with the change scope.
- **No Product Vision exists** for this project, so design decisions are grounded in PRD requirements and tech research patterns.
