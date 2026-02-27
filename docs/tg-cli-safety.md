# tg CLI Safety Guide for Phase-Golem Stores

Phase-golem uses task-golem's `.task-golem/` directory as its persistence layer but extends each item with `x-pg-*` extension fields that encode phase management state (status mapping, phase tracking, assessments, review gates, and more). The `tg` CLI operates on the same store but is unaware of these extensions. Running certain `tg` commands directly can bypass phase-golem's management logic, corrupt extension fields, or create items that phase-golem cannot process.

This guide classifies every `tg` CLI command by its safety **relative to phase-golem's extended state** — not task-golem's native operation. A command that works correctly from task-golem's perspective may still be unsafe for a phase-golem-managed store.

If you encounter a `tg` command or flag not listed here, consult `tg --help` or the task-golem CLI source. Report discrepancies as a documentation issue.

**Last verified against:** 2026-02-27, commit `ce8aad1`, task-golem 0.1.0, `src/pg_item.rs:14-28`

---

## Quick-Reference Summary Table

| Command | Tier | Description | Recommendation |
|---------|------|-------------|----------------|
| `tg list` | Safe | List active items, filter by status/tag | OK to use |
| `tg show` | Safe | Display a single item by ID | OK to use |
| `tg dump` | Safe | Export all items as JSON/YAML | OK to use |
| `tg next` | Safe | Show highest-priority ready item | OK to use |
| `tg ready` | Safe | List items with satisfied dependencies | OK to use |
| `tg doctor` | Safe | Run read-only store diagnostics | OK to use (see warning below) |
| `tg completions` | Safe | Generate shell completion scripts | OK to use |
| `tg do` | Caution | Transition item to Doing | Use phase-golem pipeline instead |
| `tg done` | Caution | Transition item to Done and archive | Use phase-golem pipeline instead |
| `tg todo` | Caution | Revert item to Todo | Use phase-golem pipeline instead |
| `tg block` | Caution | Block an item with optional reason | Use `phase-golem block` or pipeline instead |
| `tg unblock` | Caution | Unblock an item | Use `phase-golem unblock` instead |
| `tg add` | Dangerous | Create a new item | Use `phase-golem add` instead |
| `tg edit` | Dangerous | Modify item fields or extensions | Avoid; use phase-golem operations |
| `tg rm` | Dangerous | Delete an item permanently | Avoid; no phase-golem equivalent |
| `tg init` | Dangerous | Initialize a new store | Safe only on fresh directories (see details) |
| `tg init --force` | Dangerous | Reinitialize an existing store | Avoid; destroys all items and extensions |
| `tg doctor --fix` | Dangerous | Repair store structural issues | Avoid; unaware of `x-pg-*` semantics |
| `tg dep add` | Dangerous | Add a dependency to an item | Avoid; modifies item without phase awareness |
| `tg dep rm` | Dangerous | Remove a dependency from an item | Avoid; modifies item without phase awareness |
| `tg archive` | Dangerous | Archive maintenance; prune with `--before` | Avoid `--before`; moves items to `archive-pruned.jsonl` |

---

## Dual-Status Mapping

Phase-golem uses a 6-state status model while task-golem uses 4 native statuses. The `x-pg-status` extension field bridges this gap: when an item's task-golem status is `todo`, phase-golem reads `x-pg-status` to determine whether the item is `new` (not yet triaged), `scoping` (pre-phases running), or `ready` (eligible for main phases). For `doing`, `done`, and `blocked`, the native status is authoritative and `x-pg-status` is cleared.

This mapping is why Caution-tier commands are risky: they change the native status without updating `x-pg-status`, which can silently collapse three distinct phase-golem states into one.

| Phase-Golem Status | Task-Golem Status | x-pg-status Value | Notes |
|--------------------|-------------------|-------------------|-------|
| New | Todo | `"new"` | Initial state; awaiting triage. Absent `x-pg-status` also defaults to New. |
| Scoping | Todo | `"scoping"` | Pre-phases (research, discovery) are running |
| Ready | Todo | `"ready"` | Pre-phases complete; eligible for main phase promotion |
| InProgress | Doing | *(cleared)* | Main phases executing; native status is authoritative |
| Done | Done | *(cleared)* | Terminal state; item is archived |
| Blocked | Blocked | *(cleared)* | Awaiting human decision or clarification |

**Source:** `src/pg_item.rs:79-114`

---

## Safe Tier (Read-Only Commands)

These commands only read from the store and cannot modify items or metadata. They are always safe to run against a phase-golem-managed store.

> **Note:** While these commands cannot corrupt data, operators remain responsible for how they use the information obtained. For example, using `tg list` output to script bulk `tg edit` operations would bypass phase-golem's management logic.

### `tg list`

Lists active items with optional filtering by status, tag, or other criteria. Reads `tasks.jsonl` only. Phase-golem's extension fields are stored in each item but are not modified.

### `tg show`

Displays a single item by ID, including its extensions. Reads from `tasks.jsonl` and `archive.jsonl`. Useful for inspecting `x-pg-*` extension field values without risk.

### `tg dump`

Exports all items (active and archived) as JSON or YAML. Reads from `tasks.jsonl` and `archive.jsonl`. Useful for backups or external analysis.

### `tg next`

Shows the highest-priority ready item. Reads `tasks.jsonl` only. Uses task-golem's native priority and dependency logic, which does not account for phase-golem's 6-state model.

### `tg ready`

Lists items with all dependencies satisfied. Reads `tasks.jsonl` only. Dependency resolution is handled at the task-golem level and is unaffected by extension fields.

### `tg doctor`

Runs read-only diagnostics on the store structure (JSONL syntax, duplicate IDs, orphaned references). Does not modify any files.

> **Warning:** If `tg doctor` reports issues, do **NOT** run `tg doctor --fix` without reading the [Dangerous tier section](#tg-doctor---fix) first. The `--fix` flag modifies the store without awareness of phase-golem extensions and can invalidate phase state.

### `tg completions`

Generates shell completion scripts for bash, zsh, fish, etc. Writes only to stdout. Does not interact with the store at all.

---

## Caution Tier (Status Transition Commands)

These commands modify item status in the task-golem native model but bypass phase-golem's phase management logic. The native status changes, but `x-pg-*` extension fields are **not** updated. This can leave items in an inconsistent state where the native status and extension fields disagree.

### `tg do`

Transitions an item from Todo to Doing, optionally claiming it.

**What is bypassed:**
- 6-state status mapping: collapses New/Scoping/Ready distinction into a single Doing state without updating `x-pg-status`
- Phase assignment: does not set `x-pg-phase` or `x-pg-phase-pool`
- Human review gates: does not check `x-pg-requires-human-review`
- Guardrail enforcement: does not validate size/complexity/risk thresholds

**Phase-golem equivalent:** Let the `phase-golem run` pipeline promote items to InProgress when they are Ready. Use `phase-golem advance` to manually push a specific item.

### `tg done`

Transitions an item from Todo or Doing to Done and archives it.

**What is bypassed:**
- Phase completion checks: does not verify that all pipeline phases have completed
- Commit tracking: does not set `x-pg-last-phase-commit`
- Phase cleanup: does not clear `x-pg-phase` or `x-pg-phase-pool`
- Worklog archiving: does not create worklog entries

**Phase-golem equivalent:** Let the pipeline complete all phases naturally. Running `tg done` on a Scoping item will archive it without completing any pre-phase or main-phase work.

### `tg todo`

Reverts an item from Doing to Todo.

**What is bypassed:**
- Phase state: `x-pg-phase`, `x-pg-phase-pool`, and `x-pg-status` are not restored; the item may appear as New (default) instead of its actual pre-transition state (Scoping or Ready)
- Mid-phase work: any in-progress phase execution is not cleaned up

**Phase-golem equivalent:** No direct equivalent. If you need to revert an InProgress item, consider blocking it with a reason instead.

### `tg block`

Blocks an item with an optional reason string.

**What is bypassed:**
- `x-pg-blocked-type`: not set (phase-golem distinguishes "clarification" vs "decision" blockers)
- `x-pg-blocked-from-status`: not set (phase-golem saves the full 6-state status for accurate restoration on unblock; task-golem's native `blocked_from_status` is lossy — New, Scoping, and Ready all collapse to Todo)

**Phase-golem equivalent:** Use phase-golem's blocking mechanism, which preserves the full 6-state `blocked_from_status` and sets the blocker type.

### `tg unblock`

Unblocks an item, restoring it to its previous status.

**What is bypassed:**
- `x-pg-blocked-from-status`: task-golem restores the native `blocked_from_status` (lossy 4-state), not phase-golem's full 6-state value. A Scoping item that was blocked will be restored to Todo, appearing as New.
- `x-pg-unblock-context`: not set (phase-golem records the unblock decision context)

**Phase-golem equivalent:** Use `phase-golem unblock --notes "..."`, which reads `x-pg-blocked-from-status` to restore the correct 6-state status and records the unblock context.

---

## Dangerous Tier (Mutation Commands)

These commands create, modify, or delete items. They can produce items without required extensions, overwrite phase state, or permanently destroy metadata.

### `tg add`

Creates a new item in the store.

**Risk:** The new item will have no `x-pg-*` extension fields. Phase-golem requires at minimum `x-pg-status` to distinguish New/Scoping/Ready states, and uses `x-pg-pipeline-type` to determine the phase sequence. An item without extensions is invisible to the phase pipeline and may cause unexpected behavior when phase-golem encounters it.

**Phase-golem equivalent:** Use `phase-golem add`, which creates items with all required extension defaults.

### `tg edit`

Modifies item fields, including extensions.

**Risk:** Can overwrite any `x-pg-*` extension field. Enum-typed extensions (`x-pg-status`, `x-pg-size`, `x-pg-risk`, etc.) are case-sensitive and strict — invalid values trigger warnings and fallback to defaults, which silently changes the item's effective state. Editing `x-pg-phase` or `x-pg-phase-pool` can cause phase-golem to skip or repeat phases.

**Phase-golem equivalent:** Avoid direct edits. Use phase-golem operations (`advance`, `block`, `unblock`, `triage`) to manage item state through validated transitions.

### `tg rm`

Permanently deletes an item from the store.

**Risk:** Destroys the item and all its `x-pg-*` extension metadata. Additionally, the associated change folder under `changes/` (containing PRDs, specs, design docs, and other artifacts) is **not** automatically cleaned up — it becomes an orphan that must be manually deleted.

**Phase-golem equivalent:** No equivalent — phase-golem does not support item deletion. Avoid this operation on phase-golem stores. If an item must be removed, ensure you also clean up the corresponding `changes/<item-id>_*/` directory.

### `tg init`

Initializes a new `.task-golem/` store directory.

**Risk:** Safe when creating a new store on a fresh directory that has no existing `.task-golem/` data. Categorized as Dangerous because it writes store files (`tasks.jsonl`, `archive.jsonl`, `tasks.lock`). On a directory that already has a store, `init` without `--force` will refuse to overwrite.

**Phase-golem equivalent:** Use `phase-golem init`, which creates both the `.task-golem/` store and phase-golem's configuration files.

### `tg init --force`

Reinitializes an existing store, overwriting all store files.

**Risk:** **Destroys ALL items and their extension fields.** This is the most destructive `tg` command. All phase-golem state — every `x-pg-*` extension, every item's phase progress, every assessment — is permanently lost. Change folders under `changes/` survive but become orphaned.

**Phase-golem equivalent:** No equivalent. Avoid this operation unless you intend to completely reset the store. Consider backing up `.task-golem/` first, or rely on git history to recover.

### `tg doctor --fix`

Repairs store structural issues detected by `tg doctor`.

**Risk:** `doctor --fix` can repair JSONL syntax errors, remove orphaned done items, and fix structural inconsistencies, but it is completely unaware of `x-pg-*` extension semantics. Repairs may invalidate phase-golem state — for example, removing a "done" item that still has meaningful extension metadata, or rewriting item entries in a way that strips or alters extension fields.

**Phase-golem equivalent:** No equivalent. If `tg doctor` reports issues, evaluate the specific problems before deciding whether `--fix` is appropriate. For phase-golem state issues, check git history for `.task-golem/tasks.jsonl` and restore manually.

### `tg dep add`

Adds a dependency to an item.

**Risk:** Modifies the item's dependency list and updates its `updated_at` timestamp without phase-golem awareness. While this does not directly touch `x-pg-*` extensions, it changes item data that affects dependency-based ready checks and can cause phase-golem's scheduling to behave unexpectedly.

**Phase-golem equivalent:** No direct equivalent. If dependencies must be adjusted, prefer doing so before phase-golem begins processing the item, or stop phase-golem first.

### `tg dep rm`

Removes a dependency from an item.

**Risk:** Same as `dep add` — modifies item data without phase-golem awareness. Removing a dependency may cause an item to become "ready" prematurely from task-golem's perspective, but phase-golem's own scheduling uses additional criteria.

**Phase-golem equivalent:** No direct equivalent. Same guidance as `dep add`.

### `tg archive`

Without flags, scans the active store for done items that were not yet archived and moves them to `archive.jsonl` (a recovery/maintenance operation). With `--before <date>`, prunes archive entries older than the specified date, moving them from `archive.jsonl` to `archive-pruned.jsonl`.

**Risk:** The recovery mode (no flags) writes to `archive.jsonl` without phase-golem awareness but does not destroy data. The prune mode (`--before`) removes items from the primary archive — while they are preserved in `archive-pruned.jsonl`, they are no longer visible to `tg show` or `tg dump`. Archived items may contain valuable `x-pg-*` extension metadata (final assessments, phase history, commit references) useful for auditing.

**Phase-golem equivalent:** No equivalent. The recovery mode is relatively low-risk. Avoid `--before` pruning if you need to preserve phase-golem metadata on archived items in the primary archive.

---

## Extension Field Reference

Phase-golem stores its management state in `x-pg-*` extension fields on each item. These are the fields that `tg` commands are unaware of and that Caution/Dangerous operations can leave inconsistent.

**Source of truth:** `src/pg_item.rs:14-28` — always verify against this file for the current field set.

> **Enum case-sensitivity:** Enum-typed extensions (e.g., `x-pg-status`, `x-pg-size`, `x-pg-risk`) are case-sensitive and strict. Invalid values cause warnings and fallback to defaults, which may change the item's effective state without obvious errors.

> **Verification hint:** To check whether this table is current, count extension field constants in `src/pg_item.rs` (search for `X_PG_`) and compare to the 15 entries below.

| Field | Type | Purpose | If Missing/Corrupted |
|-------|------|---------|---------------------|
| `x-pg-status` | string enum: `"new"`, `"scoping"`, `"ready"` | Disambiguates Todo sub-states in the 6-state model | Defaults to New; cannot distinguish New from Scoping or Ready |
| `x-pg-phase` | string | Current phase name (e.g., `"prd"`, `"build"`) | Cannot determine current phase or which to run next |
| `x-pg-phase-pool` | string enum: `"pre"`, `"main"` | Groups phases into pre/main pools | Skips pool-specific guardrails |
| `x-pg-size` | string enum: `"small"`, `"medium"`, `"large"` | Effort size assessment | Triage may re-assess; guardrails may misfire |
| `x-pg-complexity` | string enum: `"low"`, `"medium"`, `"high"` | Technical complexity assessment | Human review gates may be bypassed |
| `x-pg-risk` | string enum: `"low"`, `"medium"`, `"high"` | Risk level for breaking changes | Risk guardrails cannot be enforced |
| `x-pg-impact` | string enum: `"low"`, `"medium"`, `"high"` | User-facing impact level | Prioritization may be incorrect |
| `x-pg-requires-human-review` | boolean | Flags item for human decision gate | Items can auto-promote incorrectly |
| `x-pg-pipeline-type` | string | Pipeline name (e.g., `"feature"`, `"bugfix"`) | Executor cannot find correct phase sequence |
| `x-pg-origin` | string | How item was created (e.g., `"backlog_inbox"`) | Provenance/audit trail lost |
| `x-pg-blocked-type` | string enum: `"clarification"`, `"decision"` | Type of blocker | Cannot distinguish blocker reason |
| `x-pg-blocked-from-status` | string enum: `"new"`, `"scoping"`, `"ready"`, `"in_progress"`, `"done"`, `"blocked"` | Saves 6-state status before blocking | Unblock restores to wrong status (defaults to New) |
| `x-pg-unblock-context` | string | Notes from unblock decision | No record of what was decided |
| `x-pg-last-phase-commit` | string (git SHA) | Last completed phase's commit | Staleness detection fails |
| `x-pg-description` | JSON object (`StructuredDescription`) | Structured problem/context metadata | Phases run without context |

---

## Recovery Guidance

### Recovering Deleted or Corrupted Items

If you accidentally ran `tg rm`, `tg init --force`, or another destructive command, the `.task-golem/tasks.jsonl` file is tracked by git. Check the commit history to find the last good state:

1. Run `git log --oneline -- .task-golem/tasks.jsonl` to find recent commits
2. Run `git show <commit>:.task-golem/tasks.jsonl` to view the file at that point
3. Restore with `git checkout <commit> -- .task-golem/tasks.jsonl`

If `archive.jsonl` was also affected, apply the same process for `.task-golem/archive.jsonl`.

### What `tg doctor --fix` Can and Cannot Repair

**Can repair:**
- JSONL syntax errors (malformed lines)
- Orphaned done items that should be in the archive
- Structural inconsistencies in the store format

**Cannot repair:**
- Missing or invalid `x-pg-*` extension fields
- Incorrect `x-pg-status` values that disagree with the native status
- Stale `x-pg-blocked-from-status` after a `tg unblock`
- Missing phase-golem extension defaults on items created by `tg add`

For phase-golem state issues, manual repair via git history (see above) or re-running `phase-golem triage` on affected items is the recommended approach.

### Inconsistent Status After `tg` Status Commands

If you ran `tg do`, `tg done`, `tg todo`, `tg block`, or `tg unblock` and the item's phase-golem state is now inconsistent (e.g., native status is `doing` but `x-pg-phase` is empty):

1. Use `tg show <id>` to inspect the current state including extensions
2. Restore `.task-golem/tasks.jsonl` from git history to the last known good state
3. Alternatively, if the item has not progressed far, use `tg edit` to manually correct the extension fields — but be aware this carries its own risks (see the [Dangerous tier](#tg-edit))
