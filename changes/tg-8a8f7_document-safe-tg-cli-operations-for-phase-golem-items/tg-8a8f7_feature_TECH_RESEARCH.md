# Tech Research: Document Safe tg CLI Operations for Phase-Golem Items

**ID:** tg-8a8f7
**Status:** Complete
**Created:** 2026-02-27
**PRD:** ./tg-8a8f7_document-safe-tg-cli-operations-for-phase-golem-items_PRD.md
**Mode:** Light

## Overview

Research to verify the complete `tg` CLI command set, the full `x-pg-*` extension field contract, and documentation patterns for classifying CLI command safety tiers. This informs the design and implementation of `docs/tg-cli-safety.md`, a documentation-only deliverable that categorizes `tg` commands by their safety when run against a phase-golem-managed `.task-golem/` store.

## Research Questions

- [x] What is the complete set of `tg` CLI commands and their read/write semantics?
- [x] What are all `x-pg-*` extension fields, their types, and consequences if missing?
- [x] How do other projects document CLI command safety tiers?
- [x] Are there any PRD assumptions that need correction based on codebase analysis?

---

## External Research

### Landscape Overview

Documenting CLI command safety tiers is an established practice that appears across multiple domains: HTTP method safety (RFC 9110), Kubernetes RBAC verb classification, Terraform plan/apply/destroy workflows, and AI agent safety tools like Destructive Command Guard (DCG). The common thread is a tiered classification system separating read-only operations from mutating ones, with further distinction between reversible mutations and destructive/irreversible ones.

For the specific case of a wrapper tool documenting safe operations against an underlying shared store (exactly the phase-golem/tg relationship), the closest analogs are the HTTP method safety classification and DCG's whitelist-first architecture.

### Common Patterns & Approaches

#### Pattern: HTTP Method Safety Classification (RFC 9110)

**How it works:** Formally defines two properties: *safe* (read-only) and *idempotent* (repeatable without different effects). Creates a 2x2 matrix (safe/unsafe × idempotent/non-idempotent).

**When to use:** When classification needs to be precise and based on observable side effects.

**Tradeoffs:**
- Pro: Universally understood, formally defined
- Pro: Two-axis approach provides nuance
- Con: Does not address the "bypass" dimension relevant to wrapper tools

**References:**
- [MDN: Safe HTTP Methods](https://developer.mozilla.org/en-US/docs/Glossary/Safe/HTTP)
- [HTTP Methods: Idempotency and Safety](https://www.mscharhag.com/api-design/http-idempotent-safe)

#### Pattern: DCG Whitelist-First Three-Tier Classification

**How it works:** Classifies commands into Safe (allowed), Destructive (blocked with alternative suggested), and Default Allow (unrecognized). Safe patterns checked first to prevent false negatives.

**When to use:** When multiple CLI tools interact with the same data and you need defense-in-depth.

**Tradeoffs:**
- Pro: "Explain why blocked + suggest alternative" maps to PRD's requirement for phase-golem equivalents
- Pro: Whitelist-first ordering prevents false negatives on safe operations
- Con: "Default allow" tier not appropriate for documentation (we want explicit classification)

**References:**
- [DCG GitHub Repository](https://github.com/Dicklesworthstone/destructive_command_guard)
- [DCG Safety Philosophy](https://reading.torqsoftware.com/notes/software/ai-ml/safety/2026-01-26-dcg-destructive-command-guard-safety-philosophy-design-principles/)

### Standards & Best Practices

1. **Define "safe" relative to a specific invariant.** The PRD correctly defines it as "performs no writes to `.task-golem/` store files."
2. **Classify by observable effect, not intent.**
3. **Safe-first ordering in documentation.** List safe operations first, then escalate to caution and dangerous.
4. **Suggest alternatives for every dangerous operation.** DCG and Terraform both follow this pattern.
5. **Include a quick-reference summary table.** Every well-documented pattern includes one.
6. **Version-stamp the document.** Note which version of `tg` and `src/pg_item.rs` the document was verified against.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Conflating "safe for tg" with "safe for phase-golem" | `tg do` is safe for task-golem but bypasses phase management | Make "safe for phase-golem" explicit in the document title and introduction |
| Incomplete command enumeration | New commands/flags get added over time | Reference `tg` CLI source as source of truth; include version marker |
| Stale documentation | Command classifications change as tools evolve | Reference source files; include "last verified against" marker |
| Ambiguous tier boundaries | Subjective criteria lead to misclassification | Use precise, testable definitions (PRD already does this well) |

---

## Internal Research

### Existing Codebase State

Phase-golem uses task-golem's `.task-golem/` directory (tasks.jsonl, archive.jsonl, tasks.lock) as its persistence layer. The `PgItem` newtype in `src/pg_item.rs` wraps task-golem's `Item` and provides typed access to 15 `x-pg-*` extension fields that encode phase management state. No `docs/` directory exists yet.

**Relevant files:**
- `src/pg_item.rs` (lines 12-28) — Extension field constant definitions
- `src/pg_item.rs` (lines 79-114) — Bidirectional status mapping (`pg_status()`)
- `src/pg_item.rs` (lines 567-618) — Item creation with extension defaults (`new_from_parts`)
- `src/pg_item.rs` (lines 470-565) — Central mutation dispatch (`apply_update`)
- `src/types.rs` (lines 5-44) — `ItemStatus` enum and transition validation
- `../task-golem/src/cli/args.rs` — Command definitions (clap)
- `../task-golem/src/cli/commands/` — Individual command implementations

### Complete tg CLI Command Set (19 commands)

**Safe (Read-Only) — 6 commands:**

| Command | What It Does | Reads | Writes |
|---------|--------------|-------|--------|
| `list` | List active items, filter by status/tag | tasks.jsonl | — |
| `show` | Display single item by ID (active or archive) | tasks.jsonl, archive.jsonl | — |
| `dump` | Export all items as JSON/YAML | tasks.jsonl, archive.jsonl | — |
| `next` | Show highest-priority ready item | tasks.jsonl | — |
| `ready` | List items with satisfied dependencies | tasks.jsonl | — |
| `completions` | Generate shell completion scripts | — | stdout only |

**Caution (Status Transitions) — 5 commands:**

| Command | What It Does | Bypasses |
|---------|--------------|----------|
| `do` | Todo → Doing; optionally claims item | 6-state mapping, phase assignment, review gates |
| `done` | Todo/Doing → Done; archives item | Phase completion checks, commit tracking |
| `todo` | Doing → Todo; reverts to todo | Phase state; can revert mid-phase |
| `block` | Any → Blocked; sets optional reason | `x-pg-blocked-type`, `x-pg-blocked-from-status` |
| `unblock` | Blocked → previous; clears reason | `x-pg-blocked-from-status` (6-state restore) |

**Dangerous (Mutations) — 8 commands:**

| Command | What It Does | Risk |
|---------|--------------|------|
| `add` | Create new item | Missing `x-pg-*` extensions; invisible to phase pipeline |
| `edit` | Modify item fields/extensions | Can corrupt enum extensions; overwrite phase state |
| `rm` | Delete item permanently | Destroys metadata; orphans change folder artifacts |
| `init` | Initialize new store | Safe on new dirs; see `init --force` |
| `init --force` | Reinitialize existing store | Destroys ALL items and extension fields |
| `doctor --fix` | Repair store structural issues | Unaware of `x-pg-*` semantics; can invalidate phase state |
| `dep add` | Add dependency to item | Modifies item without phase awareness |
| `dep rm` | Remove dependency from item | Modifies item without phase awareness |
| `archive` | Prune archive entries by date | Removes archived items (may contain useful extension metadata) |

**Note:** `doctor` (without `--fix`) is read-only and belongs in the Safe tier.

### x-pg-* Extension Fields (15 fields)

All constants from `src/pg_item.rs:14-28`:

| Field | Type | Purpose | If Missing/Corrupted |
|-------|------|---------|---------------------|
| `x-pg-status` | string enum: "new", "scoping", "ready" | Maps Todo to 6-state system | Defaults to New; cannot distinguish New from unrecognized items |
| `x-pg-phase` | string | Current phase name (e.g., "prd", "build") | Cannot determine current phase or which to run next |
| `x-pg-phase-pool` | string enum: "pre", "main" | Pool organizing phases into pre/main groups | Skips pool-specific guardrails |
| `x-pg-size` | string enum: "small", "medium", "large" | Effort size assessment | Triage may re-assess; guardrails misfire |
| `x-pg-complexity` | string enum: "low", "medium", "high" | Technical complexity assessment | Human review gates may be bypassed |
| `x-pg-risk` | string enum: "low", "medium", "high" | Risk level for breaking changes | Risk guardrails cannot be enforced |
| `x-pg-impact` | string enum: "low", "medium", "high" | User-facing impact level | Prioritization may be incorrect |
| `x-pg-requires-human-review` | boolean | Flag for human decision gate | Items can auto-promote incorrectly |
| `x-pg-pipeline-type` | string | Pipeline name (e.g., "feature", "bugfix") | Executor cannot find correct phase sequence |
| `x-pg-origin` | string | How item was created (e.g., "backlog_inbox") | Provenance/audit trail lost |
| `x-pg-blocked-type` | string enum: "clarification", "decision" | Type of blocker | Cannot distinguish blocker reason |
| `x-pg-blocked-from-status` | string enum (6-state) | Saves status before blocking | Unblock restores to wrong status (defaults to New) |
| `x-pg-unblock-context` | string | Notes from unblock decision | No record of what was decided |
| `x-pg-last-phase-commit` | string (git SHA) | Last completed phase's commit | Staleness detection fails |
| `x-pg-description` | JSON object (StructuredDescription) | Structured problem/context | Phases run without context |

### Dual-Status Mapping

From `src/pg_item.rs:79-114`:

| Phase-Golem Status | Task-Golem Status | x-pg-status | Notes |
|--------------------|-------------------|-------------|-------|
| New | Todo | "new" | Initial state; awaiting triage |
| Scoping | Todo | "scoping" | Pre-phases running |
| Ready | Todo | "ready" | Ready for main phases |
| InProgress | Doing | *(absent/ignored)* | Main phases executing |
| Done | Done | *(absent/ignored)* | Terminal state; archived |
| Blocked | Blocked | *(absent/ignored)* | Awaiting decision/clarification |

**Reading rule:** Native status is authoritative for Doing/Done/Blocked. For Todo, `x-pg-status` disambiguates; absent defaults to New.

### Constraints

1. Extension field names and valid values are defined in `src/pg_item.rs:14-28` (source of truth).
2. Enum-typed extensions are case-sensitive and strict; invalid values cause warnings and fallback to defaults.
3. Native `blocked_from_status` is lossy (New/Scoping/Ready all map to Todo); `x-pg-blocked-from-status` preserves full 6-state fidelity.
4. `x-pg-status` is only meaningful when native status is Todo; it is cleared/ignored for Doing/Done/Blocked.
5. Change folder artifacts (`changes/`) are outside the store and not auto-managed.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Lists 20 commands to categorize | Actual count is 19 distinct operations (doctor and doctor --fix are one command with a flag) | Minor: adjust count but keep them as separate entries per PRD requirement |
| `dep add` / `dep rm` not explicitly pre-categorized | These modify the item's dependency list (a write operation) | Should be categorized as Dangerous (mutations to item data) |
| `archive` mentioned in command list | Archive pruning modifies archive.jsonl by removing entries | Should be categorized as Dangerous (deletes data) |
| Document says "no code changes" | Confirmed: purely documentation | No concern |

---

## Critical Areas

### "Safe for whom?" framing

**Why it's critical:** The most common pitfall in wrapper-tool documentation is conflating "safe for the underlying tool" with "safe for the wrapper." A `tg do` is valid task-golem usage but bypasses phase-golem logic.

**Why it's easy to miss:** Readers familiar with `tg` may assume commands that work correctly are "safe."

**What to watch for:** The document introduction must clearly state that "safe" means "safe with respect to phase-golem's extended state."

### dep add / dep rm categorization

**Why it's critical:** These modify item data (the `dependencies` list) without phase-golem awareness. While they don't touch extensions directly, they change the item's `updated_at` timestamp and could affect dependency-based ready checks.

**Why it's easy to miss:** Dependency management feels like a minor operation but it modifies items in the store.

**What to watch for:** Document these as Dangerous tier per the PRD's rule: "Dangerous if it creates new items, modifies item fields/extensions, or deletes items."

---

## Deep Dives

*No deep dives needed for light-mode research. The command set and extension fields are well-documented in source.*

---

## Synthesis

### Open Questions

| Question | Why It Matters | Resolution |
|----------|----------------|------------|
| Should `dep add`/`dep rm` be Caution or Dangerous? | They modify item fields but not extensions | Dangerous per PRD rule: modifies item fields |
| Should `archive` be Caution or Dangerous? | It deletes data from archive.jsonl | Dangerous per PRD rule: deletes items |
| Where should the doc link from? | Discoverability requirement | Add a note in README.md pointing to `docs/tg-cli-safety.md` |

### Recommended Approaches

#### Document Structure

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Tier-grouped (Safe → Caution → Dangerous) | Reads as escalation ladder; safe-first gives confidence | Commands not alphabetical | Default; matches DCG pattern and PRD structure |
| Alphabetical with tier badges | Easy to look up specific commands | Loses the safety narrative flow | Reference-only use case |

**Initial recommendation:** Tier-grouped structure. Start with quick-reference summary table, then detail each tier. This matches the PRD's structure and the DCG safe-first documentation pattern.

#### Extension Field Documentation

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Inline table in safety doc | Self-contained; no external references needed | Duplicates source of truth; can go stale | Documentation is the primary concern |
| Reference to `src/pg_item.rs` only | Always current; no duplication | Reader must read Rust source | Developer audience comfortable with code |
| Table in doc + "see `src/pg_item.rs` for authoritative definitions" | Best of both: readable table + source reference | Minor duplication | Mixed audience (operators + developers) |

**Initial recommendation:** Table in doc with source reference. Matches PRD's explicit requirement to "list each field with name, type, purpose, consequence" while noting `src/pg_item.rs` as source of truth.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [MDN: Safe HTTP Methods](https://developer.mozilla.org/en-US/docs/Glossary/Safe/HTTP) | Standard | Canonical definition of operation safety |
| [DCG GitHub](https://github.com/Dicklesworthstone/destructive_command_guard) | Tool | Three-tier classification with whitelist-first pattern |
| `src/pg_item.rs` | Source | Extension field definitions and status mapping |
| `../task-golem/src/cli/args.rs` | Source | Complete tg CLI command definitions |
| `../task-golem/src/cli/commands/` | Source | Command implementations (read vs write behavior) |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-27 | Initial external + internal research (light mode, parallel agents) | Complete command inventory (19 commands), all 15 extension fields documented, tier classification verified against codebase, documentation patterns surveyed |

## Assumptions

- **Autonomous execution:** No human available for Q&A. Resolved open questions (dep add/rm and archive categorization) using the PRD's explicit categorization rules.
- **dep add/dep rm → Dangerous:** These modify item fields (dependencies list), which satisfies the PRD's Dangerous rule ("modifies item fields/extensions").
- **archive → Dangerous:** This deletes data from archive.jsonl, which satisfies the Dangerous rule ("deletes items").
- **doctor without --fix → Safe:** Read-only diagnostic; no writes to store files.
