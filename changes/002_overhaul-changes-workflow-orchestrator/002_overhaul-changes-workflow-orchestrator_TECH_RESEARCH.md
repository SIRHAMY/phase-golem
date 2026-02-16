# Tech Research: Overhaul Changes Workflow with Orchestrator

**ID:** 002
**Status:** Complete
**Created:** 2026-02-10
**PRD:** ./002_overhaul-changes-workflow-orchestrator_PRD.md
**Mode:** Medium

## Overview

Researching the technical landscape for building a Rust CLI orchestrator that manages an AI agent pipeline. The orchestrator manages a YAML backlog, spawns Claude CLI subprocesses for each workflow phase, parses structured output, handles retries/failures, and commits git checkpoints. Key areas: Rust CLI tooling, YAML round-trip handling, subprocess/process group management, file-based IPC, and state machine patterns.

## Research Questions

- [x] What Rust CLI framework best fits a state-machine-driven orchestrator with multiple subcommands?
- [x] How to do atomic file writes (write-then-rename) in Rust, especially on Linux/macOS?
- [x] What YAML libraries exist for Rust, and do any preserve comments on round-trip?
- [x] How to manage process groups in Rust for clean subprocess shutdown?
- [x] What patterns exist for file-based IPC between a parent process and child processes?
- [x] How do existing AI orchestration tools (if any) handle agent spawning and output parsing?
- [x] What's the best approach for lock files in Rust (PID-based, advisory locking)?
- [x] How to handle SIGTERM/SIGINT gracefully in Rust CLI applications?

---

## External Research

### Landscape Overview

The Rust ecosystem is well-suited for this orchestrator. Key areas:

- **CLI frameworks**: Dominated by clap v4; mature and well-maintained
- **YAML**: In flux — `serde_yaml` archived, `serde_yml` has a RustSec advisory (RUSTSEC-2025-0068, unsound). `serde_yaml_ng` is the recommended replacement
- **Process management**: Strong stdlib support for subprocess spawning; Unix-specific process groups via `nix` crate
- **AI orchestration**: Emerging space — `ccswarm` (Rust) and `claude-flow` (TypeScript) are the closest existing tools, but none do exactly what we're building
- **State machines**: Rust's enum system is naturally excellent for this; typestate pattern is elegant but incompatible with serde serialization

### Common Patterns & Approaches

#### Pattern: Clap v4 Derive-Based CLI

```rust
#[derive(Parser)]
#[command(name = "orchestrate")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init { #[arg(long, default_value = "WRK")] prefix: String },
    Run { #[arg(long)] target: Option<String>, #[arg(long, default_value_t = 100)] cap: u32 },
    Status,
    Add { title: String, #[arg(short)] size: Option<String> },
    Triage,
    Advance { item_id: String, #[arg(long)] to: Option<String> },
    Unblock { item_id: String, #[arg(long)] notes: Option<String> },
}
```

**When to use:** Any CLI with multiple subcommands and typed arguments.
**Tradeoffs:** Adds ~2-5s compile time from proc macros; excellent error messages with suggestions.

#### Pattern: Enum-Based State Machine

```rust
#[derive(Serialize, Deserialize)]
enum BacklogStatus { New, Researching, Scoped, Ready, InProgress, Done, Blocked }

#[derive(Serialize, Deserialize)]
enum WorkflowPhase { Prd, Research, Design, Spec, Build, Review }
```

Each variant carries data from previous phases. Transitions validated at runtime via match arms. Serde-compatible (critical since state must persist to YAML).

**Why not typestate:** State must serialize/deserialize to YAML for BACKLOG.yaml. Typestate pattern loses type information at serialization boundaries. Enum approach gives exhaustive match checking + serde support.

#### Pattern: Sequential Phase Pipeline with Fresh Subprocesses

Each phase gets a fresh Claude process via `claude -p`. Output from phase N is written to files and referenced by phase N+1. State persisted to disk between phases.

This is validated by existing tools: `ccswarm` (Rust), `claude-flow` (TypeScript), and the current `implement-spec-autonomous.sh` all use this pattern.

#### Pattern: Write-Temp-Rename for Atomic Updates

```rust
fn atomic_write(path: &Path, content: &[u8]) -> io::Result<()> {
    let dir = path.parent().unwrap();
    let mut tmp = NamedTempFile::new_in(dir)?;
    tmp.write_all(content)?;
    tmp.as_file().sync_all()?;  // fsync before rename
    tmp.persist(path)?;
    Ok(())
}
```

**Critical:** temp file must be in the same directory as target (same filesystem for atomic rename). Call `sync_all()` before `persist()` to ensure data hits disk.

#### Pattern: Process Group Isolation + Signal Handler

```rust
// Spawn in new process group
let child = Command::new("claude")
    .args(&["-p", prompt])
    .process_group(0)  // child's PID becomes group ID
    .spawn()?;

// On shutdown, kill entire group
unsafe { libc::kill(-(child.id() as i32), libc::SIGTERM); }
```

Combined with `signal-hook` for SIGTERM/SIGINT handling via `AtomicBool` flag.

#### Pattern: Convention-Based File IPC

1. Parent decides output path: `.orchestrator/phase_result_WRK-001_prd.json`
2. Parent passes path to Claude subprocess (via prompt or env var)
3. Claude agent writes JSON result to that path
4. Parent waits for subprocess exit, then reads JSON
5. Parent validates structure and deletes the file

No race conditions since parent waits for exit before reading.

### Technologies & Tools

#### CLI Framework

| Framework | Subcommands | Derive | Help | Errors | Compile Time |
|-----------|------------|--------|------|--------|-------------|
| **clap v4** | Excellent | Yes | Auto, colored | Rich, suggestions | Moderate |
| argh | Good | Yes | Basic | Basic | Fast |
| bpaf | Good | Yes + combinators | Auto | Good | Fast |

**Recommendation: clap v4**

#### YAML Libraries

| Library | Serde | Comments | YAML Ver | Status |
|---------|-------|----------|----------|--------|
| ~~serde_yaml~~ | Yes | No | 1.1 | **Archived** |
| ~~serde_yml~~ | Yes | No | 1.1 | **RUSTSEC-2025-0068, archived** |
| **serde_yaml_ng** | Yes | No | 1.1 | **Actively maintained** |
| yaml-rust2 | No | No | 1.2 | Maintained (low-level) |
| rust-yaml | No | **Yes** | 1.2 | New |
| yaml-edit | No | **Yes** | Partial | Maintained (CST-level) |

**Recommendation: serde_yaml_ng** — most direct maintained replacement. Comment preservation not needed since orchestrator owns the YAML format.

#### Atomic File Operations

| Crate | Approach | fsync | Same-dir temp |
|-------|---------|-------|---------------|
| **tempfile** | NamedTempFile::persist() | Manual | Configurable |
| atomic_write_file | Dedicated struct | Built-in | Auto |
| atomicwrites | Allow/Disallow overwrite | Built-in | Auto |

**Recommendation: tempfile** — most popular, full control, well-maintained.

#### Process & Signal Management

| Crate | Purpose | Async |
|-------|---------|-------|
| **nix** | Unix syscalls (kill, killpg, setpgid) | No |
| **signal-hook** | Signal registration/handling | Optional |
| ctrlc | Simple Ctrl-C handler | No |
| command-group | Process group management | No |

**Recommendation: Synchronous `std::process::Command`** with `process_group(0)`, `signal-hook` for graceful shutdown, `nix` for killpg.

#### Lock Files

| Crate | Mechanism | Stale Detection | PID |
|-------|-----------|----------------|-----|
| **fslock** | flock + PID | Kernel-managed | Yes |
| pidlock | PID file | Process check | Yes |
| fd-lock | flock | Kernel-managed | No |

**Recommendation: fslock** with `try_lock_with_pid()` for combined flock + PID approach. Kernel releases lock on crash; PID is readable for diagnostics.

#### Git Operations

| Approach | Pros | Cons |
|----------|------|------|
| **Shell out to git** | Battle-tested, handles all edge cases, simple | Requires git binary, text parsing |
| git2 (libgit2) | Rich API, no subprocess | Large C dep, credential issues, learning curve |
| gix (gitoxide) | Pure Rust, fast | Still maturing, API in flux |

**Recommendation: Shell out to `git` CLI.** Operations are simple (commit, status, add). Use `--porcelain` for parseable output. This is the direction major Rust projects are trending (jj/Jujutsu recently moved network ops from libgit2 to CLI).

### Standards & Best Practices

1. **Idempotent phases** — Safe to re-run any phase. Non-build phases are naturally idempotent (overwrite artifacts). Build phases use SPEC checkboxes for progress tracking.
2. **Phase isolation** — Each phase has a clear input contract (files it reads) and output contract (files it writes/modifies).
3. **Structured logging** — Log start, end, duration, and result of each phase.
4. **Git checkpoint after each phase** — Provides audit trail + rollback capability.
5. **Exhaustive match** — Never use wildcard `_` in state/phase match arms; compiler catches missing cases.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Using `serde_yml` (not `serde_yaml_ng`) | RUSTSEC-2025-0068 unsoundness advisory | Use `serde_yaml_ng` instead |
| `Child::kill()` without process groups | Only kills immediate child, not descendants | Use `process_group(0)` + negative PID kill |
| Cross-filesystem rename | `rename()` fails if temp file is on different FS | `NamedTempFile::new_in(target_dir)` |
| Forgetting fsync before rename | Power failure → zero-length file | Always `sync_all()` before `persist()` |
| Blocking lock acquisition | CLI hangs if another instance running | Use non-blocking `try_lock()` |
| Reading JSON before subprocess exits | Partial/corrupt data | Always `child.wait()` before reading |
| Parsing human-readable git output | Format varies by version/config | Use `--porcelain` flags |
| Signal handler file I/O | Unsafe in signal context | Only set `AtomicBool`; handle in main loop |
| PID reuse for stale lock detection | Theoretically possible | Combine flock (kernel-managed) with PID display |
| YAML 1.1 boolean gotcha | `yes`/`no` parsed as booleans | Quote strings in YAML or use serde_yaml_ng defaults |

### Key Learnings

- The Rust YAML ecosystem is in active transition; `serde_yaml_ng` is the safe choice today
- Process group management is essential for subprocess trees — `Child::kill()` alone is insufficient
- The "fresh subprocess per phase" pattern is validated by multiple AI orchestration tools
- Shell out to git CLI is the pragmatic choice; even major Rust git tools are moving this direction
- Enum-based state machines with serde are the right fit for serializable pipeline state
- The reth (Ethereum client) `Pipeline` struct is an excellent architectural reference for phase-based execution with checkpoint/unwind

---

## Internal Research

### Existing Codebase State

**Architecture:** Skill-based workflow system for Claude Code. Skills in `.claude/skills/changes/` invoked via `/changes:*` commands. Six-phase pipeline: PRD → Research → Design → SPEC → Build → Review.

**Current autonomous implementation** (being replaced):
- `implement-spec-autonomous.sh` — Bash script spawning fresh Claude processes
- Uses `claude --dangerously-skip-permissions -p` for subprocess invocation
- Output markers: `SPEC_COMPLETE`, `PHASE_COMPLETE`, `PHASE_FAILED`
- Retry with backoff (default 3 retries, 15s * retry_count delay)
- 5s cooldown between phases
- Logs to `/tmp/implement-spec-$$.log`
- Only handles build phases (not PRD/research/design/spec)

**No existing Rust code** — This will be the first Rust in the project.

**Relevant files/modules:**

| Path | Purpose |
|------|---------|
| `.claude/skills/changes/SKILL.md` | Skill definition, `/changes:*` routing, naming conventions |
| `.claude/skills/changes/workflow-guide.md` | Full lifecycle documentation, AI instructions |
| `.claude/skills/changes/workflows/internal/implement-spec-autonomous.sh` | Current bash orchestrator (being replaced) |
| `.claude/skills/changes/workflows/internal/implement-spec-autonomous-auto-loop.md` | Single-phase prompt template |
| `.claude/skills/changes/workflows/0-prd/create-prd.md` | PRD creation skill |
| `.claude/skills/changes/workflows/1-tech-research/tech-research.md` | Research phase skill |
| `.claude/skills/changes/workflows/2-design/design.md` | Design phase skill |
| `.claude/skills/changes/workflows/3-spec/create-spec.md` | SPEC creation skill |
| `.claude/skills/changes/workflows/4-build/implement-spec.md` | Human-in-loop build |
| `.claude/skills/changes/workflows/4-build/code-review.md` | 9-agent code review |
| `.claude/skills/changes/workflows/5-review/change-review.md` | Final review/followup aggregation |
| `.claude/skills/changes/templates/*.md` | Templates for PRD, research, design, SPEC |

**Existing patterns in use:**
- Skill invocation via `claude -p "/changes:NAMESPACE:SKILL_NAME [args]"`
- `NNN_featurename` naming convention (being migrated to `WRK-001_name`)
- Markdown with structured sections, checklists, and execution logs
- Git commit format: `[NNN][PHASE] Description`
- Phase-specific verification checklists in SPEC files
- Code review as sub-step within each build phase

### Reusable Components

1. **Subprocess invocation pattern** — The bash script demonstrates the `claude -p` invocation pattern. Translate directly to `std::process::Command`.
2. **SPEC parsing heuristics** — Find first phase with `- [ ]` incomplete tasks. Mark tasks `- [x]` as complete. Parse execution log table.
3. **Phase prompt templates** — All existing skill prompts can be reused as-is. Orchestrator wraps them with autonomous preamble.
4. **Code review as quality gate** — The existing 9-agent code review (`/code-review`) provides a built-in quality check that the autonomous loop already relies on.
5. **Template files** — Idea template, backlog template, and worklog template already exist in `changes/002_.../ideas/`.

### Constraints from Existing Code

1. **Subprocess-only invocation** — Must use `claude -p` or `claude --dangerously-skip-permissions -p`. No SDK/API.
2. **Interactive skills unchanged** — Orchestrator wraps skills with autonomous instructions; never modifies skill files for autonomous logic.
3. **11 skill files need naming migration** — `NNN_` → `WRK-001_` format across all template references and AI naming instructions.
4. **SPEC structure is fixed** — Orchestrator must parse the existing SPEC format (header fields, phase sections, task checklists, execution log table, followup sections).
5. **No existing configuration system** — No config files exist; the orchestrator needs to define its own config format and location.
6. **`.claude/settings.json`** — Pre-authorizes `cargo build`, `cargo test`, and `cargo run` commands.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| YAML libraries handle round-trip well | `serde_yaml` is archived; `serde_yml` has security advisory; ecosystem in flux | Must use `serde_yaml_ng` specifically. Need to verify it handles all schema needs. PRD decision to not preserve comments is correct and avoids this minefield |
| File-based IPC via `.orchestrator/` directory | Pattern is sound but agent must be explicitly told where to write | Autonomous prompt wrapper must include the output file path. Agent must reliably write valid JSON — malformed output = FAILED is the right call |
| Agents can self-assess whether to BLOCKED | No existing evidence this works reliably in practice | This is the highest-risk assumption. Design should plan for a "wrapping first, variants if needed" fallback. Track agent BLOCKED accuracy in worklog |
| "Same skills, autonomous wrapper" approach | Other tools (claude-flow, ccswarm) use similar wrapping | Validated pattern. But non-build phases (PRD, research, design) are inherently interactive — the quality of autonomously-generated PRDs/designs needs monitoring |
| Progressive assessment refinement works | No precedent for AI agents reliably self-assessing size/complexity/risk | Guardrail defaults (medium size, low risk) are appropriately conservative. Design should consider how to validate/calibrate assessments over time |
| Lock file prevents concurrent runs | Combined flock + PID approach handles this well | PRD's "check if PID alive" approach works. But flock is superior for crash recovery (kernel releases on crash). Use combined approach |
| Rust binary distribution ("copy skills folder + binary") | Static musl builds work but require per-platform compilation | This is an open question in the PRD. `cargo build` as init step is simpler but requires Rust toolchain. Design must resolve |

---

## Critical Areas

### Agent Output Reliability

**Why it's critical:** The entire orchestrator depends on agents producing parseable structured output. If agents fail to write valid JSON to the expected file path, the pipeline stalls.

**Why it's easy to miss:** In testing, agents usually produce good output. In production (long autonomous runs, complex prompts), output quality degrades. The PRD acknowledges malformed output = FAILED, but the retry budget (3 attempts) may not be enough if the issue is systemic (e.g., prompt too complex for reliable JSON output).

**What to watch for:**
- Design the JSON schema to be as simple as possible — fewer fields, flat structure
- Consider having the agent echo the JSON path it wrote to (stderr) as a secondary validation
- Track malformed output frequency in worklog to detect systemic issues
- Consider a brief validation prompt ("output this JSON structure") at the end of the autonomous wrapper rather than relying on agents to remember

### YAML Schema Evolution

**Why it's critical:** BACKLOG.yaml is the single source of lifecycle truth. As the system evolves, fields will be added/changed. If the binary can't handle schema changes gracefully, it breaks.

**Why it's easy to miss:** v1 schema is clean. But follow-up work will add fields, change semantics, add optional fields. Without schema versioning or lenient parsing, the binary becomes brittle.

**What to watch for:**
- Use serde's `#[serde(default)]` extensively for optional fields
- Add a `schema_version` field to BACKLOG.yaml from day one
- Design the validation step to warn on unknown fields rather than error (forward compatibility)
- Consider deriving the schema documentation from the Rust structs

### Non-Build Phase Autonomy Quality

**Why it's critical:** PRD, research, and design phases are inherently interactive — they were designed for human collaboration. Wrapping them with "use your judgment" may produce shallow or off-target artifacts.

**Why it's easy to miss:** Build phases have concrete verification (tests pass, lint clean). Non-build phases have no automated quality check. A thin PRD might look "complete" but miss critical requirements.

**What to watch for:**
- The existing self-critique steps (create-prd auto-critiques before presenting) help here
- Design should define minimum quality criteria for non-build phase artifacts
- Consider lightweight validation: "Does the PRD have N+ success criteria? Does the research doc have N+ patterns documented?"
- Track human rejection rates for autonomously-generated artifacts

### Process Cleanup on Hard Kill

**Why it's critical:** If the orchestrator is SIGKILLed (kill -9) or the machine crashes, orphan Claude processes may continue running with `--dangerously-skip-permissions`.

**Why it's easy to miss:** Process groups handle graceful shutdown (SIGTERM/SIGINT). But SIGKILL bypasses all handlers. The orphan Claude process could continue writing files, making commits, etc.

**What to watch for:**
- Process group kill (`killpg`) handles most cases
- For true hard kills: the lock file detects stale runs on next startup
- Consider: on startup, check for orphan Claude processes that reference `.orchestrator/` paths and warn the user
- The "re-run the phase from scratch" recovery strategy means orphan writes are overwritten on retry

---

## Deep Dives

### AI Orchestration Paradigms & Parallelism

**Question:** Are there orchestration paradigms beyond the sequential state machine that could work better? Can we safely fan out in early phases? What do other tools use?

#### Paradigm Survey

Seven paradigms were evaluated against our use case:

| Paradigm | Parallelism | Dynamic Structure | Failure Handling | Complexity | Best For |
|----------|------------|-------------------|-----------------|------------|----------|
| **DAG** | Native fan-out/in | Static | Per-node retry + partial re-execution | Medium | Known dependency graphs |
| **Actor Model** | Inherent (all actors concurrent) | Dynamic | Supervision trees | High | Many independent agents |
| **Event-Driven** | Multiple subscribers per event | Dynamic | Replay from event log | High | Loosely coupled async systems |
| **Blackboard** | Read-concurrent, write-serialized | Dynamic | Agent failure = no write | Medium | Exploratory/research problems |
| **Graph-Based** | Parallel branches | Dynamic (conditional edges) | Node-level checkpointing | Medium-High | Workflows with loops/branches |
| **Pipeline/Stages** | Pipeline parallelism | Static stages | Per-stage retry | Low-Medium | Sequential phase processing |
| **Petri Nets** | Formal concurrency model | Static | Must be modeled explicitly | High | Design verification |

**Key finding:** Our current design already borrows from multiple paradigms:
- **Pipeline/Stages** — core architecture (reth-inspired phase execution)
- **Actor model** — supervision hierarchy via retry policy + circuit breaker
- **Event-driven** — git commits as event log, worklog as projection
- **Blackboard** — BACKLOG.yaml as shared knowledge base agents read/write to
- **Orchestrator-Worker** — central coordinator dispatching to Claude subprocesses

The enum-based state machine is the right implementation for our use case. Typestate can't serialize to YAML. Full DAG/actor/event frameworks are overkill for a single-user CLI tool. The key insight: **the orchestration paradigm matters less than the isolation and communication boundaries.**

#### What Other Tools Use

| Tool | Pattern | Parallelism | Isolation |
|------|---------|-------------|-----------|
| **Claude Code Agent Teams** | Fork-join (lead + teammates) | 5-6 teammates parallel | File ownership convention |
| **Anthropic Multi-Agent Research** | Scatter-gather | 3-5 subagents parallel | Own context window |
| **ccswarm** | ProactiveMaster + channel coordination | Planned (not wired) | Git worktrees |
| **Vibe Kanban** | Kanban board orchestration | Task-per-agent | Git worktrees |
| **Cursor 2.0** | Multi-agent | Up to 8 parallel | Branch-per-agent |
| **claude-flow** | Swarm intelligence | Agent routing tables | Configurable |

Anthropic's own multi-agent research system found that **multi-agent outperformed single-agent by 90.2%** on evaluations. But the benefit comes from task decomposition and narrow context, not from parallelism per se. Their recommendation: "Start simple, add multi-step agentic systems only when simpler solutions fall short."

#### Fan-Out Patterns

Three patterns for parallel agents, from [Anthropic's Building Effective Agents](https://www.anthropic.com/research/building-effective-agents):

1. **Sectioning** — Different agents handle different aspects (e.g., security review, performance review, test coverage review). Already used in our 9-agent code review.
2. **Voting** — Same task N times, aggregate by majority. Useful for code review where multiple reviewers check independently.
3. **Scatter-Gather** — Fan out subtasks to N workers, gather results, synthesize. The pattern for parallel triage/research.

#### Phase-by-Phase Parallelism Safety Analysis

| Phase | Parallel Safe? | File Conflicts? | Quality Risk? | Recommendation |
|-------|---------------|-----------------|---------------|----------------|
| **Triage** | Yes | No (separate idea files) | None | Parallelize (2-3 concurrent) |
| **Research** | Yes | No (separate idea files) | Low (duplication) | Parallelize (2-3 concurrent) |
| PRD | Technically | No (separate folders) | Medium (scope conflicts) | **Sequential** |
| Tech Research | Technically | No | Low | Sequential (low value) |
| Design | Technically | No | High (arch conflicts) | **Sequential** |
| Spec | Technically | No | High (file overlap planning) | **Sequential** |
| Build | No | Yes (source files) | Very high | **Sequential** |
| Review | Technically | No | None | Sequential (not bottleneck) |

**Why triage/research are safe:** Each agent writes to a unique idea file. Agents never write to BACKLOG.yaml directly — they return structured output, and the orchestrator serializes all YAML updates. No git worktrees needed.

**Why PRD through build must be sequential:** These phases make decisions or modify state that affects the whole codebase. Parallel PRDs risk contradictory scope. Parallel designs risk conflicting architecture. Parallel builds produce merge conflicts. Anthropic's C compiler project (16 parallel agents) confirmed: parallel agents on shared code caused cascading failures.

#### The Two-Tier Pipeline Architecture

The natural evolution:

```
              ┌────────────────────────────────────┐
              │      Tier 1: Pre-Workflow           │
              │    (Parallel, max_concurrent=3)     │
              │                                     │
              │  ┌────────┐ ┌────────┐ ┌────────┐  │
              │  │Triage A│ │Triage B│ │Triage C│  │
              │  └───┬────┘ └───┬────┘ └───┬────┘  │
              │      └─────────┼──────────┘        │
              │                │                    │
              │     Fan-in: orchestrator            │
              │     serializes YAML + git           │
              └────────────────┼────────────────────┘
                               │
                               ▼
              ┌────────────────────────────────────┐
              │      Tier 2: Workflow Pipeline      │
              │    (Sequential, one item)           │
              │                                     │
              │  PRD → Research → Design → Spec →   │
              │  Build → Review                     │
              └────────────────────────────────────┘
```

**v1:** Sequential everything (current PRD design). Ship this first.
**v1.5:** Batch triage — `orchestrate triage` fans out to 2-3 agents. ~50 lines of threading code.
**v2:** Parallel triage/research while a build runs on a different item.
**v3:** Fan-out within phases (research subagents investigating different aspects).

No git worktrees needed for pre-workflow parallelism. No tokio needed — `std::thread` + `mpsc::channel` is sufficient.

#### v1 Design Decisions That Enable v2

1. **Agent invocation as a pure function:** `fn run_agent(item: &BacklogItem, phase: Phase) -> AgentResult` — easy to call from threads later
2. **Centralized YAML writes** — already planned; prevents contention
3. **Centralized git operations** — already planned; prevents index contention
4. **File-based IPC** — `.orchestrator/phase_result_<ID>_<PHASE>.json` naturally supports concurrent agents writing different files
5. **Phase concurrency as config** — add `max_concurrent` per phase type (default 1 for all = v1 behavior)

#### What Not to Do

- **Don't adopt an agent framework** (LangGraph, CrewAI are Python; actor frameworks are overkill)
- **Don't add parallelism to v1** — ship sequential, gain operational experience
- **Don't use git worktrees for pre-workflow phases** — solves a problem that doesn't exist there
- **Don't build inter-agent communication** — the orchestrator coordinates, agents are stateless workers

#### Practical Constraints

- Each `claude -p` process: ~100-300MB RAM, 2-5s startup
- 2-3 parallel agents: 300-900MB additional RAM, within typical dev machine capacity
- API rate limits: 2-3 concurrent sessions comfortable at Tier 2+ (or Max 5x subscription)
- Speedup: ~3x for triage/research batches of 3+, but only meaningful when backlog regularly accumulates multiple items

#### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Anthropic: Building Effective Agents](https://www.anthropic.com/research/building-effective-agents) | Guide | 5 composable workflow patterns; "start simple" philosophy |
| [Anthropic: Multi-Agent Research System](https://www.anthropic.com/engineering/multi-agent-research-system) | Engineering | 90.2% improvement over single-agent; scatter-gather in production |
| [Claude Code Agent Teams](https://code.claude.com/docs/en/agent-teams) | Docs | Fork-join pattern; file ownership convention; practical limitations |
| [Anthropic: Building C Compiler](https://www.anthropic.com/engineering/building-c-compiler) | Engineering | 16 parallel agents; shared-code failures; file-locking pattern |
| [Microsoft: AI Agent Design Patterns](https://learn.microsoft.com/en-us/azure/architecture/ai-ml/guide/ai-agent-design-patterns) | Guide | Sequential, concurrent, group chat, handoff, magentic patterns |
| [AWS: Scatter-Gather Patterns](https://docs.aws.amazon.com/prescriptive-guidance/latest/agentic-ai-patterns/parallelization-and-scatter-gather-patterns.html) | Guide | Fan-out/fan-in implementation patterns |
| [Simon Willison: Parallel Coding Agents](https://simonwillison.net/2025/Oct/5/parallel-coding-agents/) | Blog | Practitioner experience; bottleneck is review capacity |
| [Addy Osmani: Claude Code Swarms](https://addyosmani.com/blog/claude-code-agent-teams/) | Blog | What works/doesn't work for parallel agents |
| [ccswarm](https://github.com/nwiizo/ccswarm) | Code | Rust + git worktree isolation for parallel agents |
| [Vibe Kanban](https://github.com/BloopAI/vibe-kanban) | Code | Kanban orchestration with worktree isolation |
| [Confluent: Event-Driven Multi-Agent Systems](https://www.confluent.io/blog/event-driven-multi-agent-systems/) | Guide | Event-driven patterns for agent coordination |
| [Ractor (Rust actors)](https://github.com/slawlor/ractor) | Code | Erlang-inspired actors in Rust; supervision trees |

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| Config file format and location | Affects init scaffolding and where users configure guardrails | `orchestrate.toml` in `.claude/skills/changes/` (co-located) vs project root (visible) vs embedded in BACKLOG.yaml |
| Binary embed templates or read from files? | Affects portability and update story | Embed for things binary exclusively manages; read from files for things users might customize |
| Binary distribution approach | Affects onboarding experience | Compile from source via `cargo build` (requires Rust), static musl builds per platform, or put on PATH |
| Autonomous prompt wrapper exact wording | Affects agent output quality | Needs iteration with real skill invocations. Start with the PRD's description and refine |
| YAML comment preservation | PRD says "orchestrator owns format, no comments" but human editability suffers | `serde_yaml_ng` strips comments. Accept this (use `orchestrate status` for human view) or investigate `yaml-edit` for surgical updates |
| How to validate agent assessment accuracy | Progressive assessment refinement assumes agents can reliably estimate size/complexity/risk | Track assessment vs actual outcome; compare initial vs final ratings in worklog; calibrate over time |

### Recommended Approaches

#### CLI Framework

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| **clap v4 derive** | Ecosystem standard, rich features, great docs | Moderate compile time | Always (for this project) |

**Initial recommendation:** clap v4 with derive macros. No reason to consider alternatives for this use case.

#### YAML Handling

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| **serde_yaml_ng only** | Simple, one dependency, full serde | Strips comments, 1.1 spec | Orchestrator owns the YAML (our case) |
| serde_yaml_ng + yaml-edit | Comments preserved for human edits | Two deps, complexity | YAML is human-edited |

**Initial recommendation:** serde_yaml_ng only. The PRD explicitly positions the orchestrator as YAML owner, with `orchestrate status` as the human-readable view. Accepting no comments is the pragmatic choice.

#### Process Management

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| **Sync (std::process + nix)** | Simple, no runtime dep | Blocking waits, harder timeouts | Sequential pipeline (our case) |
| Async (tokio) | Non-blocking, easy timeouts | Runtime dep, complexity | Concurrent children |

**Initial recommendation:** Synchronous. Phases are sequential (one at a time). Per-phase timeout can be implemented with a watchdog thread or `wait_timeout` from `wait-timeout` crate rather than pulling in all of tokio.

#### Git Operations

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| **Shell out to git CLI** | Simple, battle-tested, handles edge cases | Requires git binary | Simple operations (our case) |
| git2 (libgit2) | Rich API, no subprocess | C dependency, credential issues | Deep git manipulation |

**Initial recommendation:** Shell out to `git` CLI. Our operations (status --porcelain, add, commit, log) are simple and well-served by the CLI.

#### State Machine

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| **Enum + serde** | Serializable, exhaustive matching, simple | Runtime validation | State must persist (our case) |
| Typestate | Compile-time guarantees | Can't serialize, verbose | In-memory only |

**Initial recommendation:** Enum-based state machine with serde derives. State must round-trip through YAML.

#### Lock Files

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| **flock + PID (fslock)** | Kernel releases on crash, PID for diagnostics | Unix-specific | Single-instance CLI (our case) |
| PID-only (pidlock) | Stale detection | Manual cleanup on crash | Cross-platform |

**Initial recommendation:** `fslock` with `try_lock_with_pid()`. Gets kernel-managed cleanup on crash plus readable PID.

#### Architectural Reference

The **reth (Ethereum client) Pipeline** struct is the best architectural reference:
- Stages defined as traits with `execute()` and `unwind()`
- Pipeline commits successful results to database
- Events track execution (Prepare, Run, Ran, Error, Skipped)
- Thresholds control intermediate commits

Study this for design inspiration, but our implementation will be simpler (no unwind needed, linear pipeline).

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [clap v4 docs](https://docs.rs/clap/latest/clap/) | Docs | CLI framework API and examples |
| [clap git-derive example](https://github.com/clap-rs/clap/blob/master/examples/git-derive.rs) | Code | Multi-subcommand pattern |
| [serde_yaml_ng](https://crates.io/crates/serde_yaml_ng) | Crate | YAML serialization |
| [RUSTSEC-2025-0068](https://rustsec.org/advisories/RUSTSEC-2025-0068.html) | Advisory | Why to avoid serde_yml |
| [tempfile crate](https://docs.rs/tempfile/) | Docs | Atomic file writes |
| [signal-hook crate](https://docs.rs/signal-hook) | Docs | Signal handling |
| [nix crate](https://docs.rs/nix/latest/nix/) | Docs | Unix process management |
| [fslock crate](https://docs.rs/fslock/latest/fslock/) | Docs | Lock files |
| [reth Pipeline](https://reth.rs/docs/reth_stages/struct.Pipeline.html) | Code | Phase-based execution reference |
| [reth stages docs](https://github.com/paradigmxyz/reth/blob/main/docs/crates/stages.md) | Docs | Stage execution architecture |
| [Hoverbear: State Machine Patterns](https://hoverbear.org/blog/rust-state-machine-pattern/) | Article | Enum vs typestate in Rust |
| [ccswarm](https://github.com/nwiizo/ccswarm) | Code | Rust AI agent orchestration |
| [claude-flow](https://github.com/ruvnet/claude-flow) | Code | Claude subprocess orchestration |
| [CommandExt::process_group](https://doc.rust-lang.org/std/os/unix/process/trait.CommandExt.html) | Docs | Process group spawning |
| [Rust CLI Signal Handling](https://rust-cli.github.io/book/in-depth/signals.html) | Guide | Signal handling patterns |
| [jj shells out to git](https://github.com/jj-vcs/jj/pull/5228) | PR | Why to shell out vs libgit2 |
| [Atomic file writing PSA](https://blog.elijahlopez.ca/posts/data-corruption-atomic-writing/) | Article | fsync before rename pattern |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-10 | Initial research doc created | Starting medium research |
| 2026-02-10 | External research (10 topics) | Comprehensive landscape mapping; crate recommendations for all areas |
| 2026-02-10 | Internal codebase research | Full mapping of existing skills, patterns, constraints, integration points |
| 2026-02-10 | PRD analysis + synthesis | 7 PRD concerns, 4 critical areas, 6 open questions, 6 decision areas with recommendations |
| 2026-02-10 | Deep dive: orchestration paradigms & parallelism | 7 paradigms evaluated; triage/research safe to parallelize; two-tier pipeline architecture for v2; v1 stays sequential |
| 2026-02-10 | Research marked complete | All questions answered; clear recommendations for all technical areas |
