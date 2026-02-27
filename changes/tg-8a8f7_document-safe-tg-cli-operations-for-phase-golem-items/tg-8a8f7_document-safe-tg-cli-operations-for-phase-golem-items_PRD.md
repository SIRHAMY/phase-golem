# Change: Document Safe tg CLI Operations for Phase-Golem Items

**Status:** Proposed
**Created:** 2026-02-27
**Author:** phase-golem (autonomous)

## Problem Statement

Phase-golem recently migrated its storage layer to task-golem (WRK-076), making `.task-golem/` the shared persistence layer. The `tg` CLI can operate directly on this store, but phase-golem decorates items with `x-pg-*` extension fields (custom metadata properties stored in each item's `extensions` map) that the `tg` CLI is unaware of. These extensions encode phase management state — the set of validations, state mappings, workflow gates, and metadata updates that phase-golem performs when transitioning items through its pipeline — that phase-golem relies on for correct operation.

Key terms used in this document:
- **Extension fields (`x-pg-*`):** Custom key-value metadata that phase-golem adds to task-golem items to track phase state, assessments, and workflow context. Defined in `src/pg_item.rs`.
- **Phase management logic:** The validations, 6-state status mapping, phase/pool assignment, human review gates, and commit tracking that phase-golem performs on every state transition.
- **Dual-status system:** task-golem uses 4 native statuses (todo, doing, done, blocked); phase-golem maps these to 6 statuses (new, scoping, ready, in_progress, done, blocked) using the `x-pg-status` extension field.
- **Change folder:** A directory under `changes/` containing PRDs, specs, and other artifacts associated with a phase-golem work item.

Without documentation classifying which `tg` CLI operations are safe to run against a phase-golem-managed store, operators risk:
- Creating items via `tg add` that lack required `x-pg-*` extensions, producing items that phase-golem cannot recognize as managed items or that cause parsing errors when phase-golem reads their status
- Removing items via `tg rm` that destroys phase-golem metadata and orphans associated change folder artifacts (PRDs, specs, design docs stored under `changes/`)
- Performing status transitions via `tg do`/`tg done`/`tg block` that bypass phase management logic, skipping phase/pool assignment, human review checks, and commit tracking
- Editing items via `tg edit --set` that overwrites extension fields phase-golem depends on, or setting invalid values for enum-typed extensions

This is a documentation-only change. No code modifications are required.

## User Stories / Personas

- **Phase-golem operator** - A developer or CI agent who uses both `tg` and phase-golem CLIs against the same store. They need clear guidance on which `tg` commands are safe to run directly and which should only be performed through phase-golem to avoid creating items without phase metadata, corrupting status state, or orphaning change artifacts.

- **New contributor** - Someone onboarding to a project using phase-golem who sees the `.task-golem/` directory and naturally reaches for `tg` commands. They need to understand the relationship between `tg` and phase-golem before accidentally breaking phase management state.

## Desired Outcome

A documentation file exists (e.g., `docs/tg-cli-safety.md`) that categorizes every `tg` CLI command into one of three tiers:

1. **Safe** (read-only) — Commands that only read from the `.task-golem/` store and cannot modify items or metadata. Information obtained from safe commands could be misused in downstream actions; operators remain responsible for what they do with the output.
2. **Caution** (status transitions) — Commands that modify item status in the `tg` native model but bypass phase-golem's phase management logic (6-state mapping, phase/pool assignment, human review gates, commit tracking). The item's native status changes, but `x-pg-*` extensions are not updated.
3. **Dangerous** (mutations) — Commands that create, edit, or delete items. These can produce items without required `x-pg-*` extensions, overwrite extension field values, or permanently remove items and their associated metadata.

**Categorization rules:**
- A command is **Safe** if it performs no writes to the `.task-golem/` store files.
- A command is **Caution** if it writes to the store but only changes item status (not item content, extensions, or existence).
- A command is **Dangerous** if it creates new items, modifies item fields/extensions, or deletes items.
- `tg doctor --fix` is **Dangerous** because it modifies the store to repair structural issues without awareness of `x-pg-*` extension semantics.

For each command entry: include the command name, what it does, why it's in that tier, and (for Caution/Dangerous commands) the phase-golem equivalent operation if one exists, or an explicit warning to avoid the operation.

## Success Criteria

### Must Have

- [ ] Documentation file covers all `tg` CLI commands: `list`, `show`, `ready`, `next`, `dump`, `doctor`, `doctor --fix`, `completions`, `init`, `add`, `edit`, `rm`, `do`, `done`, `todo`, `block`, `unblock`, `dep add`, `dep rm`, `archive`
- [ ] Each command is categorized into Safe, Caution, or Dangerous tier using the categorization rules above
- [ ] Caution and Dangerous commands explain which specific phase-golem logic is bypassed (e.g., "bypasses 6-state status mapping", "skips phase assignment")
- [ ] Dangerous commands explain what data integrity issues can result (e.g., "creates item without x-pg-status, invisible to phase-golem pipeline")
- [ ] Document lists each `x-pg-*` extension field with: field name, data type, purpose, and consequence if missing or corrupted. Reference `src/pg_item.rs` as the source of truth for the current field set.

### Should Have

- [ ] Each Caution/Dangerous command suggests the phase-golem equivalent operation (if one exists) or states "no equivalent — avoid this operation on phase-golem stores"
- [ ] Document includes a quick-reference summary table with columns: Command | Tier | Description | Recommendation
- [ ] Document includes a mapping table showing how task-golem's 4 native statuses (todo, doing, done, blocked) map to phase-golem's 6 statuses (new, scoping, ready, in_progress, done, blocked) via the `x-pg-status` extension
- [ ] Document notes that `tg init --force` on an existing phase-golem store will reinitialize the store files and destroy all items including their extension fields

### Nice to Have

- [ ] Examples of specific corruption scenarios (e.g., "running `tg done` on an item in `scoping` status will archive the item without completing phase work")
- [ ] Guidance on `tg doctor --fix` behavior: what it can repair (JSONL syntax, orphaned done items) and what it is unaware of (x-pg-* extension validity, phase-golem state consistency)
- [ ] Recovery guidance for common mistakes: what to do after accidentally running a Dangerous command (e.g., check git history for `.task-golem/tasks.jsonl` to recover deleted items)

## Scope

### In Scope

- A single documentation file categorizing `tg` CLI command safety
- Reference to `x-pg-*` extension fields with names, types, and purposes
- Explanation of the dual-status mapping between `tg` and phase-golem (mapping table)
- Quick-reference summary table
- Categorization of `tg doctor --fix` as a separate entry from `tg doctor` (read-only)

### Out of Scope

- Code changes to phase-golem or task-golem
- Adding safety guards or warnings to the `tg` CLI itself
- Automated linting or validation of the `.task-golem/` store for phase-golem compliance
- Documentation of phase-golem's own CLI commands
- Extending `tg doctor` to detect phase-golem extension issues
- Detailed state transition diagrams or truth tables (reference `src/pg_item.rs` for these)
- Guidance on compound/scripted operations (e.g., piping `tg list` into bulk edits)

## Non-Functional Requirements

- **Discoverability:** The document should be findable from the project README or `docs/` directory
- **Maintainability:** The document should reference source files (`src/pg_item.rs`, task-golem CLI source) as sources of truth so readers can verify accuracy against current code. The document is a snapshot; when the extension field contract or `tg` CLI commands change, it should be updated.

## Constraints

- Must accurately reflect the current `tg` CLI command set (as of the task-golem version used by phase-golem's local dependency in `Cargo.toml`)
- Must accurately reflect the current `x-pg-*` extension fields as defined in `src/pg_item.rs` (reference the file rather than hardcoding a count)
- Documentation only — no code changes permitted in this change
- The `docs/` directory may need to be created if it does not already exist

## Dependencies

- **Depends On:** WRK-076 (storage migration to task-golem) — already completed
- **Blocks:** Nothing directly, but enables safer operator workflows

## Risks

- [ ] Extension field contract may change as phase-golem evolves, requiring documentation updates. Mitigation: reference `src/pg_item.rs` as the source of truth so readers can verify against current code. Update the safety doc when extension fields are added or removed.

## Open Questions

None — the scope and content are well-defined by the existing codebase.

## Assumptions

- **Mode: light** — The idea is fully formed in the item description. No discovery or deep exploration needed.
- **File location:** `docs/tg-cli-safety.md` is the assumed path. The `docs/` directory will be created if it doesn't exist. If a different convention is established, the path can be adjusted during implementation.
- **No Product Vision exists** for this project, so the PRD was written without vision alignment context.
- **Autonomous execution:** No human was available for interview. Decisions were made based on codebase analysis. Key autonomous decisions: (1) chose "light" mode given the well-defined scope, (2) defined categorization rules based on read vs. write semantics, (3) included `tg doctor --fix` and `tg init --force` as separately categorized entries.

## References

- Extension field definitions: `src/pg_item.rs` (lines 14-28)
- PgItem wrapper and status mapping: `src/pg_item.rs` (lines 30-280)
- Task-golem CLI source: `../task-golem/src/cli/`
- WRK-076: Storage migration to task-golem (completed)
