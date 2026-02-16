# Design: Overhaul Changes Workflow with Orchestrator

**ID:** 002
**Status:** Complete
**Created:** 2026-02-11
**PRD:** ./002_overhaul-changes-workflow-orchestrator_PRD.md
**Tech Research:** ./002_overhaul-changes-workflow-orchestrator_TECH_RESEARCH.md
**Mode:** Medium

## Overview

A Rust CLI binary (`orchestrate`) that drives an AI agent pipeline by managing a YAML backlog, spawning fresh Claude subprocesses for each workflow phase, parsing structured file-based results, and committing git checkpoints. The system has two tiers: a pre-workflow tier (triage and research agents that promote raw ideas to pipeline-ready items) and a workflow tier (the existing changes workflow phases — PRD through Review — wrapped with autonomous instructions). The orchestrator owns all deterministic state transitions; agents own all judgment calls. Same pipeline runs with or without a human directing it.

---

## System Design

### High-Level Architecture

```
                          orchestrate CLI (clap v4)
                                    │
                    ┌───────────────┼───────────────┐
                    │               │               │
                  init            run            status/add/triage/
                                    │            advance/unblock
                                    │
                              Pipeline Engine
                    ┌───────────────┼───────────────┐
                    │                               │
              Pre-Workflow                    Workflow Pipeline
           (Triage, Research)         (PRD → Research → Design →
                    │                  Spec → Build → Review)
                    │                               │
                    └───────────┬───────────────────┘
                                │
                          Agent Runner
                     (subprocess per phase)
                                │
                    ┌───────────┼───────────────┐
                    │           │               │
              Prompt Builder  Git Manager  Backlog Manager
                    │           │               │
              claude -p    git add/commit   BACKLOG.yaml
              (subprocess)  (shell out)    (atomic writes)
```

### Project Layout

```
project/
├── BACKLOG.yaml                    # Work queue (orchestrator-managed)
├── orchestrate.toml                # Project config
├── _ideas/                         # Pre-workflow idea files
│   ├── WRK-001_dark-mode.md
│   └── WRK-002_auth-refactor.md
├── _worklog/                       # Archived completed items
│   └── 2026-02.md
├── changes/                        # Active change folders
│   └── WRK-003_feature/
│       ├── WRK-003_feature_PRD.md
│       ├── WRK-003_feature_TECH_RESEARCH.md
│       ├── WRK-003_feature_DESIGN.md
│       └── WRK-003_feature_SPEC.md
├── .orchestrator/                  # Runtime ephemera (gitignored)
│   ├── orchestrator.lock
│   └── phase_result_WRK-003_prd.json
└── .claude/skills/changes/
    └── orchestrator/               # Rust source for the binary
        ├── Cargo.toml
        └── src/
```

### Component Breakdown

#### CLI Layer (`main.rs`)

**Purpose:** Parse commands and dispatch to the appropriate handler.

**Responsibilities:**
- Define all subcommands and arguments via clap v4 derive macros
- Validate argument combinations
- Dispatch to command handlers

**Interfaces:**
- Input: Command-line arguments
- Output: Calls into pipeline, backlog, or utility functions

**Dependencies:** clap v4

#### Backlog Manager (`backlog.rs`)

**Purpose:** All CRUD and lifecycle operations on BACKLOG.yaml.

**Responsibilities:**
- Load and validate BACKLOG.yaml against expected schema (with `schema_version`)
- Generate next sequential ID using configured prefix
- Enforce valid status transitions (see state machine below)
- Atomic writes via write-temp-rename with fsync. Implementation: use `NamedTempFile::new_in(parent_dir)` to ensure temp file is on the same filesystem as target, call `as_file().sync_all()` before `persist()` to ensure durability
- Archive done items: prune from YAML first, then write to worklog

**Interfaces:**
- Input: File path to BACKLOG.yaml
- Output: `BacklogFile` struct, mutation methods that save atomically

**Dependencies:** serde_yaml_ng, tempfile

**State Machine:**

The orchestrator manages two levels of state: **item status** (where an item is in its lifecycle) and **workflow phase** (which pipeline step is active when `in_progress`).

```
Item Status (BACKLOG.yaml `status` field)
==========================================

                    ┌─────────── Pre-Workflow ──────────────┐
                    │                                       │
  orchestrate add → new                                     │
                    │                                       │
                    ├── triage agent ──→ researching         │
                    │                      │                │
                    │                      research agent    │
                    │                      │                │
                    │                      ▼                │
                    │                    scoped              │
                    │                      │                │
                    │   (guardrails met    │                │
                    │    + no human        │                │
                    │    review needed)    │                │
                    │                      │                │
                    ├── direct promote ────┤                │
                    │   (small+low risk)   │                │
                    └──────────────────────┼────────────────┘
                                           │
                                           ▼
                    ┌─────────── Workflow Pipeline ─────────┐
                    │                                       │
                    │  ready ──→ in_progress ──→ done       │
                    │            (see phase                 │
                    │             state machine)            │
                    └───────────────────────────────────────┘

  Any status ──→ blocked (stores blocked_from_status)
  blocked    ──→ {blocked_from_status} (via orchestrate unblock)


Workflow Phase (BACKLOG.yaml `phase` field, active when status=in_progress)
============================================================================

  prd ──→ research ──→ design ──→ spec ──→ build ──→ review ──→ (done)
   │         │           │         │        │          │
   │         │           │         │        │          │
   ▼         ▼           ▼         ▼        ▼          ▼
  Each phase: one agent invocation per attempt
  Result determines next action (same loop for ALL phases):

    SUBPHASE_COMPLETE ──→ commit checkpoint ──→ loop (same phase)
    PHASE_COMPLETE    ──→ commit checkpoint ──→ advance to next phase
    FAILED            ──→ retry (up to max_retries), then block
    BLOCKED           ──→ block item, skip to next


Phase Execution Loop (universal — same for every phase)
========================================================

  ┌──→ invoke agent for current phase
  │         │
  │         ├── SUBPHASE_COMPLETE ──→ commit ──→ loop ──┐
  │         │                                            │
  │    ◄────┘────────────────────────────────────────────┘
  │
  │         ├── PHASE_COMPLETE ──→ commit ──→ advance to next phase
  │         │
  │         ├── FAILED ──→ retry logic (fresh agent, failure context)
  │         │
  │         └── BLOCKED ──→ block item

  The orchestrator doesn't know if a phase has sub-phases.
  The agent decides — build uses it for SPEC sub-phases,
  other phases could use it for draft+critique, etc.
```

#### Pipeline Engine (`pipeline.rs`)

**Purpose:** The main orchestration loop — picks items, executes phases, handles results. Starts as a single module for v1; split into sub-modules (item selection, retry handling, guardrail evaluation) proactively when natural seams emerge.

**Responsibilities:**
- Item selection: highest-impact ready item (or targeted item via `--target`)
- Phase sequencing: advance through workflow phases in order
- Build phase sub-loop: iterate over SPEC phases within the build workflow phase
- Retry logic: up to `max_retries` per phase (default 2, so 3 total attempts)
- Circuit breaker: halt after 2 consecutive items exhaust retries. Counter increments when an item exhausts all retries and is marked blocked. Resets to 0 when any item (not necessarily the same one) completes a phase successfully. Trips when counter reaches 2
- Phase cap enforcement: count every agent spawn toward `--cap N`
- Guardrail re-evaluation: check item assessments against thresholds at each phase transition
- Follow-up triage: batch follow-ups from phase results into backlog between transitions

**Interfaces:**
- Input: Backlog state, config, target item (optional), cap
- Output: Mutated backlog state, git commits, worklog entries

**Dependencies:** Backlog Manager, Agent Runner, Git Manager, Config

**Phase-to-Skill Mapping:**

| Workflow Phase | Skill Command | Notes |
|---|---|---|
| PRD | `/changes:0-prd:create-prd` | Creates change folder + PRD |
| Research | `/changes:1-tech-research:tech-research` | Mode from config or agent judgment |
| Design | `/changes:2-design:design` | Mode from config or agent judgment |
| Spec | `/changes:3-spec:create-spec` | Creates SPEC from PRD + research + design |
| Build | `/changes:4-build:implement-spec` | One invocation per SPEC sub-phase |
| Review | `/changes:5-review:change-review` | Final review + followup aggregation |

**Build Phase Sub-Loop (Agent-Driven):**

The build phase maps to multiple SPEC sub-phases. The orchestrator does NOT parse the SPEC — the agent owns all SPEC understanding. This mirrors the existing `implement-spec-autonomous.sh` pattern where the agent reads the SPEC, finds the next incomplete phase (by looking for `- [ ]` tasks), executes it, and reports back with a result code.

The build phase uses the same orchestrator loop as every other phase — no special case:
1. Orchestrator invokes build skill: "execute the next incomplete build sub-phase"
2. Agent reads SPEC, finds next incomplete phase (first `- [ ]` tasks), executes it, marks tasks `- [x]`
3. Agent returns a result code (same 4 codes used by all phases):
   - `SUBPHASE_COMPLETE` → this SPEC sub-phase done, more remain. Orchestrator commits, loops
   - `PHASE_COMPLETE` → all SPEC sub-phases done. Orchestrator commits, advances to review
   - `FAILED`/`BLOCKED` → normal retry/block logic
4. Each invocation is a separate agent spawn (fresh context) counting toward the cap
5. Crash recovery: agent re-reads SPEC and finds next `- [ ]` — naturally idempotent

#### Agent Runner (`agent.rs`)

**Purpose:** Spawn Claude subprocesses and collect structured results.

**Responsibilities:**
- Build and execute `claude --dangerously-skip-permissions -p "<prompt>"` commands
- Spawn in new process group via `process_group(0)`
- Enforce per-phase timeout via watchdog thread (default 30 minutes)
- Wait for subprocess exit, then read result JSON from `.orchestrator/`
- Validate result JSON structure
- Delete result file after successful read
- On timeout: kill process group, return FAILED result

**Interfaces:**
- Input: Full prompt string, result file path, timeout duration
- Output: `PhaseResult` (parsed from JSON) or error

**Dependencies:** std::process, nix (for killpg), signal-hook (for graceful shutdown)

**Subprocess Lifecycle:**

```
1. Create result file path: .orchestrator/phase_result_{ID}_{PHASE}.json
2. Check for stale result file at that path — if exists, delete with warning
3. Build prompt with result path included
4. Spawn: claude --dangerously-skip-permissions -p "<prompt>"
   - process_group(0) for isolation
5. Start watchdog thread with timeout
6. Wait for exit:
   - Normal exit → read result JSON → validate → return PhaseResult
   - Non-zero exit with valid result JSON → log warning, respect agent's result code
   - Timeout → SIGTERM process group, wait 5s, SIGKILL if still alive → return FAILED
   - Signal (SIGTERM/SIGINT) → kill process group → propagate shutdown
7. Delete result file
```

#### Prompt Builder (`prompt.rs`)

**Purpose:** Construct full prompts by wrapping skill invocations with autonomous instructions.

**Responsibilities:**
- Build the autonomous preamble (context, instructions, assessment info)
- Embed the skill invocation command
- Append the structured output suffix (JSON schema, result file path)
- Include item context (idea file content, previous phase summaries)
- Include unblock notes when resuming from blocked state

**Interfaces:**
- Input: Skill command, item data, phase, result file path, unblock notes
- Output: Complete prompt string

**Prompt Structure:**

```
[Autonomous Preamble]
- You are running autonomously in an orchestrated pipeline.
- Do not wait for user input. Use your best judgment.
- If you truly cannot proceed without human input, write a BLOCKED result.
- Current item: {id} — {title}
- Previous phase summary: {summary from last phase result}
- Current assessments: size={size}, complexity={complexity}, risk={risk}, impact={impact}
{unblock notes if resuming from blocked}

[Skill Invocation]
/changes:NAMESPACE:SKILL {path} {args}

[Structured Output Suffix]
CRITICAL — After completing your work, you MUST write a JSON result file.
Path: {result_file_path}
Schema:
{json_schema}
- Record questions you would have asked in the artifact's "Assumptions" section.
- Report any new work items discovered in the follow_ups array.
- Update size/complexity/risk/impact assessments if your understanding changed.
```

#### Git Manager (`git.rs`)

**Purpose:** All git operations — precondition checks and checkpoints.

**Responsibilities:**
- Startup preconditions: clean working tree, valid branch (not detached HEAD, no rebase/merge in progress)
- Stage files: `git add` with explicit paths only (change folder contents, BACKLOG.yaml, idea files, worklog). Never use `git add -A` or `git add .`
- Commit with format: `[WRK-001][PHASE] Description`
- Porcelain output parsing for status checks

**Interfaces:**
- Input: Paths to stage, commit message components
- Output: Success/failure, error details

**Dependencies:** std::process (shell out to `git` CLI)

**Commit Strategy:**

Every successful phase commits:
- The phase's artifact files (PRD, research doc, design doc, SPEC, or source code)
- Updated BACKLOG.yaml (status/phase changes)
- Updated idea files (if modified)

Format: `[WRK-001][PRD] Created PRD with 5 success criteria`

If `git commit` fails: treat as phase failure. Halt the item, log the error. Do not continue without the checkpoint.

#### Config (`config.rs`)

**Purpose:** Load and validate project-level configuration.

**Responsibilities:**
- Load `orchestrate.toml` from project root
- Provide defaults for all optional fields
- Validate values (e.g., guardrail levels are valid enum values)

**Interfaces:**
- Input: File path to orchestrate.toml (or default location)
- Output: `OrchestrateConfig` struct

**Dependencies:** toml (crate), serde

#### Lock Manager (`lock.rs`)

**Purpose:** Prevent concurrent orchestrator runs.

**Responsibilities:**
- Acquire lock on startup (non-blocking `try_lock`)
- Write PID to lock file for diagnostics
- On startup: if lock exists and PID is dead, remove stale lock with warning
- On startup: if lock exists and PID is alive, refuse to start
- Release lock on clean shutdown

**Interfaces:**
- Input: Lock file path (`.orchestrator/orchestrator.lock`)
- Output: Lock guard (RAII — released on drop)

**Dependencies:** fslock

#### Work Log (`worklog.rs`)

**Purpose:** Write narrative audit trail of completed work.

**Responsibilities:**
- Append entries to `_worklog/YYYY-MM.md` (newest at top)
- Each entry: datetime, item ID, title, phase, outcome, summary
- Called during archive (when item transitions to done)

**Interfaces:**
- Input: Completed item data, phase results history
- Output: Updated worklog file

#### Triage Agent (pre-workflow, prompt in `prompt.rs`)

**Purpose:** Assess new backlog items for size/complexity/risk/impact and route them into the appropriate pre-workflow path.

**Responsibilities:**
- Assess item dimensions (size, complexity, risk, impact) based on title and any provided context
- Decide routing: create idea file (needs research) or promote directly to `ready` (small + low risk)
- Create idea file in `_ideas/{ID}_{slug}.md` when item needs research, using the idea file template from the skills folder
- Set `requires_human_review: true` on any item assessed as medium+ risk
- Reconsider impact relative to existing backlog items ("is this worth doing given what else is queued?")

**Interfaces:**
- Input: Backlog item (title, context from `orchestrate add`)
- Output: Phase result JSON with assessments, routing decision, idea file creation flag

**Idea File Structure** (loaded from `.claude/skills/changes/templates/idea-template.md`):
- Problem statement (required for completion)
- Proposed approach (required for completion)
- Size/complexity/risk assessment (required for completion)
- Dependencies (informational)
- Open questions
- Context/references

#### Research Agent (pre-workflow, prompt in `prompt.rs`)

**Purpose:** Build up idea files for items in `researching` status until they have enough detail to be scoped.

**Responsibilities:**
- Read existing idea file for the item
- Research the problem space (codebase exploration, pattern analysis)
- Update idea file with findings: problem statement, proposed approach, assessment refinement
- Determine if idea file meets completion criteria (non-empty problem statement, proposed approach, and size/complexity/risk assessment)
- Recommend promotion to `scoped` when complete

**Interfaces:**
- Input: Backlog item + existing idea file content
- Output: Phase result JSON with updated assessments, completion determination

**Completion Criteria:** Idea file must have non-empty content in all three required sections (problem statement, proposed approach, assessment). Research agent sets result to `PHASE_COMPLETE` only when criteria are met.

### Data Models

#### BACKLOG.yaml

```yaml
schema_version: 1
items:
  - id: "WRK-001"
    title: "Add dark mode support"
    status: ready           # new | researching | scoped | ready | in_progress | done | blocked
    phase: null             # null | prd | research | design | spec | build | review
    size: small             # small | medium | large
    complexity: low         # low | medium | high
    risk: low               # low | medium | high
    impact: high            # low | medium | high
    requires_human_review: false
    origin: null            # "WRK-002/build" — source item/phase that created this
    blocked_from_status: null  # status before blocking, for restore on unblock
    blocked_reason: null
    blocked_type: null      # clarification | decision
    unblock_context: null   # human-provided notes, cleared after next successful phase
    tags: []
    dependencies: []        # informational only in v1
    created: "2026-02-11"
    updated: "2026-02-11"
```

#### Phase Result JSON (`.orchestrator/phase_result_{ID}_{PHASE}.json`)

**Result codes (uniform across all phases):**
- `SUBPHASE_COMPLETE` — A sub-unit of work within this phase is done, but more remain. Orchestrator commits checkpoint, loops back to invoke the same phase again. Used by build (SPEC sub-phases), and available for any phase that has internal sub-steps (e.g., design draft + critique, PRD draft + interview).
- `PHASE_COMPLETE` — The entire phase is done. Orchestrator commits, advances to next workflow phase.
- `FAILED` — Phase could not be completed. Triggers retry logic.
- `BLOCKED` — Phase needs human input. Includes `block_type` (clarification | decision).

The orchestrator's phase execution loop is identical for every phase — it doesn't know or care whether a phase has sub-phases. The agent decides based on its own internal state.

```json
{
  "item_id": "WRK-001",
  "phase": "prd",
  "result": "PHASE_COMPLETE",      // or SUBPHASE_COMPLETE, FAILED, BLOCKED
  "summary": "Created PRD with 5 success criteria and 3 user stories",
  "context": "PRD focuses on accessibility-first dark mode with system preference detection",
  "updated_assessments": {
    "size": "small",
    "complexity": "low",
    "risk": "low",
    "impact": "high"
  },
  "follow_ups": [
    {
      "title": "Research accessibility contrast ratios for dark mode",
      "context": "WCAG AA requires 4.5:1 ratio; need to verify palette meets this",
      "suggested_size": "small",
      "suggested_risk": "low"
    }
  ]
}
```

#### orchestrate.toml

```toml
[project]
prefix = "WRK"

[guardrails]
max_size = "medium"
max_complexity = "medium"
max_risk = "low"

[execution]
phase_timeout_minutes = 30
max_retries = 2
default_cap = 100
```

### Data Flow

1. **Command entry** — User runs `orchestrate run [--target WRK-001] [--cap N]`
2. **Startup checks** — Acquire lock, validate git preconditions, load + validate BACKLOG.yaml and config
3. **Item selection** — Pick highest-impact `ready` item (or targeted item)
4. **Transition** — Move item to `in_progress`, set phase to `prd`
5. **Phase execution loop:**
   a. Build prompt (autonomous wrapper + skill command + item context)
   b. Spawn Claude subprocess in process group with timeout
   c. Wait for exit, read result JSON, validate
   d. Handle result:
      - `SUBPHASE_COMPLETE` → triage follow-ups, commit checkpoint, loop (invoke same phase again)
      - `PHASE_COMPLETE` → triage follow-ups, commit checkpoint, advance to next workflow phase
      - `FAILED` → increment retry counter, re-run phase (up to max_retries). If exhausted, mark blocked, check circuit breaker
      - `BLOCKED` → mark item blocked with reason/type, move to next item
   e. Re-evaluate guardrails against updated assessments. If exceeded, block for human review
   f. Check phase cap. If reached, exit cleanly
6. **Item completion** — After review phase completes: archive to worklog, prune from BACKLOG.yaml
7. **Next item** — Return to step 3. Exit when no actionable items remain or cap is reached

### Key Flows

#### Flow: Autonomous Run (`orchestrate run --cap 50`)

> Process the backlog autonomously, executing up to 50 phase invocations.

1. **Startup** — Acquire lock, validate git (clean tree, valid branch), load backlog + config
2. **Select item** — Sort ready items by impact (descending), then by created date (ascending, oldest first) as tiebreaker. Pick highest. If none, exit with "No actionable items"
3. **Begin pipeline** — Transition item to `in_progress`, phase = `prd`
4. **Execute phase** — Build prompt, spawn agent, wait for result
5. **Handle PHASE_COMPLETE** — Triage follow-ups (generate next sequential ID for each, add with status `new`, origin = `{item_id}/{phase}`), commit `[WRK-001][PRD] Created PRD` (commit includes BACKLOG.yaml + artifact files + any new follow-up items), advance to next phase
6. **Continue pipeline** — Repeat step 4-5 for research, design, spec
7. **Build** — Same loop as every other phase. Agent returns `SUBPHASE_COMPLETE` (commit, loop) or `PHASE_COMPLETE` (commit, advance to review). Agent reads SPEC and finds next `- [ ]` on each invocation
8. **Review** — Execute review phase, commit
9. **Archive** — Prune item from BACKLOG.yaml, write worklog entry, commit `[WRK-001][ARCHIVE] Completed: {title}`
10. **Next item** — Return to step 2. Continue until cap reached or no items remain
11. **Shutdown** — Release lock, print summary (phases executed, items completed, items blocked, follow-ups created)

**Edge cases:**
- Phase cap reached mid-item — commit current phase completion first if applicable, then exit cleanly. Item stays `in_progress` at current phase. Resume on next run
- Agent returns BLOCKED — mark item, record reason, skip to next ready item
- Agent returns FAILED — retry up to max_retries with fresh agent, including previous failure context. If exhausted, mark blocked
- Circuit breaker trips (2 consecutive items exhaust retries) — halt run, likely systemic issue
- Guardrails exceeded after assessment update — block item for human review, preserve completed artifacts
- Git commit fails — treat as phase failure, halt item, log error

#### Flow: Targeted Run (`orchestrate run --target WRK-005`)

> Process a specific item through the pipeline.

1. **Startup** — Same as autonomous
2. **Find item** — Look up WRK-005 in backlog. Error if not found, blocked, or done
3. **Pre-workflow stages** — If item is `new`: spawn triage agent, update assessments, create idea file or direct promote based on result, commit. If `researching`: spawn research agent, update idea file, promote to `scoped` when complete, commit. If `scoped`: check guardrails, promote to `ready` if met and `requires_human_review: false`, otherwise block for human review. Each stage is a separate agent spawn counting toward the cap
4. **Begin pipeline** — Same as autonomous from step 3 onward
5. **Single item** — Run exits after this item completes, blocks, or exhausts retries

**Edge cases:**
- Item is blocked — error with message "WRK-005 is blocked: {reason}. Use `orchestrate unblock` first"
- Item is done — error with message "WRK-005 is already done"

#### Flow: Triage (`orchestrate triage`)

> Process all `new` items through triage agents.

1. **Load backlog** — Find all items with status `new`
2. **For each item:**
   a. Spawn triage agent with item title and any context
   b. Agent assesses size/complexity/risk/impact
   c. Agent decides: create idea file (needs research) or promote directly (small + low risk)
   d. Parse result, update item assessments
   e. Transition: `new` → `researching` (with idea file) or `new` → `ready` (direct promote, if guardrails met and `requires_human_review: false`)
   f. Commit checkpoint
3. **Summary** — Print count of items triaged, assessments assigned, idea files created

#### Flow: Add Item (`orchestrate add "title" [-s small] [-r low]`)

> Quick capture a new item into the backlog.

1. **Load backlog** — Read BACKLOG.yaml
2. **Generate ID** — Find highest existing numeric suffix, increment, format with prefix (e.g., WRK-014)
3. **Create item** — Status `new`, optional size/risk from flags, other fields null/default
4. **Save** — Atomic write to BACKLOG.yaml
5. **Confirm** — Print "Added WRK-014: {title}"

#### Flow: Init (`orchestrate init [--prefix=WRK]`)

> Scaffold a new project for orchestrated workflow.

1. **Create directories** — `_ideas/`, `_worklog/`, `changes/`, `.orchestrator/`
2. **Create BACKLOG.yaml** — Empty items list with schema_version: 1
3. **Create orchestrate.toml** — Defaults with specified prefix
4. **Add `.orchestrator/` to .gitignore** — Append if not already present
5. **Confirm** — Print created files and next steps

#### Flow: Advance (`orchestrate advance WRK-001 [--to spec]`)

> Manually advance an item to the next or a specific phase.

1. **Load item** — Find WRK-001, validate it's in an advanceable state
2. **Determine target** — Next sequential phase, or `--to` target
3. **Validate prerequisites** — If skipping phases, check that prerequisite artifact files exist and are non-empty. Prerequisite map: Research requires PRD file. Design requires PRD + Tech Research files. Spec requires PRD + Tech Research + Design files. Build requires SPEC file. Review requires all prior artifact files
4. **Update** — Set new phase (and status to `in_progress` if needed)
5. **Confirm** — Print "Advanced WRK-001 to {phase}"

#### Flow: Unblock (`orchestrate unblock WRK-005 [--notes "use JWT"]`)

> Clear a blocked item and provide decision context.

1. **Load item** — Find WRK-005, validate it's blocked
2. **Restore** — Set status to `blocked_from_status`, clear blocked fields
3. **Store notes** — Save unblock notes in the backlog item's `unblock_context` YAML field (cleared after the next successful phase run)
4. **Confirm** — Print "Unblocked WRK-005, resuming at {phase}. Notes: {notes}"

#### Flow: Status (`orchestrate status`)

> Show compact prioritized backlog view.

1. **Load backlog**
2. **Sort** — in_progress first, then blocked, then ready (by impact desc), then scoped, researching, new
3. **Display table:**

```
ID        Title                        Status        Phase    Impact  Size    Risk
WRK-003   Implement dark mode          in_progress   build    high    small   low
WRK-005   Refactor auth flow           blocked       design   high    medium  medium
WRK-001   Add search feature           ready         -        high    medium  low
WRK-007   Fix typo in header           ready         -        low     small   low
WRK-009   Research caching strategies  researching   -        medium  -       -
WRK-010   Improve error messages       new           -        -       -       -

6 items (1 in progress, 1 blocked, 2 ready, 1 researching, 1 new)
```

---

## Technical Decisions

### Decision: Config file as `orchestrate.toml` in project root

**Context:** Need a place for project-level configuration (prefix, guardrails, execution settings). PRD left this as an open question.

**Decision:** TOML file at project root named `orchestrate.toml`.

**Rationale:** Project root is visible and discoverable. TOML is the standard Rust config format with an excellent, stable crate (`toml`). Separate from BACKLOG.yaml keeps concerns clean — BACKLOG is data, config is settings. Can be committed to git for team sharing.

**Consequences:** Adds one file to project root. Users must know to look here for settings.

### Decision: Embed orchestrator-managed schemas, read user-customizable templates from files

**Context:** The binary needs templates for BACKLOG.yaml structure, worklog format, and phase result JSON. It also references idea file templates that users might want to customize.

**Decision:** Binary-managed schemas (BACKLOG.yaml validation, phase result JSON schema, worklog format) are embedded in the binary via compile-time constants. User-customizable templates (idea files) are read from the skills folder at runtime.

**Rationale:** The binary must validate its own schemas — embedding ensures the binary and schema can't drift. Idea file templates are user-facing and may vary per project.

**Consequences:** Schema changes require recompilation. Template changes don't.

### Decision: Binary source lives in `.claude/skills/changes/orchestrator/`

**Context:** PRD requires portability — copying the skills folder should give you everything needed. Binary distribution is an open question.

**Decision:** Rust source code lives in the skills folder. User builds with `cargo build --release` and puts the binary on PATH (or uses `cargo install --path`).

**Rationale:** Source in the skills folder means copying the folder copies the orchestrator too. Building requires Rust toolchain, but this is a Rust project — the toolchain is already present. Avoids cross-compilation burden of pre-built binaries.

**Consequences:** First-time setup requires `cargo build`. Users without Rust toolchain can't use the orchestrator (acceptable — this targets developers).

### Decision: Synchronous subprocess management (no async runtime)

**Context:** Could use tokio for async process management, or stick with synchronous std::process.

**Decision:** Synchronous. Per-phase timeout implemented via a watchdog thread using the `wait-timeout` crate.

**Rationale:** v1 is strictly sequential — one subprocess at a time. Tokio adds ~200KB binary size, compilation time, and async complexity for no benefit in sequential mode. The `wait-timeout` crate provides `child.wait_timeout(duration)` which is all we need.

**Consequences:** Adding concurrent agent support (v2) would require either threading with channels or migrating to async. The agent runner interface (`fn run_agent(...) -> PhaseResult`) abstracts this — callers don't care about the implementation.

### Decision: Flat, minimal phase result JSON schema

**Context:** Agents must write structured output reliably. Complex schemas increase failure rate.

**Decision:** Six top-level fields, no nesting beyond one level. `follow_ups` is an array of flat objects.

**Rationale:** Agent output reliability is the highest technical risk (identified in both PRD and tech research). Fewer fields and flatter structure = higher success rate. The schema can be expressed in 15 lines of JSON — easy for agents to reproduce.

**Consequences:** Less structured data per phase. If more fields are needed later, can add with `#[serde(default)]` for backward compatibility.

### Decision: Prompt wrapper approach (preamble + skill + suffix)

**Context:** The orchestrator needs to run existing interactive skills autonomously. Options: modify skills, create autonomous variants, or wrap skills.

**Decision:** Orchestrator constructs prompts as: autonomous preamble (context, instructions) + skill invocation command + structured output suffix (JSON schema, file path). Skills are never modified.

**Rationale:** PRD requires interactive skills to remain unchanged. Wrapping is validated by other tools (ccswarm, claude-flow). The preamble gives agents the context they need; the suffix gives them the output format. Existing self-critique sub-agents within skills still run and provide quality checks.

**Consequences:** If wrapping proves insufficient for specific phases (e.g., agents can't self-direct through PRD interview), dedicated autonomous skill variants may be needed. This is an acknowledged risk — start with wrapping, iterate.

### Decision: Pre-workflow and workflow phases share the same agent runner

**Context:** Triage and research agents are custom prompts (not existing skills). Workflow phases wrap existing skills. Could use different execution paths.

**Decision:** Same `run_agent()` function for both. Only the prompt construction differs — pre-workflow phases use custom prompts, workflow phases wrap skill commands.

**Rationale:** Same subprocess lifecycle, same result JSON format, same retry/timeout behavior. Different prompt construction is handled by the prompt builder, not the agent runner. One code path = fewer bugs.

**Consequences:** Pre-workflow agents must produce the same result JSON format as workflow agents. This is fine — the format is general enough.

### Decision: `.orchestrator/` directory for runtime ephemera

**Context:** Need a place for lock files and phase result JSON files. These are transient.

**Decision:** `.orchestrator/` directory at project root, added to `.gitignore` during init.

**Rationale:** Keeps ephemeral files separate from user artifacts. Gitignoring prevents accidental commits of lock files or partial results. The directory name is distinctive and unlikely to collide.

**Consequences:** Users must not manually clean this directory while the orchestrator is running.

### Decision: `schema_version` in BACKLOG.yaml from day one

**Context:** Tech research flagged YAML schema evolution as a critical area. Fields will be added as the system evolves.

**Decision:** Include `schema_version: 1` in BACKLOG.yaml. Binary validates on startup. Use `#[serde(default)]` for all optional fields. Warn on unknown fields rather than error (forward compatibility).

**Rationale:** Small upfront cost, significant future benefit. Version field enables migration logic if schema changes are breaking. Default values and lenient parsing handle non-breaking additions.

**Consequences:** Must maintain version-aware loading logic as versions increment.

### Decision: Agent-driven build sub-phases (orchestrator doesn't parse SPECs)

**Context:** The build workflow phase maps to multiple SPEC sub-phases. The orchestrator needs to invoke one agent per sub-phase (fresh context, per-sub-phase commits, cap counting) but doesn't need to understand SPEC structure.

**Decision:** All phases use the same four result codes: `SUBPHASE_COMPLETE` (more work in this phase), `PHASE_COMPLETE` (phase done), `FAILED`, `BLOCKED`. The orchestrator's execution loop is identical for every phase — it doesn't know whether a phase has sub-phases. For build, the agent reads the SPEC, finds the next phase with incomplete tasks (`- [ ]`), executes it, and returns `SUBPHASE_COMPLETE` (more SPEC phases remain) or `PHASE_COMPLETE` (all done). This mirrors the existing `implement-spec-autonomous.sh` pattern.

**Rationale:** Uniform result codes mean the orchestrator has one execution loop for all phases. The agent already knows how to read SPECs and find incomplete phases. SPEC checkboxes are the progress tracking mechanism, naturally idempotent on crash recovery. Any phase can adopt sub-phases in the future (e.g., design could split into draft + critique) without changing the orchestrator.

**Consequences:** Orchestrator has zero knowledge of phase internals. Progress display comes from the agent's summary field. No phase-specific tracking fields in BACKLOG.yaml. If any phase wants to introduce sub-phases later, it just starts returning `SUBPHASE_COMPLETE` — the orchestrator already handles it.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| No YAML comments | Human-editable BACKLOG.yaml loses inline documentation | Simpler YAML handling with serde_yaml_ng; `orchestrate status` is the human-readable view | Orchestrator is the primary writer; comments would be stripped on every write anyway |
| Sync-only in v1 | No concurrent agents; triage of 10 items is 10 sequential spawns | Simpler code, no async runtime, easier debugging | Ship v1 faster; agent runner interface abstracts the implementation for v2 migration |
| Rust toolchain required | Non-Rust developers can't build the binary without installing Rust | Source portability (copy skills folder = copy everything); no cross-compilation burden | Target users are developers; Rust toolchain install is a one-time cost |
| High commit volume | ~6+ commits per item clutters git log | Complete audit trail per phase; `git log --grep='WRK-001'` filters perfectly; any phase is revertible | Git is the undo system — more checkpoints = finer rollback granularity |
| Sequential pipeline only | Can't parallelize triage/research in v1 | Simpler state management, no concurrency bugs | v1 design decisions (centralized YAML writes, centralized git, file-based IPC) all enable v2 parallelism without redesign |
| Wrapping may produce shallow artifacts | Autonomous PRDs/designs may lack depth vs. interactive | Reuses existing skills unchanged; each phase's internal self-critique and review sub-agents provide quality signal; agent BLOCKED result catches cases where quality is insufficient | No external validation agent for v1. Monitor via worklog; add validation agents later if internal checks prove insufficient |

---

## Alternatives Considered

### Alternative: Async Runtime (tokio)

**Summary:** Use tokio for async subprocess management, timeouts, and a natural path to concurrent agents.

**How it would work:**
- `tokio::process::Command` for non-blocking subprocess spawning
- `tokio::time::timeout` for per-phase timeouts
- `tokio::spawn` for concurrent triage/research in future versions

**Pros:**
- Native timeout support without watchdog threads
- Natural path to v2 concurrency
- Non-blocking I/O for reading result files

**Cons:**
- Adds tokio runtime dependency (~200KB binary overhead, compile time)
- Async Rust has steeper learning curve and harder debugging (colored functions, pin, etc.)
- Sequential v1 gains nothing from async
- `wait-timeout` crate provides the one feature we'd use tokio for

**Why not chosen:** The only feature needed from tokio in v1 is timeouts, which `wait-timeout` handles in 3 lines. The migration cost to add tokio for v2 concurrency is bounded (swap `std::process` for `tokio::process` in agent runner). Not worth the complexity tax now.

### Alternative: TOML for Backlog

**Summary:** Use TOML instead of YAML for the BACKLOG file.

**How it would work:**
- `[[items]]` TOML array of tables for backlog items
- `toml` crate for serialization (stable, well-maintained)
- Comments preserved natively

**Pros:**
- TOML crate is rock-solid in Rust (no ecosystem churn like YAML)
- Native comment preservation
- Stricter typing (no YAML 1.1 boolean gotcha)

**Cons:**
- TOML arrays of tables (`[[items]]`) get unwieldy with many fields per item
- Less natural for hierarchical/nested data
- PRD specifies YAML; changing requires stakeholder alignment
- `serde_yaml_ng` is stable enough for our needs

**Why not chosen:** YAML is better suited for lists of complex structured items. TOML's array-of-tables syntax becomes verbose with 15+ fields per item. The YAML ecosystem concern is resolved by using `serde_yaml_ng`. PRD already specifies YAML.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Agents fail to write valid result JSON | Pipeline stalls; retries consumed on format errors, not task failures | Medium | Minimal flat schema (6 fields). Validation prompt in suffix explicitly shows the schema. Retry with failure context. Track malformed rate in worklog |
| Autonomous PRDs/designs are shallow | Low-quality artifacts propagate through pipeline; wasted build effort | Medium | Each phase's internal self-critique and review sub-agents provide quality signal. Agent returns BLOCKED when it can't meet quality bar. Human review on medium+ complexity items. Monitor quality via worklog; add validation agents if internal checks prove insufficient |
| YAML schema evolution breaks binary | New fields or changed semantics cause parse failures on existing backlogs | Low | `schema_version` from day one. `#[serde(default)]` on all optional fields. Warn on unknown fields. Version-aware migration logic when needed |
| Orphan Claude processes on hard kill (SIGKILL) | Unrestricted Claude processes continue running after orchestrator dies | Low | Process group kill handles SIGTERM/SIGINT. On restart, lock file detects stale run. Re-run-the-phase overwrites orphan writes. Consider: startup check for orphan processes referencing `.orchestrator/` paths |
| Prompt injection via follow-up content | Agent-generated follow-ups flow back into prompts; could influence future agent behavior | Low | Triage agent sets `requires_human_review: true` on medium+ risk items. Only low-risk items auto-promote. Trust model: solo developer trusts their own content; risk threshold is the safety net |
| Agent misreads SPEC progress | Agent finds wrong "next" sub-phase or misses one | Low | Agent uses existing `- [ ]` checkbox pattern (proven in current autonomous loop). SPEC checkboxes are the single source of truth. Retry on failure. Crash recovery is idempotent — agent re-reads and finds next incomplete |

---

## Integration Points

### Existing Code Touchpoints

| Path | Modification |
|------|-------------|
| `.claude/skills/changes/SKILL.md` | Update naming convention from `NNN_` to `WRK-001_` format; update folder structure examples; update AI naming instructions |
| `.claude/skills/changes/workflow-guide.md` | Update naming convention references throughout |
| `.claude/skills/changes/workflows/0-prd/create-prd.md` | Update folder/file creation instructions to use new naming |
| `.claude/skills/changes/workflows/0-prd/interview-prd.md` | Update file path references |
| `.claude/skills/changes/workflows/0-prd/discovery-research.md` | Update file path references |
| `.claude/skills/changes/workflows/1-tech-research/tech-research.md` | Update file path references |
| `.claude/skills/changes/workflows/2-design/design.md` | Update file path references |
| `.claude/skills/changes/workflows/3-spec/create-spec.md` | Update file path references |
| `.claude/skills/changes/templates/spec-template.md` | Update naming convention |
| `.claude/skills/changes/templates/design-template.md` | Update naming convention in template |
| `.claude/skills/changes/templates/tech-research-template.md` | Update naming convention in template |

### New Files Created

| Path | Purpose |
|------|---------|
| `.claude/skills/changes/orchestrator/` | Rust binary source code |
| `BACKLOG.yaml` | Work queue (created by `orchestrate init`) |
| `orchestrate.toml` | Project config (created by `orchestrate init`) |
| `_ideas/` | Pre-workflow idea files |
| `_worklog/` | Archived completed items |
| `.orchestrator/` | Runtime ephemera (gitignored) |

### External Dependencies

| Dependency | How It's Used | If Unavailable |
|-----------|---------------|----------------|
| Claude Code CLI (`claude`) | Subprocess invocation for all agent work | Orchestrator cannot function — error on startup |
| `git` CLI | Status checks, staging, commits | Orchestrator cannot function — error on startup |
| Rust toolchain (`cargo`) | Building the binary from source | Cannot build — but pre-built binary works if available |

---

## Open Questions

- [x] ~~Should unblock notes be stored in BACKLOG.yaml or as a separate file?~~ **Resolved:** YAML field `unblock_context` — notes are typically short, field is cleared after next successful phase run.
- [ ] Exact autonomous prompt wrapper wording — needs iteration with real skill invocations. The structure (preamble + skill + suffix) is defined; the exact phrasing needs tuning during implementation. Draft example wrappers for PRD and Build phases during spec phase.
- [ ] Should `orchestrate triage` run pre-workflow items through both triage and research sequentially, or only triage? Current design: `triage` command only runs triage. Research happens during `orchestrate run` when the pipeline encounters a `researching` item. Alternative: `triage` does both. Leaning: triage-only — keeps commands focused.
- [x] ~~How should the orchestrator handle items that are already `in_progress` on startup (crash recovery)?~~ **Resolved:** Auto-resume with a log message. Re-running the current phase from scratch is safe — non-build phases are idempotent (overwrite artifacts), build phases use SPEC checkboxes for progress. Log message shows which item and phase are being resumed.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain (or they're flagged for spec phase)

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-11 | Initial design draft | Full architecture, 10 components, 7 flows, 10 decisions, 2 alternatives, 6 risks |
| 2026-02-11 | Self-critique (7 agents) | Added triage/research agent components, resolved 2 open questions, clarified circuit breaker/flows/staging, added atomic write details |
| 2026-02-11 | Directional decisions resolved | (1) Agent-driven build sub-loop — orchestrator has zero SPEC knowledge, agent reports PHASE_COMPLETE/SPEC_COMPLETE. (2) Keep file-based IPC. (3) Pipeline Engine single module, split proactively. (4) No external quality validation — rely on internal phase self-critique |
| 2026-02-11 | Unified result codes | SUBPHASE_COMPLETE / PHASE_COMPLETE / FAILED / BLOCKED — same 4 codes for all phases. Orchestrator has one universal execution loop. Any phase can adopt sub-phases without orchestrator changes. Added full state machine diagrams |
| 2026-02-11 | Design finalized | Status → Complete. Future idea noted: lightweight validation agent (e.g., Haiku) to inspect SPEC/artifacts for state understanding if needed |
