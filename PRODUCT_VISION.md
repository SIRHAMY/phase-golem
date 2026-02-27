# Product Vision: Phase Golem

_Created: 2026-02-27_
_Last Updated: 2026-02-27_

## Target Users

### Primary Persona
- **Who:** Solo developer or small-team lead who wants to automate multi-step, agent-driven workflows -- whether that's shipping code features, processing images, generating video, or any other pipeline that can be expressed as sequential phases driven by CLI-based agents.
- **What they care about:** Adding items to a backlog, walking away, and coming back to completed work. Wants the orchestrator to triage, route items through configured phases, delegate actual work to agents, and surface decisions only when needed.

### Secondary Persona
- **Who:** Someone building or exploring AI-agent workflows who wants a structured, repeatable pipeline runner for any kind of CLI-based agent work -- not just coding.
- **What they care about:** Configuring custom pipelines with named phases and workflow files, understanding what happened at each step, and having a system that handles retries, blocking, and lifecycle management so they can focus on the workflows themselves.

## Jobs to Be Done

### Solo Developer / Pipeline Operator
- **When** I have a backlog of work items, **I want to** point an autonomous system at it and let it work through items via configured phases, **so I can** focus on higher-level decisions and review rather than driving each phase manually.
- **When** a work item is too risky or large for full autonomy, **I want to** be notified and asked for a decision, **so I can** maintain control without micromanaging every item.
- **When** I come back after a run, **I want to** see clear artifacts from each phase, **so I can** understand what was done and why without digging through logs.

### Workflow Builder
- **When** I want a structured agent-driven pipeline, **I want to** configure pipelines with named phases and workflow files, **so I can** customize the process to match whatever kind of work I'm automating.
- **When** a phase produces follow-up work, **I want to** have those items automatically added to the backlog, **so I can** avoid losing discovered issues and improvements.

## Product Values

Ordered list -- higher wins when values conflict.

1. **Correctness over speed** -- A slower run that produces correct, reviewed artifacts is better than a fast run that ships broken output. Guardrails, staleness checks, and human review gates exist for this reason.
2. **Autonomy with escape hatches** -- The system should run unattended by default, but block and surface decisions when it encounters ambiguity, risk, or scope beyond its guardrails.
3. **Transparency over magic** -- Every phase produces readable output. The scheduler is a pure function. State transitions are explicit. An operator can always understand what happened and why.
4. **Pipeline-agnostic core** -- The orchestrator should not assume what kind of work it's running. Coding workflows are the primary use case today, but the core (scheduler, executor, coordinator, lifecycle management) should work for any CLI-agent-driven pipeline.

## Constraints

- **CLI-based agent dependency:** Requires a CLI agent runner (Claude CLI, opencode, or similar) installed and available. Phase-golem is an orchestrator -- it spawns agents as subprocesses and reads their structured output. It does not host models or manage inference directly.
- **Single-project scope:** Operates within one project directory at a time. Cross-project coordination is out of scope.
- **task-golem storage:** All item state lives in task-golem's JSONL store. Phase-golem extends it via `x-pg-*` extension fields rather than maintaining its own state store.
- **Sequential destructive phases:** Only one destructive (state-mutating) phase can run at a time to avoid conflicts. For git-based workflows this prevents merge conflicts; for other pipelines it prevents concurrent writes to shared resources.

## Non-Goals

- **IDE/editor integration:** Phase-golem is a CLI tool. It does not integrate with VS Code, JetBrains, or other editors directly.
- **Multi-user collaboration:** No support for multiple humans concurrently managing the same phase-golem instance. File locking prevents concurrent runs, not concurrent users.
- **Custom AI model hosting:** Phase-golem spawns external agent CLIs. It does not load models, manage API keys, or handle inference directly.
- **Project management UI:** No web dashboard, Kanban board, or GUI. Status is checked via `phase-golem status` at the terminal.

## Success Definition

- **Hands-off pipeline execution:** An operator can add items to the backlog, run `phase-golem run`, and return to find completed work with artifacts from each phase.
- **Predictable quality:** Items that exceed guardrails get flagged for human review rather than producing low-quality output autonomously.
- **Traceable execution:** Every completed item has a traceable chain of phase artifacts showing what each agent did and decided.
- **Recovery from failure:** Failed phases retry intelligently with context from prior attempts. Blocked items can be unblocked with human decisions and resume where they left off.

## Technical Context

- **Stack:** Rust (stable), async runtime via Tokio, task-golem (JSONL storage), clap (CLI)
- **Scale:** Single operator, single project, 1-10 concurrent backlog items. Not designed for large-scale parallel execution.
- **Key Integrations:** Claude CLI (primary agent runner), opencode (alternative), task-golem (storage), git (optional -- used for commit management in code-oriented pipelines)
- **Deployment:** Local CLI binary. Installed via `cargo build --release` and copied to PATH. No server, no container, no cloud deployment.
