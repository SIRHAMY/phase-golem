# Phase Golem

A Rust CLI that autonomously manages a backlog of changes and executes configured workflow phases using AI agents without human intervention.

Uses [task-golem](https://github.com/SIRHAMY/task-golem) as its storage backend for work item tracking.

## Prerequisites

- [Rust toolchain](https://rustup.rs/) (stable)
- [Claude CLI](https://docs.anthropic.com/en/docs/claude-cli) (`claude`) installed and authenticated
- [task-golem](https://github.com/SIRHAMY/task-golem) (`tg`) installed — phase-golem stores all work items in task-golem's JSONL store

## Installation

```bash
# Clone the repo
git clone https://github.com/sirhamy/phase-golem.git
cd phase-golem

# Build
cargo build --release

# Copy to somewhere on your PATH
cp target/release/phase-golem ~/.local/bin/
```

## Quick Start

```bash
# 1. Initialize task-golem in your project root
tg init

# 2. Initialize phase-golem
phase-golem init --prefix WRK

# 3. Add work items via task-golem
tg add "Build user authentication"

# 4. Run the pipeline
phase-golem run
```

That's it. Phase Golem triages new items, promotes them through phases, spawns Claude subagents for each phase, commits results, and stops when everything is done or blocked.

## Commands

| Command | What it does |
|---------|-------------|
| `init --prefix <PREFIX>` | Create `phase-golem.toml` and working directories (requires `tg init` first) |
| `run [--target ID] [--cap N]` | Execute phases until halted (optionally target one item or cap phase count) |
| `status` | Show items sorted by priority |
| `triage` | Assess all `New` items (size, complexity, risk, impact) and route them |
| `advance <ID> [--to phase]` | Push an `InProgress` item to its next phase (or skip to a specific one) |
| `unblock <ID>` | Restore a `Blocked` item to its previous status |

Items are added via task-golem: `tg add "title"`. See the [tg CLI safety guide](docs/tg-cli-safety.md) for which `tg` commands are safe to use alongside phase-golem.

## How It Works

### Item Lifecycle

```
New ──triage──▶ Scoping ──pre-phases──▶ Ready ──promote──▶ InProgress ──phases──▶ Done
                                                               │
                                                               ▼
                                                           Blocked
                                                    (decision / clarification)
```

Items enter as `New`, get triaged to assess scope, run pre-phases (research/scoping) during `Scoping`, promote to `Ready` when pre-phases pass guardrails, then execute main phases (PRD, build, review, etc.) while `InProgress` until `Done`.

Any phase can block an item if it needs a human decision. Use `unblock` to resume.

### The Run Loop

When you call `phase-golem run`, this happens in a loop:

1. **Snapshot** the current item state (read-through from task-golem's JSONL store)
2. **Schedule** next actions via a pure function (`select_actions`) that picks work based on:
   - **Advance-furthest-first**: Continue items closest to completion
   - **Then scope**: Run pre-phases on `Scoping` items
   - **Then triage**: Assess `New` items last
3. **Execute** each action:
   - **Promotions** happen immediately (Ready -> InProgress)
   - **Phase runs** spawn a Claude subagent with a contextual prompt, wait for completion, and apply the result
4. **Commit** results (destructive phases commit immediately; non-destructive batch together)
5. **Check halt conditions** and repeat or stop

The loop stops when:
- All items are `Done` or `Blocked`
- Phase cap reached (`--cap`, default 100)
- Circuit breaker trips (2+ consecutive retry exhaustions)
- SIGTERM/SIGINT received
- Target item finished (`--target`)

**Adding items while running**: Use `tg add "title"` in another terminal. Phase-golem reads from the task-golem store on every scheduler loop iteration (read-through, no in-memory cache), so new items are picked up automatically.

### Key Concepts

**Pipelines** define the sequence of phases for a type of work. A `feature` pipeline might be: triage -> prd -> build -> review. Pipelines are configured in `phase-golem.toml`.

**Destructive vs non-destructive phases**: Destructive phases modify code (e.g., `build`) and must run exclusively -- no other phases run concurrently. Non-destructive phases (e.g., `prd`) can batch together.

**Staleness detection**: Before running a destructive phase, phase-golem checks that the prior phase's commit SHA is still in git history. If a rebase invalidated it, the phase blocks rather than building on stale artifacts.

**Guardrails** set thresholds (max size, complexity, risk) in `phase-golem.toml`. Items exceeding guardrails during triage get flagged for human review instead of auto-promoting.

**Follow-ups**: Phases can output discovered issues or improvements. These get ingested as new backlog items automatically.

## Project Layout

After `init`, your project gets:

```
project-root/
├── phase-golem.toml     # Pipeline definitions, guardrails, execution config
├── .task-golem/         # task-golem storage (JSONL items, archive, lock)
├── changes/             # Per-item directories with PRDs, specs, designs
│   └── HAMY-a1b2c_auth/
│       ├── HAMY-a1b2c_auth_PRD.md
│       ├── HAMY-a1b2c_auth_SPEC.md
│       └── ...
├── docs/                # Project documentation (not created by init)
│   └── tg-cli-safety.md # tg CLI safety guide for phase-golem stores
├── _ideas/              # Early-stage idea files for larger items
├── _worklog/            # Monthly archives of completed items
└── .phase-golem/       # Lock file and PID (git-ignored)
```

## Documentation

- [tg CLI Safety Guide](docs/tg-cli-safety.md) — Which `tg` commands are safe to run against a phase-golem-managed store

## Configuration

All configuration lives in `phase-golem.toml` at the project root. See [`phase-golem.example.toml`](phase-golem.example.toml) for an annotated starting point.

### `[project]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `prefix` | string | `"WRK"` | Item ID prefix for new items generated by phase-golem (e.g. follow-ups) |

### `[guardrails]`

Items exceeding these thresholds during triage get flagged for human review instead of auto-promoting.

| Key | Type | Default | Values | Description |
|-----|------|---------|--------|-------------|
| `max_size` | string | `"medium"` | `small`, `medium`, `large` | Maximum allowed item size |
| `max_complexity` | string | `"medium"` | `low`, `medium`, `high` | Maximum allowed complexity |
| `max_risk` | string | `"low"` | `low`, `medium`, `high` | Maximum allowed risk level |

### `[execution]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `phase_timeout_minutes` | integer | `30` | Kill a phase after this many minutes |
| `max_retries` | integer | `2` | Retry failed phases up to N times |
| `default_phase_cap` | integer | `100` | Max total phases executed per `run` invocation |
| `max_wip` | integer | `1` | Max items in `InProgress` status at once |
| `max_concurrent` | integer | `1` | Max phases executing in parallel |

### `[pipelines.<name>]`

Pipelines define the phase sequence for a type of work. If no pipelines are configured, a default `feature` pipeline is used.

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `pre_phases` | array | `[]` | Phases run during `Scoping` (cannot be destructive) |
| `phases` | array | `[]` | Main phases run during `InProgress` (at least one required) |

### Phase configuration

Each entry in `pre_phases` or `phases` is a table with:

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `name` | string | *(required)* | Unique name within the pipeline |
| `workflows` | array of strings | `[]` | Relative file paths to workflow files (from project root) |
| `is_destructive` | boolean | `false` | If true, phase runs exclusively (no other phases concurrent) |
| `staleness` | string | `"ignore"` | `ignore`, `warn`, `block` — how to handle stale prior-phase artifacts. `block` is incompatible with `max_wip > 1` |

### Example

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
default_phase_cap = 100
max_wip = 1
max_concurrent = 3

[[pipelines.feature.pre_phases]]
name = "research"
workflows = [".claude/skills/changes/workflows/orchestration/research-scope.md"]
is_destructive = false
staleness = "ignore"

[[pipelines.feature.phases]]
name = "prd"
workflows = [".claude/skills/changes/workflows/0-prd/create-prd.md"]
is_destructive = false

[[pipelines.feature.phases]]
name = "build"
workflows = [".claude/skills/changes/workflows/4-build/implement-spec-autonomous.md"]
is_destructive = true

[[pipelines.feature.phases]]
name = "review"
workflows = [".claude/skills/changes/workflows/5-review/change-review.md"]
is_destructive = false
```

## Architecture

```
┌─────────────┐
│  Scheduler   │  Pure function: picks next actions from backlog snapshot
└──────┬──────┘
       │ actions
       ▼
┌─────────────┐     ┌──────────────┐
│  Executor    │────▶│ Agent Runner  │  Spawns Claude CLI subprocesses
└──────┬──────┘     └──────────────┘
       │ results
       ▼
┌─────────────┐     ┌──────────────┐
│ Coordinator  │────▶│     Git      │  Stage, commit, staleness checks
└─────────────┘     └──────────────┘
  (actor pattern)
  Serialized state mutations via channel
```

- **Scheduler** (`scheduler.rs`): Pure `select_actions()` function. No I/O, fully deterministic, easy to test. Handles advance-furthest-first priority, WIP limits, exclusive locking for destructive phases, and circuit breaker logic.
- **Executor** (`executor.rs`): Runs phases with retry, staleness checks, and guardrail enforcement. Resolves what state transition to apply after each phase completes.
- **Coordinator** (`coordinator.rs`): Tokio channel-based actor that serializes all item mutations via task-golem's `Store` and git operations. Handles commits (immediate for destructive, batched for non-destructive), worklog archiving, and follow-up ingestion.
- **Agent Runner** (`agent.rs`): Spawns `claude` CLI as a subprocess, manages timeouts and signal handling (SIGTERM graceful shutdown with 5s grace period).
- **Preflight** (`preflight.rs`): Validates config structure, probes that referenced skills exist, and checks that InProgress items reference valid pipelines before any work begins.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.
