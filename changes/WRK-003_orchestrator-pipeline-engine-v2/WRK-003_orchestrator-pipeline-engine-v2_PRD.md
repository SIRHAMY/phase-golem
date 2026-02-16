# Change: Orchestrator Pipeline Engine v2

**Status:** Proposed
**Created:** 2026-02-11
**Author:** Human + Claude

## Problem Statement

The orchestrator built in WRK-002 works but has three structural limitations that cap its usefulness:

1. **Hardcoded pipeline** — Every item walks the same 6-phase sequence (PRD → Research → Design → Spec → Build → Review) regardless of type. A blog post shouldn't need a PRD phase. A quick refactor shouldn't need a design phase. Different work types need different pipelines, and this can't be configured today without changing Rust code.

2. **Fully sequential execution** — The orchestrator processes one item at a time, from start to finish, before picking up the next. Non-destructive phases (research, spec writing, reviews) could safely overlap across items, but the architecture doesn't support it. This means the developer waits for item A's entire pipeline to complete before item B starts, even though B's research phase could have run while A was building.

3. **No safety checks at boundaries** — The orchestrator doesn't validate configuration on startup (missing skills, malformed configs surface as mid-run failures), doesn't track what commit a phase output was based on (specs can go stale while other items build), and doesn't prioritize completing in-progress work over starting new work (items fan out instead of draining toward completion).

These limitations compound: without configurable pipelines, parallelism has no `destructive` flag to gate serialization. Without staleness detection, parallelism is unsafe. Without preflight validation, configurable pipelines are fragile.

## User Stories / Personas

- **Solo Developer (primary)** — Runs the orchestrator to drive AI agents through structured workflows. Wants to queue multiple items, step away, and come back to completed work. Needs the orchestrator to maximize autonomous progress while maintaining safety at destructive boundaries (code changes). Currently limited to one-at-a-time sequential processing and a single workflow shape.

## Desired Outcome

When this change is complete:

1. The orchestrator reads pipeline definitions from configuration. Each pipeline defines an ordered list of phases, each with associated skill commands and a `destructive` flag. The current 6-phase coding workflow is the default pipeline, seeded from config rather than hardcoded in Rust.

2. On every startup, before touching any backlog items, the orchestrator validates all pipeline configs parse correctly, all referenced skills are reachable, all in-progress backlog items reference valid pipeline types and phases, and reports each error with the failing condition, config location, and suggested fix before proceeding.

3. The orchestrator advances items that are furthest in their pipeline before starting new items, draining work toward completion rather than fanning out.

4. Phase outputs track what git commit they were based on. Before starting a destructive phase, the orchestrator compares against current HEAD and takes the configured action (warn, block, or ignore).

5. Multiple items can progress through non-destructive phases concurrently. Only one destructive phase runs at a time across all items. WIP limits cap total items in progress and concurrent non-destructive phases.

## Pipeline Configuration Schema

Pipelines are defined in `orchestrate.toml` under a `[pipelines]` table. Each pipeline is a named entry with an ordered list of phases:

Pre-phases run during `Scoping` status (before the item is approved for work); main phases run during `InProgress` status (the main work pipeline).

```toml
[pipelines.feature]
pre_phases = [
    { name = "research", skills = ["research/scope"] },
]
phases = [
    { name = "prd",      skills = ["/changes:0-prd:create-prd"],               destructive = false },
    { name = "tech-research", skills = ["/changes:1-tech-research:tech-research"], destructive = false },
    { name = "design",   skills = ["/changes:2-design:design"],                 destructive = false },
    { name = "spec",     skills = ["/changes:3-spec:create-spec"],              destructive = false },
    { name = "build",    skills = ["/changes:4-build:implement-spec-autonomous"], destructive = true },
    { name = "review",   skills = ["/changes:5-review:change-review"],          destructive = false },
]

[pipelines.blog-post]
pre_phases = [
    { name = "research", skills = ["research/scope"] },
]
phases = [
    { name = "draft",    skills = ["writing/draft"],   destructive = false },
    { name = "edit",     skills = ["writing/edit"],     destructive = false },
    { name = "publish",  skills = ["writing/publish"],  destructive = false },
]
```

### Item Lifecycle

```
New → (triage: hardcoded) → Scoping → (pre_phases) → Ready → InProgress → (phases) → Done
                                                         ↑ auto-promote if guardrails pass
```

- **Triage** is hardcoded orchestrator logic, not part of any pipeline. It assigns `pipeline_type`, makes initial guesses at size/risk/impact, and transitions the item from `New` to `Scoping`.
- **`pre_phases`** run during `Scoping` status. These produce scoping artifacts (research summaries, refined descriptions), refine scores (size/risk/impact), and define work boundaries. When complete, the item auto-promotes to `Ready` if guardrails pass, or blocks for human review if they don't.
- **`phases`** run during `InProgress` status. This is the main work pipeline.

### Phase Fields

Each phase (in both `pre_phases` and `phases`) has:
- `name` — Unique identifier within the pipeline (used in backlog, logs, result files)
- `skills` — Array of skill commands invoked sequentially for this phase. The phase isn't complete until all skills finish.
- `destructive` — Whether this phase modifies the shared codebase (default: `false`). Destructive phases are serialized across all items. Only applies to `phases`, not `pre_phases` (pre-phases are never destructive).

Staleness config is per-phase and optional:
```toml
{ name = "build", skills = ["..."], destructive = true, staleness = "warn" }
```

Values: `"warn"` (log and continue), `"block"` (halt item, surface to human), `"ignore"` (default, no check).

## Success Criteria

### Must Have

- [ ] Pipeline definitions loaded from `orchestrate.toml` `[pipelines]` section (not hardcoded enum)
- [ ] Default `feature` pipeline matches current workflow (backwards compatible)
- [ ] Item status lifecycle simplified to: `New → Scoping → Ready → InProgress → Done` (+ `Blocked` from any non-terminal state)
- [ ] `BacklogItem.pipeline_type: String` field added; defaults to `"feature"` for existing items
- [ ] `BacklogItem.description: Option<String>` field added for the user's free-form description of the desired work (passed via `orchestrate add`)
- [ ] `BacklogItem.phase` becomes `String` (not `WorkflowPhase` enum) to support arbitrary phase names
- [ ] `BacklogItem.phase_pool` field added: `"pre"` or `"main"` to disambiguate which phase list the current phase belongs to
- [ ] `BACKLOG.yaml` bumped to `schema_version: 2`; v1 files auto-migrated on load with explicit mapping: `New→New`, `Researching→Scoping` (phase_pool: `"pre"`, phase: first pre_phase), `Scoped→Ready`, `Ready→Ready`, `InProgress→InProgress`, `Done→Done`, `Blocked→Blocked` (with `blocked_from_status` also mapped). Add `pipeline_type: "feature"` to all existing items. Migration uses atomic write-temp-rename pattern. V1 struct definitions preserved in a migration module for parsing.
- [ ] `PhaseResult.phase` also migrated from `WorkflowPhase` enum to `String`. Prompt builders updated to emit string phase names in agent output schema. Stale result files from in-progress v1 runs handled gracefully (parse failure triggers re-run, not crash).
- [ ] Each pipeline has `pre_phases` (run during `Scoping` status) and `phases` (run during `InProgress` status)
- [ ] Each phase has: name, skills array (one or more skill commands, run sequentially), and `destructive` flag (default: false; only applies to main `phases`)
- [ ] Triage is hardcoded orchestrator logic: assigns `pipeline_type`, initial scores, transitions `New → Scoping`
- [ ] After `pre_phases` complete: auto-promote `Scoping → Ready` if guardrails pass (size/risk/impact scores within configured thresholds and `requires_human_review` flag not set — inherited from WRK-002 `[guardrails]` config), else block for human review
- [ ] Pre-phase retry exhaustion blocks the item (same behavior as main phase retry exhaustion). Blocked `Scoping` items are visible in `orchestrate status`.
- [ ] Preflight validation on every `orchestrate run`: configs parse, skill probe agents confirm each referenced skill is visible and readable, in-progress items reference valid pipeline types and phase names. Structural validation rules: each pipeline must have at least one main phase, phase names unique within a pipeline (across both pre_phases and phases), `destructive` flag rejected on pre_phases, `max_wip` and `max_concurrent` must be ≥ 1. Any preflight error aborts the entire run.
- [ ] Preflight errors reported with actionable messages before any work starts (each error includes: the failing condition, the config file and key that caused it, and a suggested fix)
- [ ] Item selection uses "advance furthest first" across both `Scoping` and `InProgress` pools — items with higher phase index in their total journey (pre_phases + phases) get priority. Any `InProgress` item is further than any `Scoping` item.
- [ ] Tiebreaker for items at the same phase position: FIFO (creation date)
- [ ] Orchestrator records current `HEAD` commit SHA in `PhaseResult` as `based_on_commit` field before executing each phase. The SHA is also persisted on `BacklogItem.last_phase_commit: Option<String>` so it survives result file cleanup.
- [ ] Before destructive phases: compare `BacklogItem.last_phase_commit` against current HEAD. If `last_phase_commit` references a commit not in current history (e.g., after rebase), treat as stale (block regardless of config).
- [ ] Staleness-blocked items can be unblocked via `orchestrate unblock`, which resets `last_phase_commit` to current HEAD. The user accepts that prior phase artifacts may be stale.
- [ ] Per-phase staleness config: `staleness: warn | block | ignore` (default: `ignore`)
- [ ] Multiple items can execute non-destructive phases concurrently
- [ ] Only one destructive phase executes at a time across all items
- [ ] `max_wip` config: cap items in `InProgress` status only (default: `1` for sequential behavior). `Scoping` items are not counted against this limit.
- [ ] `max_concurrent` config: cap concurrent non-destructive phases (both scoping and non-destructive main phases) running at once (default: `1`). Destructive and non-destructive phases are mutually exclusive: when a destructive phase is running, no non-destructive phases run, and vice versa. `max_concurrent` is the natural throttle for scoping work.
- [ ] When `InProgress` and `Scoping` items compete for `max_concurrent` slots, `InProgress` items always take priority. Scoping items only run when no `InProgress` items need non-destructive phase slots.
- [ ] Agents do not commit during execution. The orchestrator owns all git operations. Non-destructive phase outputs are batched into a single mechanical commit (e.g., `[WRK-001][research][WRK-003][design] Phase outputs`). Destructive phases commit individually after completion. "Nothing to commit" after a phase is normal.
- [ ] All BACKLOG.yaml writes AND in-memory backlog state mutations serialized through a single coordinator (no concurrent file writes or data races on reads). The coordinator owns the only mutable reference to `BacklogFile`.
- [ ] All git operations serialized through the same coordinator (no concurrent index access)
- [ ] Active child process groups tracked in a global registry. On shutdown: signal handler sets atomic flag, async runtime polls flag and sends SIGTERM to all registered PGIDs, waits 5-second grace period, SIGKILLs survivors, exits after all children reaped.
- [ ] On restart after crash: orchestrator detects all items in `InProgress` or `Scoping` status and resumes them according to normal scheduling rules (advance-furthest-first, `max_concurrent` limits)
- [ ] All existing tests pass (or are updated) after migration
- [ ] New tests for: preflight validation, staleness checks, concurrent scheduling, process cleanup
- [ ] Existing BACKLOG.yaml files and changes/ directories work without manual intervention
- [ ] Missing `[pipelines]` section in `orchestrate.toml` auto-generates default `feature` pipeline. `orchestrate init` writes the full default `[pipelines.feature]` section to the TOML file so users can see and edit it.
- [ ] `orchestrate advance` validates phase names against the item's pipeline definition (not a hardcoded enum). Phase progression respects `phase_pool` boundaries.
- [ ] If any skill in a multi-skill phase fails, the entire phase is considered failed. Retries re-execute all skills in the phase from the beginning.
- [ ] `orchestrate add` accepts `--description <string>` for the user's free-form description plus optional metadata flags (`--pipeline`, `--size`, `--risk`, etc.). Description stored on the backlog item and available to the triage agent as context.
- [ ] Triage agents can update `pipeline_type` in their phase result (reclassify items). Post-triage `pipeline_type` validated against configured pipelines; invalid types block the item.
- [ ] All items go through triage — no skip-triage path. Human-supplied metadata (pipeline type, scores) are hints that triage can accept or override.

### Should Have

- [ ] At least one non-coding pipeline defined (e.g., `blog-post`) to validate the system handles different pipeline shapes
- [ ] `orchestrate status` shows pipeline type per item
- [ ] Clear log output showing scheduling decisions (which item selected, why), concurrent phase starts/completions, and staleness warnings

### Nice to Have

- [ ] Orphaned `_changes/` directory detection in preflight
- [ ] `orchestrate validate` command that runs preflight checks without starting work
- [ ] `orchestrate run --dry-run` shows what the scheduler would do without executing (useful for testing scheduler logic)

## Scope

### In Scope

- Pipeline configuration schema in `orchestrate.toml` and loading logic
- Migration of hardcoded `WorkflowPhase` enum to config-driven string-based phases
- `BACKLOG.yaml` schema v1 → v2 auto-migration
- Preflight validation system
- "Advance furthest first" scheduling algorithm
- Staleness detection (`based_on_commit` tracking and comparison at destructive boundaries)
- Cross-item parallelism for non-destructive phases
- Destructive phase serialization (one at a time across all items)
- WIP and concurrency limits
- Serialized BACKLOG.yaml and git coordinators for concurrent safety
- Process group registry for clean multi-spawn shutdown

### Out of Scope

- Pipeline branching/conditionals (e.g., "if small, skip research") — linear phase sequences only
- Within-phase parallelism (fanning out skills within a single phase)
- Cross-item dependency enforcement (informational only, not scheduled)
- Automatic staleness re-runs (just detection and gating, no auto-regeneration)
- Skill composition or inheritance
- Multi-repo orchestration
- Agent resource limits (token/cost budgets)
- Dynamic pipeline creation at runtime
- Per-item git branches (all items commit to same branch, serialized)
- File-path-aware destructive phase concurrency (all destructive phases serialize regardless of affected files)

## Non-Functional Requirements

- **Performance:** Preflight config parsing and structural validation completes in under 2 seconds for configs with up to 20 pipelines and 100 skill references. Skill probe agents (one per unique skill) run as a separate preflight step; their timeout is bounded by the agent timeout config, not the 2-second structural validation budget.
- **Backwards Compatibility:** Existing `orchestrate.toml` (missing `[pipelines]`) and `BACKLOG.yaml` (schema v1) auto-migrate without manual intervention. Config loaded once at startup; changes require restart.
- **Observability:** Scheduling decisions, concurrent phase starts/completions, staleness warnings, and preflight results logged to stderr with item IDs and phase names.

## Constraints

- Must remain a single Rust binary (no external services, no database)
- File-based state (BACKLOG.yaml, changes/, _worklog/) — no migration to a database
- Git operations must be serialized (concurrent `git add`/`git commit` corrupts the index)
- BACKLOG.yaml writes must be serialized (concurrent writes cause data loss)
- The `AgentRunner` trait's testability pattern (mock injection for testing) must be preserved; the exact signature will change to support async concurrent execution
- Must work with the existing Claude Code agent spawning model (subprocess per phase)
- Concurrency via tokio async runtime (decided — not threads or multi-process)
- Pipeline config loaded once at startup — runtime config changes require restart
- Blocked items do not count against `max_wip` (humans can unblock freely; orchestrator won't start new items until WIP drains naturally)
- `max_wip` applies to `InProgress` items only; `Scoping` items are not counted against `max_wip` but are subject to `max_concurrent`

## Dependencies

- **Depends On:** WRK-002 (orchestrator v1) — must be complete and stable
- **Blocks:** Future work on scheduled autonomous runs, cross-item awareness, complexity-based pipeline selection, file-path-aware destructive concurrency

## Risks

- [ ] **Concurrent BACKLOG.yaml writes → data loss** — Two phases completing simultaneously both load/mutate/write BACKLOG; last write wins. Mitigation: serialize all BACKLOG writes through a single coordinator (mutex or channel). All current `backlog::save()` call sites (~20) must route through the coordinator.
- [ ] **Concurrent git operations → repository corruption** — Interleaved `git add`/`git commit` corrupts the index. Mitigation: serialize all git operations through the same coordinator. Note: even non-destructive phases commit artifacts, so git serialization applies to all concurrent phases.
- [ ] **WorkflowPhase enum → string migration scope** — The enum is used in ~45 locations across 5 source files, but converting to string-based phases doubles the touch points (~90 changes including new string-to-phase-config lookups). Mitigation: introduce a phase lookup helper and migrate incrementally.
- [ ] **Process group cleanup with multiple spawns** — Current signal handler sets a global flag; with multiple concurrent subprocesses, need to track all Process Group IDs (PGIDs) and kill cleanly. Mitigation: maintain a global registry of active child PGIDs (`Arc<Mutex<HashSet<Pid>>>`). Signal handler remains a simple atomic flag setter (mutexes are not async-signal-safe). The async runtime polls the flag and iterates the registry to kill processes. Shutdown sends SIGTERM to all PGIDs first, waits 5 seconds, then SIGKILLs survivors.
- [ ] **Advance-furthest-first starvation** — New items don't start while WIP limit is saturated with in-progress items, even if some are blocked. This is Working As Intended (drains toward completion) but should be documented. Mitigation: `orchestrate unblock` frees WIP slots; blocked items don't count against WIP.
- [ ] **Destructive phase head-of-line blocking** — If item A's build takes 45 minutes, items B/C/D queue behind it even if they touch different files. This is a known v2 limitation. Mitigation: log waiting status clearly; file-path-aware concurrency is future work.
- [ ] **Staleness cascading blocks** — With `staleness: block` and `max_wip > 1`, one item's commits can block all other items whose artifacts are now stale. Mitigation: default all phases to `staleness: ignore`; document that `block` is only safe with `max_wip: 1`.
- [ ] **Interleaved git history with concurrent items** — With `max_wip > 1`, git history becomes `[A][prd] → [B][research] → [A][research]` rather than grouped by item. Reverting one item's work requires cherry-picking. Mitigation: document this tradeoff; recommend `max_wip: 1` for safety-critical work. Per-item branches are future work.
- [ ] **Tokio async migration scope (~40-50% of total effort)** — The current codebase is fully synchronous (~47 function signatures, `Result<T, String>` everywhere, 0 async functions). Migrating to tokio requires changing all function signatures to `async fn`, adding a tokio runtime in `main.rs`, migrating `MockAgentRunner` from `std::sync::Mutex` to `tokio::sync::Mutex`, subprocess spawning to `tokio::process::Command`, and rewriting all tests to use `#[tokio::test]`. Mitigation: SPEC phases should separate async conversion (Phase 1, preserving sequential behavior) from concurrency feature work (Phase 2+).

## Resolved Decisions

- **Concurrency model: tokio async.** The codebase will migrate to async. This is a large overhaul but tokio is standard, well-supported, and natural for subprocess coordination. The async contagion (the requirement that all callers of an async function must also be async, propagating through the call stack) is acceptable given the scope of this change.
- **Blocked items do NOT count against `max_wip`.** The WIP limit constrains what the orchestrator starts autonomously. Humans can unblock items freely; if unblocking temporarily exceeds the limit, the orchestrator simply won't start new items until WIP drains. This preserves "advance furthest first" — the orchestrator works through existing items before opening new ones.
- **Triage agents can reclassify `pipeline_type`.** Items may arrive as one-sentence descriptions; the triage agent should be able to recognize "this is a blog post, not a feature" and set the appropriate pipeline type.
- **Skill invocation: context preamble + skill command.** The orchestrator passes structured context (item metadata, previous phase summaries, failure context, unblock notes) as a preamble, then invokes the skill command. The skill handles domain logic; the orchestrator handles coordination context.
- **Skills must support autonomous execution.** Skills invoked by the orchestrator must run without human interaction (no interviews, no prompts for clarification). Domain logic should not diverge between autonomous and human-invocable versions. The specific architecture (core skills + wrappers, mode flags, etc.) is a design-phase decision.
- **Skill validation via agent probes.** Preflight does not check file paths. Instead, the orchestrator spins up a lightweight agent per unique skill reference, asking the agent to confirm it can see and read the skill. This validates the full chain (skill exists, agent can access it, agent can parse it). Probe agents return a structured pass/fail response.
- **Agents do not commit.** All git operations are owned by the orchestrator. Non-destructive phase outputs are batched into a single mechanical commit. Destructive phase results are committed individually. This eliminates concurrent git index corruption from agent subprocesses.
- **Destructive and non-destructive phases are mutually exclusive.** When a destructive phase is running, no non-destructive phases run (and vice versa). `max_concurrent` governs non-destructive concurrency only. This ensures the codebase is stable during destructive work.
- **`InProgress` items always take scheduling priority over `Scoping` items.** Scoping items only get `max_concurrent` slots when no `InProgress` items need non-destructive phase slots. This is consistent with "advance furthest first."
- **Pre-phase progress tracked via `phase` + `phase_pool` discriminator.** `BacklogItem.phase` holds the current phase name; `BacklogItem.phase_pool` (values: `"pre"` or `"main"`) disambiguates which phase list it belongs to.
- **Preflight failures are hard-fail.** Any preflight error (config parse, skill probe failure, invalid item references) aborts the entire `orchestrate run`. User must fix all issues before the orchestrator proceeds.
- **Tokio async migration sized as ~40-50% of effort.** SPEC phases should separate async conversion (preserving sequential behavior) from concurrency features.

## Open Questions

- [ ] **Skill architecture for autonomous vs human modes:** How to structure skills so domain logic is shared but execution mode differs (autonomous skips interviews/reviews, human mode includes them). Core + wrapper? Mode flag? Separate entrypoints? Defer to design phase.
- [ ] **Context preamble format:** The resolved decision says the orchestrator passes "structured context" as a preamble before skill invocation, but the delivery mechanism (file? stdin? CLI args? embedded in prompt?) is unspecified. Defer to design phase.
- [ ] **Concurrent Claude CLI instances:** With `max_concurrent > 1`, multiple `claude` processes run simultaneously. Are there shared state conflicts, lock files, or API rate limiting concerns? May need per-agent backoff or shared rate limiter. Defer to design/research phase.

## References

- WRK-002 orchestrator implementation (changes/002_overhaul-changes-workflow-orchestrator/)
- Architecture brainstorm document (provided in conversation — cross-item parallelism, destructive/non-destructive phases, WIP limits, preflight validation, staleness detection, configurable pipelines)
- CI/CD pipeline patterns (GitLab stages, CircleCI scheduling)
- Work-stealing scheduler literature (advance-furthest-first is the inverse — prioritize completion over breadth)
