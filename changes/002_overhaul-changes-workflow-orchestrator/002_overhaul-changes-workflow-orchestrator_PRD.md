# Change: Overhaul Changes Workflow with Orchestrator

**Status:** Proposed
**Created:** 2026-02-10
**Author:** Human + Claude

## Problem Statement

The current changes workflow (a skill-based pipeline that takes work items from PRD through research, design, SPEC (Implementation Specification), build, and review — see [Current Changes Workflow](../../.claude/skills/changes/SKILL.md)) handles individual changes well but has no system for managing work across changes. Follow-ups discovered during builds dead-end in SPEC followup sections. There's no structured way to capture new work items, triage them, or let an AI agent autonomously work through a queue of tasks between human check-ins.

The workflow also lacks a way to scale effort to task size. A one-line bug fix goes through the same heavyweight process as a multi-phase feature. Small items should complete in fewer phase executions with thin artifacts; large items should earn detail progressively.

The result: human time is spent on mechanical coordination (what to work on next, creating change folders, triaging follow-ups) instead of on judgment calls (architectural decisions, requirements clarification, risk assessment).

## User Stories / Personas

- **Solo Developer** — Uses the changes workflow on personal or small-team projects. Wants to queue up ideas, step away, and come back to find small/medium items completed and large items researched with clear decision points waiting. Checks in periodically to review work, make decisions, and point the system at the next thing.

- **Changes Workflow Adopter** — Someone copying the changes workflow into a new project. Needs the system to be self-contained and easy to set up: copy the skills folder, run an init command, start working.

## Desired Outcome

A three-component system that maximizes AI agent utilization between human check-ins:

1. **Backlog** — A structured YAML work queue where items live at various stages of readiness. Items are promoted through a pre-workflow lifecycle (captured → researched → scoped → ready) managed by triage and research agents, then through the changes workflow pipeline (PRD → research → design → spec → build → review) when in progress. Each step adds detail only when warranted. Items carry impact, size, complexity, and risk ratings that are progressively refined as more information is gathered through each phase.

2. **Orchestrator** — A Rust binary that runs the loop: picks the highest-impact ready item from the backlog, feeds it through the changes workflow pipeline (starting at the PRD phase), collects follow-ups, triages them back into the backlog, and repeats. Owns all deterministic state transitions. Agents own all judgment calls. Commits after every successful phase (including non-build phases) to create git checkpoints throughout the pipeline.

3. **Enhanced Changes Workflow** — The existing workflow extended to support autonomous execution via prompt wrapping, a structured return protocol for agent-to-orchestrator communication, and integration points for the backlog and orchestrator. Interactive skills remain unchanged; the orchestrator wraps them with autonomous instructions at invocation time.

When the human is present, they direct the orchestrator at specific items and make decisions on blocked work. When the human is away, the orchestrator runs the same pipeline autonomously — picking ready items that meet configurable guardrails (size, risk thresholds) and processing them through the full workflow.

Same pipeline either way. The "autonomous" part isn't a separate mode — it's the loop running without a human directing it.

## Success Criteria

### Must Have

#### Backlog System
- [ ] YAML-based work queue (`BACKLOG.yaml`) with items tracking id, title, size, complexity, risk, impact, status, origin (source item/phase that created it), requires_human_review flag, and lifecycle metadata
- [ ] Backlog lifecycle: pre-workflow statuses (`new → researching → scoped → ready`) managed by triage/research agents. When an item transitions to `in_progress`, it enters the changes workflow pipeline starting at the PRD phase and progresses through all phases (PRD → research → design → spec → build → review). Light items get thin artifacts; every item traverses all phases. `done` means the final phase completed. Any status can transition to `blocked`
- [ ] Jira-style short IDs: auto-assigned, project-configurable prefix (e.g., `WRK-001`), used in folder names, file names, commits, and cross-references. Orchestrator enforces uniqueness.
- [ ] Folder/file naming: `changes/WRK-001_descriptive-name/WRK-001_descriptive-name_PRD.md` format (clean break from old `NNN_` naming convention)
- [ ] Idea files: lightweight markdown docs (`_ideas/`) for items needing research/scoping before entering the changes workflow. Idea is seed, PRD is full requirements — every item gets a PRD, but light items have thin PRDs seeded from the idea file
- [ ] Impact rating on backlog items for prioritization. Orchestrator picks highest-impact ready item, not just oldest
- [ ] Auto-archiving: done items are pruned from BACKLOG.yaml first, then written to the work log (this order ensures a crash between operations leaves the item in the backlog rather than re-processing a completed item)
- [ ] Source of truth policy: YAML tracks lifecycle state (which item, which phase), SPEC tracks implementation progress within a phase (which tasks done). SPECs must include a machine-parseable phase status line (e.g., `**Phase Status:** complete | in_progress | not_started`) per phase section for orchestrator reconciliation, rather than inferring status from checkbox counts

#### Orchestrator
- [ ] Rust binary: state machine managing backlog lifecycle transitions and changes workflow phase transitions
- [ ] Spawns fresh Claude processes per phase (like current `implement-spec-autonomous.sh`) to avoid context window exhaustion that degrades agent quality
- [ ] Invokes existing changes workflow skills via `claude -p`, wrapping with autonomous mode instructions and structured output format
- [ ] Phase execution cap: `--cap N` limits total phase executions per autonomous run (safety valve against runaway loops). Each agent spawn counts toward the cap, including retries — a phase that fails twice and succeeds on the third attempt counts as 3 toward the cap
- [ ] Retry logic: configurable max retries per phase (default 2, so 3 total attempts), fresh agent on each retry with previous failure context. Malformed/unparseable agent output counts as `FAILED`
- [ ] Consecutive failure circuit breaker: if 2 consecutive items exhaust retries, stop the loop (likely systemic issue). Counter resets on any successful phase completion
- [ ] Follow-up triage between phases: phases report follow-ups in structured output, orchestrator batches them into backlog between phase transitions. Follow-up creation is always allowed (no caps on what gets created)
- [ ] Git commits after every successful phase (including non-build phases like PRD, research, design, spec) using format `[WRK-001][PHASE] Description`. Git is the undo/checkpoint system. Note: this produces high commit volume (~6 commits per item minimum); the `[WRK-001][PHASE]` format enables easy filtering with `git log --grep`
- [ ] Process group management: Claude subprocess runs in orchestrator's process group, killed on graceful shutdown (SIGTERM, SIGINT)
- [ ] Lock file: prevents running two orchestrator instances simultaneously. On startup, check for existing lock — if the PID recorded in the lock is no longer alive, remove the stale lock with a warning and proceed; if the PID is alive, refuse to start
- [ ] Git preconditions: on startup, verify clean working tree and valid branch (not detached HEAD, no rebase/merge in progress). If `git commit` fails during a phase, treat as phase failure — halt the item, log the error, alert the user. Do not continue without the checkpoint
- [ ] Per-phase timeout: configurable maximum wall-clock time per phase (default 30 minutes, `--phase-timeout`). On timeout, kill the subprocess and count as FAILED (triggers normal retry logic)
- [ ] Crash recovery: on restart, re-run the current phase from scratch. Build phases resume via SPEC checkboxes; non-build phases are effectively idempotent

#### Autonomous Guardrails & Assessment
- [ ] Configurable guardrails: project-level config for per-dimension thresholds (default: `max_size: medium`, `max_complexity: medium`, `max_risk: low`). Guardrails check each dimension independently — an item must satisfy ALL thresholds to proceed autonomously
- [ ] Progressive assessment refinement: each phase can update the item's size/complexity/risk/impact ratings as more information is gathered. Orchestrator re-evaluates guardrails at each phase transition. If an item exceeds any guardrail after refinement, it blocks for human review. Completed phase artifacts are preserved; after human approval the orchestrator resumes from the blocked phase
- [ ] Size/complexity/risk assessment uses "highest dimension wins" rule for the human-readable overall rating: if any dimension is medium, overall is medium; if any is high, overall is high. This overall rating is for display/reporting; guardrail checks use individual dimension values

#### Assessment Guidelines

| Dimension | Low | Medium | High |
|---|---|---|---|
| Size | 1-3 files, one phase does real work | 4-10 files, multiple phases | 11+ files, cross-cutting |
| Complexity | Single well-understood pattern applied in one place | Multiple approaches to evaluate, some design decisions | New architecture or cross-cutting design implications |
| Risk | No shared interfaces touched, easily reversible | Modifies shared code/APIs | Breaking changes, data migration |
| Impact | Nice-to-have, minor improvement | Meaningful user/developer improvement | Critical path, blocking issue, or high-value |

Overall = max(size, complexity, risk). Impact is used for prioritization, not for the overall rating.

#### Structured Return Protocol
- [ ] File-based return: agents write structured JSON results to `.orchestrator/phase_result_<ID>_<PHASE>.json` (e.g., `phase_result_WRK-001_prd.json`). Orchestrator reads and deletes the file after processing to prevent stale data. The ID and phase in the filename allow the orchestrator to validate it is reading the correct result
- [ ] Three return codes: `PHASE_COMPLETE`, `FAILED`, `BLOCKED`
- [ ] `BLOCKED` includes a `block_type` field (e.g., "clarification" — task is unclear, "decision" — needs a design/product decision)
- [ ] All returns include: item_id, phase, return_code, summary, context, and follow_ups array
- [ ] Autonomous skill wrapping: same skills as interactive mode, orchestrator prepends "run autonomously, don't wait for answers, use your judgment, return BLOCKED if you need a human call." Agents still generate questions they would ask — these are recorded in the phase artifact's "Assumptions" section as documentation of decisions made without human input. Existing self-critique/review sub-agents provide a built-in quality check; the wrapping approach relies on agents to self-assess whether they can proceed or should BLOCKED

#### Pre-Workflow Agents
- [ ] Triage agent: assesses new items for size/complexity/risk/impact, creates idea files if needed, promotes small + low risk items directly to ready
- [ ] Research agent: builds up idea files for items in `researching` status, promotes to `scoped` when complete. Completion requires the idea file to have non-empty problem statement, proposed approach, and size/complexity/risk assessment sections at minimum (exact required sections are a design-phase detail)
- [ ] Triage agent reconsiders impact when evaluating whether to pick up a task ("is this worth doing given what else is in the queue?")
- [ ] Auto-promotion: scoped items that meet guardrail thresholds and have `requires_human_review: false` auto-promote to ready

#### CLI Commands
- [ ] `orchestrate init [--prefix=WRK]`: scaffolds BACKLOG.yaml, `_ideas/`, `_worklog/`, and config
- [ ] `orchestrate run [--target WRK-001] [--cap N]`: directed mode (specific item) or autonomous mode (work through queue by impact priority). Exits cleanly with a message when no items are in an actionable state. `--target` on a pre-ready item forces it through remaining pre-workflow stages first (triage if `new`, research if `researching`, etc.) before entering the pipeline. `--target` on `blocked` or `done` items errors with a message
- [ ] `orchestrate status`: compact prioritized list showing ID, title, status, impact/size/risk ratings for all active items
- [ ] `orchestrate add "title" [-s small] [-r low]`: quick capture to backlog
- [ ] `orchestrate triage`: process `new` items via triage agents
- [ ] `orchestrate advance WRK-001 [--to spec]`: advance an item to next or specific phase. Validates that prerequisite artifact files exist before skipping phases; errors if they don't
- [ ] `orchestrate unblock WRK-005 [--notes "..."]`: clear blocked items with decision context. Restores the item to its pre-blocked status and phase; notes are passed as context for the next agent run

#### Deployment
- [ ] Lives in `.claude/skills/changes/` alongside existing skills — copying the skills folder gives you everything (interactive skills + orchestrator binary)

### Should Have

- [ ] Dependency tracking between backlog items (item X blocks item Y). Note: dependency fields in idea file templates are informational only in v1 — the orchestrator does not enforce them
- [ ] Tags/categories on backlog items for filtering
- [ ] Config validation on init (check skills folder structure is correct)

### Nice to Have

- [ ] Time limit on autonomous runs (in addition to phase cap)
- [ ] Bulk triage operations for processing many new items at once

## Scope

### In Scope

- Rust orchestrator binary (CLI tool)
- BACKLOG.yaml schema and management
- Idea file template and lifecycle
- Work log format and writing
- Integration with existing changes workflow skills
- Prompt wrapping for autonomous execution of existing skills
- Structured return protocol for agent output
- Project-level configuration
- Init scaffolding command
- Pre-workflow agents (triage and research)
- All CLI commands listed in success criteria
- Updating existing skills to use new `WRK-001_name` folder/file naming convention. Files requiring updates:
  - `.claude/skills/changes/SKILL.md` — folder structure examples, AI naming instructions, template folder reference
  - `.claude/skills/changes/workflow-guide.md` — naming convention references throughout
  - `.claude/skills/changes/workflows/0-prd/create-prd.md` — folder/file creation instructions
  - `.claude/skills/changes/workflows/0-prd/interview-prd.md` — file path references
  - `.claude/skills/changes/workflows/0-prd/discovery-research.md` — file path references
  - `.claude/skills/changes/workflows/1-tech-research/tech-research.md` — file path references
  - `.claude/skills/changes/workflows/2-design/design.md` — file path references
  - `.claude/skills/changes/workflows/3-spec/create-spec.md` — file path references
  - `.claude/skills/changes/templates/spec-template.md` — naming convention in template
  - `.claude/skills/changes/templates/design-template.md` — naming convention in template
  - `.claude/skills/changes/templates/tech-research-template.md` — naming convention in template

### Out of Scope

- Multi-project orchestration (one backlog per project)
- Remote/cloud execution (runs locally)
- Parallel item execution (one item at a time for v1)
- GUI or web dashboard
- Integration with external issue trackers (Jira, GitHub Issues, etc.)
- Changing how Claude Code's skill system works
- Backward-compatible support for old `NNN_` naming convention

## Non-Functional Requirements

- **Performance:** Orchestrator binary should start and make decisions in <100ms. Agent spawning time is dominated by Claude startup, not the binary.
- **Portability:** Copy `.claude/skills/changes/` into any project + have the compiled binary available → fully functional. No other runtime dependencies.
- **Reliability:** Binary must handle crashes gracefully — atomic YAML writes (write-then-rename), process group management for clean subprocess shutdown, lock file to prevent concurrent runs, and re-run-the-phase as the recovery strategy. BACKLOG.yaml is validated against the expected schema on startup; validation errors produce actionable messages and halt the orchestrator. Recovery from a corrupted YAML is `git revert` to the last good checkpoint. Note: human edits to BACKLOG.yaml while the orchestrator is running may be overwritten by the next atomic write — the orchestrator is the primary writer during autonomous runs.
- **Observability:** Terminal output shows orchestrator status (current item, phase, retry/failure counters, phase cap progress) and structured results when phases complete. Agent raw output is suppressed — the user sees the orchestrator's view. Work log provides detailed narrative audit trail.

## Constraints

- Must invoke Claude via `claude -p` (or `claude --dangerously-skip-permissions -p` for autonomous) as a subprocess. No Claude SDK or API integration — the binary is a process manager, not an AI application.
- Existing interactive skills must continue working unchanged via `/changes:*` commands. The orchestrator wraps them, not modifies them.
- Binary is compiled and copied between projects for now (not a published crate yet).
- BACKLOG.yaml is the single backlog file (no splitting across files for v1).

## Dependencies

- **Depends On:** Existing changes workflow skills in `.claude/skills/changes/` (these are invoked by the orchestrator)
- **Depends On:** Claude Code CLI being installed and available on PATH
- **Blocks:** Nothing — existing workflow continues to work independently

## Risks

- [ ] Autonomous skill invocation may not work well for phases designed for interactivity (PRD interview, design Q&A). Mitigation: prompt wrapping that tells agents to use judgment and return BLOCKED with block_type "decision" when they truly can't proceed. Agents still generate questions as assumption documentation. May need dedicated autonomous skill variants if wrapping proves insufficient for specific phases.
- [ ] Agent size/complexity/risk assessment for triage may be unreliable. Mitigation: default guardrails of medium size + low risk. Progressive refinement at each phase — if an item exceeds guardrails after reassessment, it blocks for human review. Human can adjust guardrail thresholds as trust builds.
- [ ] YAML corruption on crash. Mitigation: atomic writes (write temp file, rename) in the Rust binary.
- [ ] Skill prompt changes could break the orchestrator's output parsing. Mitigation: orchestrator appends its own output format wrapper after the skill prompt; it parses its own wrapper, not the skill's internal format.
- [ ] Two-source-of-truth drift (YAML vs SPEC). Mitigation: clear policy — YAML owns lifecycle state, SPEC owns implementation progress. Binary reconciles by reading SPEC status when resuming an in_progress item.
- [ ] Runaway follow-up generation. Mitigation: follow-up creation is uncapped (follow-ups are valuable), but phase execution cap (`--cap N`) bounds total work per run. Impact-based prioritization ensures the most important items are worked on first. Triage agent reconsiders impact when picking tasks.
- [ ] Orphan Claude processes after hard kill. Mitigation: process group management handles graceful shutdown. Lock file prevents concurrent orchestrator runs. Re-run-the-phase recovery means worst case is wasted work, not corruption.
- [ ] Prompt injection via follow-up content. Agents generate follow-ups that flow back into agent prompts via the backlog. Since autonomous mode uses `--dangerously-skip-permissions`, a malicious or badly-worded follow-up could influence agent behavior with unrestricted system access. Mitigation: triage agent sets `requires_human_review: true` on any follow-up item it assesses as medium+ risk. Only low-risk items auto-promote to `ready` for autonomous execution. The trust model assumes the solo developer trusts their own backlog content; the risk threshold provides a configurable safety net for agent-generated items.

## Open Questions

- [ ] What's the exact config file format and where does it live? (`orchestrate.yaml` in skills folder? `.orchestrate.yaml` in project root? Section in BACKLOG.yaml?)
- [ ] Should the binary embed operational templates (backlog schema, worklog format) or read them from files? Current leaning: embed in binary for things the binary exclusively manages.
- [ ] How does the binary name / path work for portability? Compile once, put on PATH? Or compile per-project in the skills folder?
- [ ] What's the exact autonomous prompt wrapper? Needs iteration with real skill invocations to tune.
- [ ] YAML comment preservation: standard Rust YAML libraries strip comments on round-trip. Accept that the orchestrator owns the YAML format (no comments), or find/build a comment-preserving approach? Current leaning: orchestrator owns the format, `orchestrate status` is the human-readable view.
- [ ] Rust binary distribution: the portability claim ("copy skills folder + have binary available") requires the binary to be pre-compiled per platform or compiled from source. Options: static musl builds for common targets, require `cargo build` as an init step, or put on PATH. Needs resolution before `orchestrate init` design.

## Work Log Format

The work log (`_worklog/YYYY-MM.md`) serves as both the archive of completed work and the detailed narrative of what happened. Newest entries at the top. Each entry includes:

- Datetime
- Item ID and task name
- Phase and outcome (return code)
- Summary of what the agent did

The orchestrator writes entries immediately when items complete (same operation as archiving from BACKLOG.yaml).

## Terminal Output Format

During autonomous runs, the terminal shows the orchestrator's status view (not raw agent output):

- Current item ID, name, and phase
- Phase cap progress (e.g., "phases completed: 3/100")
- Retry counter and consecutive failure counter
- Structured result summary when each phase completes (same content written to work log)
- Follow-up count when new items are added to backlog

## Git Checkpoint Strategy

Every successful phase commits its artifacts using the format: `[WRK-001][PHASE] Description`

This applies to all phases, not just build — PRD creation, research, design, and spec all get committed. Git history becomes a complete, greppable record of the pipeline. Course correction is standard git operations (revert, reset, cherry-pick) plus editing the YAML phase field to tell the orchestrator where to resume.

## References

- [Post-Build Workflow System Ideas](ideas/post-build-workflow-system.md) — Original idea doc with system design, backlog schema, orchestrator loop, and example flow
- [Idea File Template](ideas/IDEA_TEMPLATE.md) — Template for idea files
- [Backlog Template](ideas/BACKLOG.yaml) — Template for BACKLOG.yaml
- [Work Log Template](ideas/WORKLOG_TEMPLATE.md) — Template for work log
- [Current Changes Workflow](../../.claude/skills/changes/SKILL.md) — Existing workflow being extended
- [Current Autonomous Implementation](../../.claude/skills/changes/workflows/internal/implement-spec-autonomous.sh) — Bash script being replaced by orchestrator

## Interview Notes

_Interview conducted: 2026-02-10_
_Mode: standard_

### Key Decisions

1. **Pre-workflow agents promoted to Must Have** — the autonomous loop must be self-sustaining; follow-ups need to flow through triage and research without human intervention
2. **Phase execution cap (`--cap N`) added as Must Have** — safety valve preventing runaway loops during autonomous runs
3. **Same skills, autonomous wrapper** — interactive skills are reused; orchestrator wraps with "don't wait for answers, use judgment, return BLOCKED if you need a human." Agents still generate questions as assumption documentation
4. **Progressive assessment refinement** — each phase updates size/complexity/risk/impact ratings with better information; orchestrator re-evaluates guardrails at each transition
5. **Impact rating added to backlog schema** — used for prioritization; triage agent reconsiders impact when deciding what to work on
6. **"Highest dimension wins" for size assessment** — if any of size/complexity/risk is medium, item is medium; if any is high, item is large
7. **Follow-up creation always allowed** — no caps on creation, phase cap bounds execution; impact prioritization ensures best items worked first
8. **Crash recovery: process group + lock file + re-run-the-phase** — graceful shutdown kills subprocess; lock file prevents concurrent runs; restart re-runs current phase (idempotent for non-build, SPEC checkboxes for build)
9. **Consecutive failure circuit breaker** — 2 consecutive items exhausting retries stops the loop; resets on any success
10. **Malformed agent output = FAILED** — counts against phase_attempts, same retry logic as task failures
11. **Auto-archive done items immediately** — write to work log and prune from YAML in one operation
12. **Three return codes: PHASE_COMPLETE, FAILED, BLOCKED** — BLOCKED carries block_type field for nuance (replaces separate NEEDS_INPUT code)
13. **Single BACKLOG.yaml** — sufficient with auto-archiving keeping file small
14. **Default guardrails: medium size + low risk** — conservative but useful out of box; configurable per-project
15. **Follow-up triage between phases** — phases report in structured output; orchestrator batches into backlog atomically between transitions
16. **Work log = detailed narrative** — newest at top; datetime, ID, task, phase, outcome, summary
17. **`orchestrate status` = compact state view** — prioritized list with ID, title, status, ratings; drill into idea files for blocked-item details
18. **Git as the undo system** — commit after every phase (all phases, not just build); format `[WRK-001][PHASE] Description`; course correction via standard git operations
19. **Terminal shows orchestrator view** — status, counters, structured results; raw agent output suppressed
20. **Clean break on naming** — new `WRK-001_name` format; old `NNN_name` is legacy, not supported by orchestrator

### Critique Decisions

_Critique conducted: 2026-02-10_

21. **Backlog lifecycle maps cleanly to workflow phases** — pre-workflow statuses (`new` through `ready`) are managed by triage/research agents. `in_progress` means the item enters the changes workflow pipeline starting at PRD, traversing all phases. `done` means the final phase completed.
22. **Structured return protocol is file-based** — agents write JSON to `.orchestrator/phase_result_<ID>_<PHASE>.json`. Orchestrator reads and deletes after processing. ID+phase in filename prevents stale/mismatched data.
23. **Both pre-workflow agents remain Must Have** — without the research agent, items pile up in `researching` with no path to `scoped`/`ready`, breaking the autonomous loop. Both need detailed sub-requirements (prompt template, inputs, outputs, completion criteria) during design.
24. **Autonomous wrapping first, variants if needed** — start with prompt wrapping for all skills. Existing self-critique/review sub-agents provide built-in quality checks. Agents self-assess whether to proceed or BLOCKED. Iterate based on results.
25. **Guardrails check individual dimensions** — `max_size: medium, max_risk: low` checks each dimension independently. The "overall" rating is for human readability and display, not guardrail evaluation.
26. **Naming migration enumerated, bundled** — all 11 skill files requiring `NNN_` → `WRK-001_` updates are listed in scope as explicit success criteria.

### Deferred Items

- Exact config file format and location — decide during design phase
- Autonomous prompt wrapper wording — needs iteration with real skill invocations during implementation
