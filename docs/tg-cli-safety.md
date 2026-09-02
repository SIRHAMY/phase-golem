# tg CLI Safety Guide for Phase-Golem Stores

Phase-golem uses task-golem's `.task-golem/` store directly. Task-golem owns item identity, status, claims, dependencies, readiness, and archive behavior. Phase-golem adds `x-pg-*` metadata for execution policy and progress, but it does not define a second lifecycle.

This guide classifies direct `tg` operations by whether they preserve both TG data integrity and phase-golem execution policy.

## Quick Reference

| Command | Tier | Guidance |
|---------|------|----------|
| `tg list`, `tg show`, `tg dump` | Safe | Read-only inspection |
| `tg next`, `tg ready` | Safe | Uses the same TG readiness phase-golem consumes |
| `tg doctor` | Safe | Read-only diagnostics |
| `tg add` | Safe | Creates a non-PG task; phase-golem ignores it until ownership metadata is attached |
| `tg do` | Caution | Explicitly claims work; do not use on a planned human gate |
| `tg done` | Caution | Explicitly completes and archives work without running remaining PG phases |
| `tg todo` | Caution | Returns work to Todo and may abandon PG phase progress |
| `tg block`, `tg unblock` | Caution | Native transitions are valid, but direct unblock does not add PG operator context |
| `tg dep add`, `tg dep rm` | Caution | Immediately changes TG readiness |
| `tg edit` | Dangerous | Can corrupt PG ownership, gate, phase, or assessment metadata |
| `tg rm` | Dangerous | Permanently deletes the item and its metadata |
| `tg init --force` | Dangerous | Reinitializes the store and destroys task data |
| `tg doctor --fix` | Dangerous | Mutates storage; review each proposed repair first |
| `tg archive --before <date>` | Dangerous | Removes entries from the primary archive |

## Native Lifecycle

The only task statuses are `todo`, `doing`, `blocked`, and `done`.

- `todo`: unclaimed work. TG dependencies determine whether it is ready.
- `doing`: claimed or explicitly started work.
- `blocked`: attempted-but-unable work with a diagnostic reason.
- `done`: explicitly completed work, normally stored in the archive.

Phase-golem reads TG's dependency evaluation and only claims PG-owned Todo tasks that TG reports ready. A planned human gate is a PG-owned Todo task with `x-pg-human-decision = true`; phase-golem reports it as a stop condition without claiming it. Completing a child never changes its parent automatically.

## Read-Only Commands

`tg list`, `tg show`, `tg dump`, `tg next`, `tg ready`, and read-only `tg doctor` are safe. In particular, `tg ready` and phase-golem use the same TG dependency semantics, including missing-dependency and archived-dependency handling.

## Status Commands

Direct TG status transitions are structurally valid because TG owns lifecycle state. They can still bypass phase-golem policy:

- `tg do` claims a task without phase-golem's human-gate check. Never claim a planned human gate.
- `tg done` explicitly completes and archives a task even if configured PG phases remain.
- `tg todo` abandons the current claim while PG phase metadata may still describe unfinished work.
- `tg block` records native blocking diagnostics. Prefer phase-golem when its typed blocker metadata is useful.
- `tg unblock` restores TG's recorded prior status. Prefer `phase-golem unblock --notes "..."` when operator context should be recorded in `x-pg-unblock-context`.

These commands do not roll status up to parents, containers, or roots.

## Creation And Dependencies

`tg add` creates a normal TG task with a canonical UUIDv7. It has no `x-pg-owner`, so phase-golem leaves it alone. Use a phase-golem creation path when the task should enter PG execution.

TG dependencies are authoritative. `tg dep add` and `tg dep rm` are valid but can make work ready or unready immediately. Stop phase-golem before changing dependencies when a run is active.

## Metadata Edits

Avoid direct edits to `x-pg-*` fields. Important fields include:

| Field | Purpose |
|-------|---------|
| `x-pg-owner` | Opts the task into phase-golem execution |
| `x-pg-human-decision` | Marks a planned human gate |
| `x-pg-template-node-key` | Identifies a materialized workflow node |
| `x-pg-phase`, `x-pg-phase-pool` | Tracks configured execution progress |
| `x-pg-pipeline-type` | Selects the configured pipeline |
| `x-pg-size`, `x-pg-complexity`, `x-pg-risk`, `x-pg-impact` | Stores assessments used by PG policy |
| `x-pg-blocked-type`, `x-pg-unblock-context` | Stores PG-specific blocker context |
| `x-pg-last-phase-commit` | Supports destructive-phase staleness checks |
| `x-pg-origin`, `x-pg-description` | Stores provenance and structured context |

The current field list and typed accessors live in `src/pg_item.rs`.

## Destructive Maintenance

- `tg rm` permanently deletes an item. It does not remove related files under `changes/`.
- `tg init --force` destroys and recreates the store.
- `tg doctor --fix` changes store data. Inspect the reported issue and back up `.task-golem/` first.
- `tg archive --before <date>` moves old archive entries out of the primary archive, which can change dependency history available to normal reads.

The `.task-golem/` files are normally tracked by git. Before destructive maintenance, commit or otherwise back up the store so active and archived data can be recovered.
