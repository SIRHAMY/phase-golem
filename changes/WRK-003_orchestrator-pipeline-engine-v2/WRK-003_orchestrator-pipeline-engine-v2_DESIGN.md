# Design: Orchestrator Pipeline Engine v2

**ID:** WRK-003
**Status:** Complete
**Created:** 2026-02-11
**PRD:** ./WRK-003_orchestrator-pipeline-engine-v2_PRD.md
**Tech Research:** ./WRK-003_orchestrator-pipeline-engine-v2_TECH_RESEARCH.md
**Mode:** Medium

## Overview

The v2 orchestrator replaces the hardcoded 6-phase pipeline with config-driven pipelines, adds concurrent multi-item execution with safety constraints, and introduces preflight validation and staleness detection. The architecture centers on three new modules: a **coordinator** (single actor owning all shared state), a **scheduler** (advance-furthest-first item selection with WIP/concurrency limits), and an **executor** (async phase execution with subprocess lifecycle management). The coordinator eliminates data races by serializing all backlog and git operations through mpsc channels, while tokio's async runtime enables concurrent non-destructive phases with exclusive access for destructive ones.

---

## System Design

### High-Level Architecture

```
┌──────────────────────────────────────────────────────────────┐
│                         main.rs                              │
│  Load config → Preflight → Start Coordinator → Run Scheduler │
└──────────────┬───────────────────────────────────────────────┘
               │
               ▼
┌──────────────────────────┐     ┌───────────────────────────────┐
│        Scheduler         │────▶│         Coordinator            │
│  (item selection loop)   │◀────│  (actor: backlog + git state)  │
└──────────┬───────────────┘     └───────────────────────────────┘
           │                              ▲
           ▼                              │
┌──────────────────────────┐              │
│        Executor          │──────────────┘
│  (phase tasks, retry,    │
│   subprocess lifecycle)  │
└──────────┬───────────────┘
           │
           ▼
┌──────────────────────────┐
│      AgentRunner         │
│  (async subprocess)      │
└──────────────────────────┘
```

**Data flow:** Scheduler queries coordinator for backlog state → selects items → spawns executor tasks → executors run agents and send results back to coordinator → coordinator updates state and commits.

### Component Breakdown

#### Coordinator (`coordinator.rs` — NEW)

**Purpose:** Single actor owning all mutable shared state. Eliminates data races by serializing access through message passing.

**Responsibilities:**
- Owns `BacklogFile` (the only mutable reference in the system)
- Owns git index access (stage, commit, HEAD queries, ancestry checks)
- Processes commands sequentially from a bounded mpsc channel
- Replies to each command via oneshot channel
- Batches non-destructive phase commits
- Saves final backlog state on shutdown

**Interfaces:**
- Input: `CoordinatorCommand` messages via `mpsc::Sender<CoordinatorCommand>`
- Output: Results via `oneshot::Sender<Result<T>>`
- Exposes: `CoordinatorHandle` (clonable struct wrapping the sender, with typed async methods)

**Dependencies:** `backlog.rs`, `git.rs`, `worklog.rs`

**Key types:**

```rust
enum CoordinatorCommand {
    GetSnapshot { reply: oneshot::Sender<BacklogSnapshot> },
    UpdateItem { id: String, update: ItemUpdate, reply: oneshot::Sender<Result<(), String>> },
    CompletePhase {
        item_id: String,
        result: PhaseResult,
        output_paths: Vec<PathBuf>,
        is_destructive: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    BatchCommit { reply: oneshot::Sender<Result<(), String>> },
    GetHeadSha { reply: oneshot::Sender<Result<String, String>> },
    IsAncestor { sha: String, reply: oneshot::Sender<Result<bool, String>> },
    RecordPhaseStart {
        item_id: String,
        phase: String,
        commit_sha: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    WriteWorklog {
        item: BacklogItem,
        phase: String,
        summary: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    ArchiveItem {
        item_id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    IngestFollowUps {
        follow_ups: Vec<FollowUp>,
        origin: String,
        reply: oneshot::Sender<Result<Vec<String>, String>>,  // returns new item IDs
    },
    UnblockItem {
        item_id: String,
        context: Option<String>,
        reply: oneshot::Sender<Result<(), String>>,
    },
}

struct CoordinatorHandle {
    sender: mpsc::Sender<CoordinatorCommand>,
}

impl CoordinatorHandle {
    async fn get_snapshot(&self) -> BacklogSnapshot { ... }
    async fn complete_phase(&self, ...) -> Result<(), String> { ... }
    async fn batch_commit(&self) -> Result<(), String> { ... }
    async fn get_head_sha(&self) -> Result<String, String> { ... }
    async fn is_ancestor(&self, sha: &str) -> Result<bool, String> { ... }
    // ... other typed methods
}

// Read-only snapshot of backlog state for scheduling decisions
struct BacklogSnapshot {
    items: Vec<BacklogItem>,
    schema_version: u32,
}
```

**Design notes:**
- Channel capacity: 32 (well above expected concurrent operations with typical `max_concurrent` values)
- Coordinator processes commands in a `while let Some(cmd) = rx.recv().await` loop
- Shutdown: when all senders drop, `recv()` returns `None`, coordinator saves final state and exits
- Non-destructive phase outputs are staged but not committed until `BatchCommit` is received
- Destructive phase outputs are staged and committed immediately within `CompletePhase`
- Git subprocess calls use `spawn_blocking` to avoid blocking the async runtime
- **Error handling contract:** If a coordinator command fails (git error, YAML write error), the coordinator returns `Err` via the oneshot reply. The executor treats persistence failures as distinct from phase execution failures — the item is blocked with reason "coordinator error: ..." rather than entering the retry loop (the phase succeeded; persistence failed). The scheduler detects coordinator errors and may initiate shutdown if the coordinator returns repeated errors.
- **Staging details:** `CompletePhase` stages `output_paths` plus `BACKLOG.yaml` automatically. Worklog files staged if present. The coordinator reuses `commit_checkpoint()`-style path filtering logic (stage only `changes/`, `_ideas/`, `_worklog/`, and `BACKLOG.yaml`). `BatchCommit` commits all currently staged files; no-ops if nothing staged.

#### Scheduler (`scheduler.rs` — NEW)

**Purpose:** Decides which items to advance and when, implementing advance-furthest-first scheduling with WIP and concurrency limits.

**Responsibilities:**
- Main orchestration loop (the top-level control flow for `orchestrate run`)
- Item selection based on advance-furthest-first priority
- WIP limit enforcement (`max_wip` for InProgress items only)
- Concurrency limit enforcement (`max_concurrent` non-destructive phases)
- Mutual exclusion between destructive and non-destructive phases
- Ready → InProgress promotion (when `max_wip` allows)
- New item triage dispatching

**Interfaces:**
- Input: `CoordinatorHandle`, `PipelineConfigs` (`HashMap<String, PipelineConfig>`), `ExecutionConfig`, `CancellationToken`
- Output: Spawns executor tasks via `JoinSet<(String, PhaseExecutionResult)>`

**Dependencies:** `coordinator.rs` (via handle), `executor.rs`, `types.rs`, `config.rs`

**Run-tracking data structure:**

```rust
/// Tracks actively running tasks so the scheduler knows what is in flight.
struct RunningTasks {
    /// JoinSet of (item_id, result) for all spawned executor tasks.
    join_set: JoinSet<(String, PhaseExecutionResult)>,
    /// Metadata about each running task, keyed by item_id.
    active: HashMap<String, RunningTaskInfo>,
}

struct RunningTaskInfo {
    phase: String,
    phase_pool: PhasePool,
    is_destructive: bool,
}

impl RunningTasks {
    fn has_destructive(&self) -> bool { ... }
    fn non_destructive_count(&self) -> usize { ... }
    fn is_item_running(&self, item_id: &str) -> bool { ... }
}
```

**Circuit breaker:** Preserved from v1. The scheduler tracks consecutive retry exhaustions across items. After 2 consecutive items exhaust retries (no successful phase completion in between), the scheduler halts the run with `HaltReason::CircuitBreakerTripped`. With concurrent execution, "consecutive" means sequential in completion order — if items A and B both exhaust retries without any intervening success from another item, the circuit trips.

**Scheduling algorithm (pseudocode):**

```
loop {
    snapshot = coordinator.get_snapshot().await

    if shutdown_requested: break
    if running.has_destructive(): await_any_completion(); continue

    actions = select_actions(snapshot, &running, config, pipelines)

    if actions.is_empty():
        if running.active.is_empty(): break  // all items Done/Blocked
        await_any_completion(); continue

    for action in actions:
        match action:
            Triage(id) → spawn triage executor task into join_set
            Promote(id) → coordinator.update_item(id, TransitionStatus(InProgress))
            RunPhase { item_id, phase, .. } → spawn phase executor task into join_set

    // await_any_completion: JoinSet::join_next().await
    // On completion: remove from running.active, process result
    await_any_completion()

    // Batch commit non-destructive outputs (coordinator no-ops if nothing staged)
    coordinator.batch_commit().await
}
```

**`select_actions()` priority rules:**

`fn select_actions(snapshot, running, config, pipelines) -> Vec<SchedulerAction>`

This is a pure function. Input: snapshot + running task state + config. Output: list of actions.

1. **Destructive exclusion:** If a destructive phase is running, return empty (nothing else can run).
2. **Promote:** Ready → InProgress when `in_progress_count < max_wip`.
3. **InProgress phases first:** Select InProgress items needing advancement (not already running), sorted by advance-furthest-first. Fill `max_concurrent` slots. If next phase is destructive, it must be the **only** action in the batch (exclusive access).
4. **Scoping pre_phases:** Fill remaining `max_concurrent` slots with Scoping items (not already running), sorted by advance-furthest-first.
5. **Triage:** Fill remaining `max_concurrent` slots with New items needing triage. Triage is lowest priority — InProgress and Scoping items always take scheduling priority per PRD.

**Advance-furthest-first sorting:**
- InProgress items always rank above Scoping items (regardless of phase index)
- Within each pool: higher phase index first (further along = higher priority)
- Tiebreaker: creation date ascending (FIFO — oldest item first)

#### Executor (`executor.rs` — NEW, refactored from `pipeline.rs`)

**Purpose:** Runs individual phase executions with retry logic, staleness checks, and subprocess lifecycle management.

**Responsibilities:**
- Build phase prompt (context preamble + skill command)
- Run staleness check before destructive phases
- Invoke AgentRunner with timeout
- Parse phase result
- Handle retry logic (re-execute all skills from the beginning on failure)
- Report results back to coordinator
- Track child process PGIDs in the process registry

**Interfaces:**
- Input: `BacklogItem`, `PhaseConfig`, `CoordinatorHandle`, `Arc<dyn AgentRunner>`, `CancellationToken`
- Output: `PhaseExecutionResult` (Success / Failed / Blocked)

**Dependencies:** `coordinator.rs` (via handle), `agent.rs`, `prompt.rs`, `types.rs`

**Phase transition ownership:** The executor computes what transitions are needed after a phase completes, then sends the mutations to the coordinator. The coordinator applies them atomically but does not contain domain logic. A pure `resolve_transition()` function takes `(item, phase_result, pipeline_config, guardrails_config)` and returns a list of `ItemUpdate` mutations:
- Last pre_phase completed → check guardrails → if pass: `[ClearPhase, TransitionStatus(Ready)]`; if fail: `[SetBlocked("guardrails: ...")]`
- Last main phase completed → `[TransitionStatus(Done)]`
- Mid-pipeline phase completed → `[SetPhase(next_phase, pool), SetLastPhaseCommit(sha)]`
- Phase failed with retries exhausted → `[SetBlocked("retry exhaustion: ...")]`

This keeps domain logic testable as a pure function, separate from the coordinator's state serialization.

**Phase execution flow:**

```
async fn execute_phase(item, phase_config, coordinator, agent, cancel, guardrails) -> PhaseExecutionResult {
    // 1. Staleness check (destructive phases only)
    if phase_config.destructive {
        match check_staleness(&item, &phase_config, &coordinator).await {
            StalenessResult::Proceed => {}
            StalenessResult::Warn => log_staleness_warning(&item),
            StalenessResult::Block(reason) => return PhaseExecutionResult::Blocked(reason),
        }
    }

    // 2. Record phase start (capture HEAD SHA)
    let head_sha = coordinator.get_head_sha().await
        .map_err(|e| return PhaseExecutionResult::Failed(format!("coordinator: {e}")))?;
    coordinator.record_phase_start(&item.id, &phase_config.name, &head_sha).await
        .map_err(|e| return PhaseExecutionResult::Failed(format!("coordinator: {e}")))?;

    // 3. Build prompt
    let prompt = build_phase_prompt(&item, &phase_config, &coordinator).await;
    let result_path = phase_result_path(&item, &phase_config);

    // 4. Run skills sequentially with retry (all skills re-run on retry per PRD)
    for attempt in 0..max_retries {
        let mut last_result = None;
        let mut all_succeeded = true;

        for skill in &phase_config.skills {
            let skill_prompt = format!("{}\n---\n{} {}", prompt, skill, item.change_path);
            tokio::select! {
                result = agent.run_agent(&skill_prompt, &result_path, timeout) => {
                    match result {
                        Ok(phase_result) => {
                            match phase_result.result {
                                ResultCode::SubphaseComplete => {
                                    // Build phase sub-looping: return immediately, reset retries
                                    return PhaseExecutionResult::SubphaseComplete(phase_result);
                                }
                                _ => { last_result = Some(phase_result); }
                            }
                        }
                        Err(e) => { all_succeeded = false; break; }
                    }
                }
                _ = cancel.cancelled() => return PhaseExecutionResult::Cancelled,
            }
        }

        if all_succeeded {
            let result = last_result.unwrap();
            // 5. Compute transitions and send to coordinator
            let updates = resolve_transition(&item, &result, &pipeline_config, &guardrails);
            for update in updates {
                coordinator.update_item(&item.id, update).await?;
            }
            return PhaseExecutionResult::Success(result);
        }

        if attempt < max_retries - 1 {
            log_retry(&item.id, &phase_config.name, attempt, &e);
            continue;
        }
    }
    PhaseExecutionResult::Failed(format!("retries exhausted for {}", phase_config.name))
}
```

**SubphaseComplete handling:** When the executor receives `SubphaseComplete` from a skill (currently used by the build phase for multi-step SPEC implementation), it returns `PhaseExecutionResult::SubphaseComplete`. The scheduler sees this, resets the item's retry counter, updates `previous_summary` from the result context, and re-queues the same phase for the next scheduling iteration. The item stays in the same phase and pool.

#### Preflight (`preflight.rs` — NEW)

**Purpose:** Validates all configuration and state before any work begins. Runs on every `orchestrate run`.

**Responsibilities:**
- Structural config validation (pipeline schemas, limits)
- Single skill probe agent (verify all referenced skills are accessible in one invocation)
- In-progress item validation (valid pipeline types, phase names)
- Actionable error reporting with config locations and suggested fixes

**Interfaces:**
- Input: `OrchestrateConfig`, `BacklogFile`, `Arc<dyn AgentRunner>`
- Output: `Vec<PreflightError>` (empty = pass, any errors = abort)

**Validation phases (run in order):**

1. **Structural validation** (fast, no I/O):
   - Each pipeline has ≥1 main phase
   - Phase names unique within a pipeline (across both `pre_phases` and `phases`)
   - `destructive` flag rejected on `pre_phases`
   - `max_wip >= 1` and `max_concurrent >= 1`
   - `staleness: block` rejected when `max_wip > 1` (cascading blocks make this combination unsafe)
   - Missing `[pipelines]` section → auto-generate default `feature` pipeline (not an error)

2. **Skill probe** (slow, single agent spawning):
   - Collect unique skill references across all pipelines
   - Spawn **one** probe agent with the full list of skills to verify
   - Agent checks each skill and returns a structured pass/fail per skill
   - Single agent avoids O(N) subprocess cost and Claude CLI multi-instance issues during preflight
   - Probe timeout: 60 seconds (hardcoded, not tied to `phase_timeout_minutes`)
   - Any skill failure in the probe result is an error

3. **Item validation** (fast, in-memory):
   - Each in-progress item's `pipeline_type` references a valid pipeline in config
   - Each in-progress item's `phase` references a valid phase name in that pipeline
   - Phase pool (`pre`/`main`) matches the phase's location in the pipeline config

**Error format:**
```
Preflight error: Pipeline "blog-post" phase "draft" references unknown skill "/writing/draft"
  Config: orchestrate.toml → pipelines.blog-post.phases[0].skills[0]
  Fix: Ensure the skill file exists and is readable, or correct the skill path
```

#### Config (`config.rs` — EXTENDED)

**Purpose:** Load and validate orchestrator configuration including pipeline definitions.

**New types:**

```rust
struct PipelineConfig {
    pre_phases: Vec<PhaseConfig>,
    phases: Vec<PhaseConfig>,
}

struct PhaseConfig {
    name: String,
    skills: Vec<String>,
    destructive: bool,           // default: false, rejected on pre_phases
    staleness: StalenessAction,  // default: Ignore
}

enum StalenessAction {
    Ignore,
    Warn,
    Block,
}
```

**Extended ExecutionConfig:**

```rust
struct ExecutionConfig {
    phase_timeout_minutes: u64,
    max_retries: u32,
    default_phase_cap: u32,
    max_wip: u32,          // NEW — default: 1
    max_concurrent: u32,   // NEW — default: 1
}
```

**Validation:** Serde deserialization + custom `validate()` function. Cross-field semantic rules can't be expressed with annotation-based validation alone.

**Default pipeline:** When `[pipelines]` section is missing, auto-generate:
```toml
[pipelines.feature]
pre_phases = []
phases = [
    { name = "prd",           skills = ["/changes:0-prd:create-prd"],               destructive = false },
    { name = "tech-research", skills = ["/changes:1-tech-research:tech-research"],   destructive = false },
    { name = "design",        skills = ["/changes:2-design:design"],                 destructive = false },
    { name = "spec",          skills = ["/changes:3-spec:create-spec"],              destructive = false },
    { name = "build",         skills = ["/changes:4-build:implement-spec-autonomous"], destructive = true },
    { name = "review",        skills = ["/changes:5-review:change-review"],          destructive = false },
]
```

#### Types (`types.rs` — MODIFIED)

**Removals:**
- `WorkflowPhase` enum — replaced by `String` phase names

**Changes:**
- `BacklogItem.phase: Option<WorkflowPhase>` → `Option<String>`
- `BacklogItem.status: ItemStatus` — `Researching` renamed to `Scoping`, `Scoped` removed
- `PhaseResult.phase: WorkflowPhase` → `String`

**New fields on `BacklogItem`:**
- `pipeline_type: Option<String>` — which pipeline config to use (default: `"feature"`)
- `description: Option<String>` — free-form user description from `orchestrate add --description`
- `phase_pool: Option<PhasePool>` — disambiguates which phase list the current phase belongs to
- `last_phase_commit: Option<String>` — HEAD SHA at last phase execution, for staleness detection

**New field on `PhaseResult`:**
- `based_on_commit: Option<String>` — HEAD SHA when this phase ran
- `pipeline_type: Option<String>` — set by triage agent to assign/reclassify an item's pipeline

**New types:**

```rust
/// Discriminator for which phase list the current phase belongs to.
/// Exactly two valid values — use an enum, not a string.
enum PhasePool {
    Pre,
    Main,
}

/// Describes a mutation to apply to a BacklogItem via the coordinator.
enum ItemUpdate {
    TransitionStatus { new_status: ItemStatus },
    SetPhase { phase: String, phase_pool: PhasePool },
    ClearPhase,
    SetBlocked { reason: String, blocked_type: Option<String> },
    Unblock { reset_commit: Option<String> },
    UpdateAssessments(UpdatedAssessments),
    SetPipelineType(String),
    SetLastPhaseCommit(String),
    SetDescription(String),
}

/// Result of executing a single phase via the executor.
enum PhaseExecutionResult {
    Success(PhaseResult),
    SubphaseComplete(PhaseResult),  // Build phase sub-looping: stay in same phase, reset retries
    Failed(String),
    Blocked(String),
    Cancelled,
}

/// Action selected by the scheduler for the current iteration.
enum SchedulerAction {
    Triage(String),                                         // item_id
    Promote(String),                                        // item_id: Ready → InProgress
    RunPhase { item_id: String, phase: String, phase_pool: PhasePool, is_destructive: bool },
}
```

**ItemStatus changes:**
```
Before: New, Researching, Scoped, Ready, InProgress, Done, Blocked
After:  New, Scoping, Ready, InProgress, Done, Blocked
```

#### Migration (`migration.rs` — NEW)

**Purpose:** Migrate BACKLOG.yaml from schema v1 to v2. Preserves v1 struct definitions for parsing.

**Status mapping:**

| v1 Status | v2 Status | Notes |
|-----------|-----------|-------|
| New | New | Unchanged |
| Researching | Scoping | `phase_pool: "pre"`, `phase`: first pre_phase name |
| Scoped | Ready | Phase cleared |
| Ready | Ready | Unchanged |
| InProgress | InProgress | Unchanged, `phase_pool: "main"` |
| Done | Done | Unchanged |
| Blocked | Blocked | `blocked_from_status` also mapped |

**Additional fields added during migration:**
- `pipeline_type: Some("feature")` on all existing items
- `description: None`
- `phase_pool`: set based on mapped status
- `last_phase_commit: None`
- `phase`: `WorkflowPhase` variant → equivalent string name via existing `as_str()` mapping

**Migration process:**
1. Load raw YAML, check `schema_version` (default 1 if missing)
2. If v1: parse using v1 struct definitions (preserved in this module)
3. Map all fields per table above
4. Write v2 with atomic write-temp-rename pattern (existing pattern in `backlog.rs`)
5. Bump `schema_version: 2`

**Safety:**
- V1 struct definitions preserved in this module for parsing old formats
- Stale v1 result files: parse failure triggers re-run of the phase, not a crash
- Migration is idempotent: running on v2 data is a no-op

#### Agent (`agent.rs` — MODIFIED)

**Trait change:**

```rust
// Before
pub trait AgentRunner {
    fn run_agent(&self, prompt: &str, result_path: &Path, timeout: Duration)
        -> Result<PhaseResult, String>;
}

// After
#[async_trait]
pub trait AgentRunner: Send + Sync {
    async fn run_agent(&self, prompt: &str, result_path: &Path, timeout: Duration)
        -> Result<PhaseResult, String>;
}
```

**`CliAgentRunner` changes:**
- `std::process::Command` → `tokio::process::Command`
- `wait-timeout` crate → `tokio::time::timeout` wrapping `child.wait()`
- `pre_exec`/`setpgid` preserved (still needed for process group creation)
- `kill_on_drop(true)` set on all spawned children as safety net
- On spawn: register PGID in global process registry
- On completion/drop: remove PGID from registry

**`MockAgentRunner` changes:**
- `std::sync::Mutex` → `tokio::sync::Mutex`
- Result popping becomes an async operation (trivially, since the lock isn't held across `.await`)

**Process registry:**

```rust
// Global registry of active child process group IDs
static PROCESS_REGISTRY: OnceLock<Arc<std::sync::Mutex<HashSet<Pid>>>> = OnceLock::new();

fn register_child(pgid: Pid) { ... }
fn unregister_child(pgid: Pid) { ... }
fn kill_all_children() {
    // SIGTERM all PGIDs, wait 5s, SIGKILL survivors
}
```

Uses `std::sync::Mutex` (not tokio's) because registry operations are fast (insert/remove/iterate) with no I/O under the lock.

#### Prompt (`prompt.rs` — MODIFIED)

**Changes:**
- Skill invocation: lookup from pipeline config (`PhaseConfig.skills`) instead of hardcoded `WorkflowPhase → skill` mapping
- Context preamble: structured markdown block prepended to skill command
- Autonomous mode flag included in preamble

**Context preamble format:**

```markdown
## Orchestrator Context

**Mode:** autonomous
**Item:** WRK-003 — Orchestrator Pipeline Engine v2
**Pipeline:** feature
**Phase:** build (4/6, main)
**Description:** [user's free-form description]

### Previous Phase Summary
[Extracted from last PhaseResult]

### Retry Context
Attempt 2/3. Previous failure: [error summary]

### Unblock Context
[Human's unblock notes from `orchestrate unblock`]

---

/changes:4-build:implement-spec-autonomous changes/WRK-003_orchestrator-pipeline-engine-v2/
```

**Delivery mechanism:** Embedded directly in the prompt string passed to `claude` CLI via `-p` flag. No files, no stdin, no special mechanisms. The context is naturally read by the agent before it processes the skill command.

#### Git (`git.rs` — EXTENDED)

**New functions:**

```rust
pub fn get_head_sha(project_root: &Path) -> Result<String, String>
// Runs: git rev-parse HEAD
// Returns: full 40-character SHA

pub fn is_ancestor(sha: &str, project_root: &Path) -> Result<bool, String>
// Runs: git merge-base --is-ancestor <sha> HEAD
// Exit 0 → true (ancestor, not stale)
// Exit 1 → false (not ancestor, stale)
// Exit 128 → error (unknown commit, treat as stale)
```

All git functions remain synchronous internally. The coordinator calls them via `spawn_blocking` to avoid blocking the async runtime.

#### Existing Modules — Minor Changes

- **`main.rs`** — `#[tokio::main]` entry point, async command dispatch, `orchestrate add` with `--description`, `--pipeline`, `--size`, `--risk` flags (hints for triage), `orchestrate status` displays `pipeline_type` per item, `orchestrate init` writes default `[pipelines.feature]` to TOML, `orchestrate advance` validates phase names against pipeline config with `phase_pool` boundary enforcement, `orchestrate unblock` routes through coordinator when running (or direct file access with lock when standalone)
- **`backlog.rs`** — Load/save logic preserved, save operations routed through coordinator in runtime (the function itself is still callable directly for migration)
- **`worklog.rs`** — Work log writes routed through coordinator
- **`lock.rs`** — Unchanged (single-instance guarantee still needed)

### Module Structure Summary

```
src/
├── main.rs          -- #[tokio::main], CLI commands, config loading
├── types.rs         -- BacklogItem (new fields), PhaseResult, ItemStatus, PipelineConfig
├── config.rs        -- Config loading + pipeline validation + new structs
├── coordinator.rs   -- NEW: actor owning BacklogFile + git, message types, handle
├── scheduler.rs     -- NEW: advance-furthest-first loop, WIP/concurrency management
├── executor.rs      -- NEW: phase execution, retry, staleness check, subprocess lifecycle
├── preflight.rs     -- NEW: startup validation (structural + skill probes + item checks)
├── migration.rs     -- NEW: v1 struct definitions, v1→v2 backlog mapping
├── backlog.rs       -- BACKLOG.yaml load/save (save routed through coordinator at runtime)
├── agent.rs         -- Async AgentRunner trait, CliAgentRunner, MockAgentRunner, process registry
├── prompt.rs        -- Context preamble builder + config-driven skill lookup
├── git.rs           -- Git operations (+ get_head_sha, is_ancestor)
├── worklog.rs       -- Work logging (routed through coordinator)
└── lock.rs          -- Lock file management (unchanged)
```

### Data Flow

1. **Startup:** `main.rs` loads config → runs preflight → creates coordinator (loads backlog, auto-migrates v1→v2) → starts scheduler
2. **Scheduling:** Scheduler requests snapshot from coordinator → selects items via advance-furthest-first → spawns executor tasks via TaskTracker
3. **Execution:** Executor builds prompt (context preamble + skill command) → calls `AgentRunner.run_agent()` → agent subprocess runs → produces PhaseResult
4. **Completion:** Executor sends `CompletePhase` to coordinator → coordinator updates backlog item (status, phase, commit SHA) → stages output files → commits (immediately for destructive, batched for non-destructive)
5. **Loop:** Scheduler awaits any task completion → requests new snapshot → selects next items → repeats
6. **Shutdown:** Signal sets atomic flag → CancellationToken cancelled → tasks see cancellation → SIGTERM all PGIDs → 5s grace → SIGKILL survivors → TaskTracker.wait() → coordinator saves final state → exit

### Key Flows

#### Flow: Main Orchestration Loop (`orchestrate run`)

> Drives all items through their pipelines with concurrent scheduling.

1. **Load config** — Parse `orchestrate.toml` including `[pipelines]` section
2. **Preflight validation** — Structural checks → skill probes → item validation. Any error aborts.
3. **Acquire lock** — Single-instance lock via existing `LockGuard`
4. **Start coordinator** — Load backlog (auto-migrate v1→v2 if needed), spawn coordinator actor task
5. **Enter scheduler loop** — With `CancellationToken` and `TaskTracker`
6. **Get snapshot** — Scheduler calls `coordinator.get_snapshot()`
7. **Select actions** — Apply advance-furthest-first with WIP/concurrency constraints
8. **Execute actions** — Spawn executor tasks for triage, promotion, and phase execution
9. **Await completions** — `tokio::select!` on task completion handles + cancellation token
10. **Process results** — Executor reports back via coordinator handle (update item, stage files)
11. **Batch commit** — Scheduler triggers `coordinator.batch_commit()` for non-destructive outputs
12. **Loop** — Return to step 6 until all items Done/Blocked or shutdown received

**Edge cases:**
- All items Done → scheduler loop exits normally, log summary
- All remaining items Blocked → scheduler loop exits, log blocked items
- Crash recovery → items left in Scoping/InProgress are picked up by normal scheduling on restart
- Shutdown signal → graceful shutdown flow (below)

#### Flow: Phase Execution

> Running a single phase (one or more skills) for a single item.

1. **Staleness check** — (Destructive phases only) Get `item.last_phase_commit` from coordinator, run `is_ancestor` check
2. **Record phase start** — Capture HEAD SHA via coordinator, store on item as `last_phase_commit`
3. **Build prompt** — Context preamble (item metadata, previous phase summary, retry/unblock context) + skill command(s) from `PhaseConfig.skills`
4. **Run skills sequentially** — For each skill in the phase's `skills` array, invoke `AgentRunner.run_agent()`
5. **Await result** — Agent produces PhaseResult (or timeout triggers failure)
6. **On success** — Send `CompletePhase` to coordinator with output paths and `is_destructive` flag
7. **On failure** — Retry: re-execute all skills from the beginning (up to `max_retries`). If exhausted, block item.
8. **Phase transition** — Executor calls `resolve_transition()` (pure function) to compute the needed mutations, then sends them to coordinator via `UpdateItem` commands. Transitions:
   - Last pre_phase → guardrails check (via `passes_guardrails()`) → pass: `Scoping → Ready`; fail: `Scoping → Blocked`
   - Last main phase → `InProgress → Done`
   - Mid-pipeline → advance to next phase in pool
9. **SubphaseComplete** — (Build phase only) Executor returns `SubphaseComplete` to scheduler. Scheduler resets retry counter and re-queues same phase.

**Edge cases:**
- Phase timeout → treated as failure, enters retry loop
- Multi-skill phase partial failure → entire phase fails, all skills re-run on retry
- Staleness `block` → item blocked with reason, surfaced via `orchestrate status`
- Staleness with unknown commit (exit 128) → treat as stale, block regardless of config
- Cancellation during execution → agent subprocess killed via PGID, task returns `Cancelled`

#### Flow: Preflight Validation

> Validates all configuration and state before any work begins.

1. **Parse pipeline configs** — Deserialize `[pipelines]` from TOML via serde
2. **Structural validation** — Unique phase names per pipeline, ≥1 main phase, no destructive pre_phases, limits ≥ 1
3. **Default pipeline** — If no `[pipelines]` section, generate default `feature` pipeline matching current workflow
4. **Collect unique skills** — Deduplicate all skill references across all pipelines
5. **Skill probe** — Spawn a single probe agent with the full list of unique skills. Agent checks each skill and returns a structured result (pass/fail per skill). One agent invocation instead of O(N) probes. 60-second timeout.
6. **Item validation** — Each in-progress item's `pipeline_type` exists in config, current `phase` exists in that pipeline's phase list, `phase_pool` matches phase location
7. **Report errors** — Each error includes: failing condition, config file + key, suggested fix
8. **Result** — Any error aborts the entire `orchestrate run`

**Edge cases:**
- Skill probe timeout (60s) → treated as probe failure, all skills assumed inaccessible
- Probe agent returns partial failures → all failing skills reported, then abort
- Missing `[pipelines]` → auto-generate default (not an error)
- No items in progress → item validation step is skipped (no errors possible)

#### Flow: Staleness Detection

> Checks if prior phase artifacts are based on code still in current git history.

1. **Get `last_phase_commit`** — From the item's stored SHA (set during previous phase execution)
2. **If None** — No prior commit recorded (first phase or legacy item). Proceed without check.
3. **Run ancestry check** — Coordinator calls `git merge-base --is-ancestor <sha> HEAD`
4. **Ancestor (exit 0)** — Commit is in current history. Not stale. Proceed.
5. **Not ancestor (exit 1)** — Commit is no longer reachable (e.g., after rebase). Check phase's `staleness` config:
   - `ignore` (default) → proceed
   - `warn` → log staleness warning with SHA and item ID, proceed
   - `block` → block item with reason "Stale: prior phase based on commit <sha> no longer in history"
6. **Unknown commit (exit 128)** — SHA doesn't exist in repo. Block regardless of config (data integrity issue).

**Edge cases:**
- `orchestrate unblock` on staleness-blocked item → resets `last_phase_commit` to current HEAD, user accepts artifact staleness
- Multiple items stale simultaneously (with `max_wip > 1`) → each blocked independently
- `staleness: block` with `max_wip > 1` → cascading blocks possible. Documented: `block` is only safe with `max_wip: 1`.

#### Flow: Graceful Shutdown

> Clean termination when SIGTERM/SIGINT received.

1. **Signal received** — Handler sets `AtomicBool` flag (existing pattern, async-signal-safe)
2. **Scheduler detects flag** — At next loop iteration or via `select!` on flag
3. **Cancel token** — `CancellationToken.cancel()` propagates to all tasks
4. **TaskTracker closed** — `tracker.close()` — no new tasks accepted
5. **Phase tasks see cancellation** — Each `select!` branch fires the cancellation path
6. **SIGTERM all PGIDs** — Iterate process registry, send `SIGTERM` to each process group
7. **Grace period** — Wait 5 seconds for processes to exit cleanly
8. **SIGKILL survivors** — Send `SIGKILL` to any PGIDs still in registry
9. **TaskTracker wait** — `tracker.wait()` — all tasks confirmed complete
10. **Coordinator shutdown** — All `CoordinatorHandle` clones dropped → `recv()` returns `None` → coordinator saves final backlog state and exits
11. **Exit** — Process exits cleanly

**Edge cases:**
- Second signal during grace period → immediate SIGKILL all (bypass remaining grace)
- Coordinator has in-flight operation during shutdown → finishes current command, then exits on next `recv()`
- No running tasks at shutdown time → immediate clean exit

#### Flow: Schema Migration (v1 → v2)

> Automatic one-time migration on first load after upgrade.

1. **Load BACKLOG.yaml** — Read raw YAML content
2. **Check `schema_version`** — If field missing, assume v1
3. **If already v2** — No migration needed, parse normally and return
4. **Parse as v1** — Use preserved v1 struct definitions from `migration.rs`
5. **Map statuses** — New→New, Researching→Scoping, Scoped→Ready, Ready→Ready, InProgress→InProgress, Done→Done, Blocked→Blocked (with `blocked_from_status` also mapped)
6. **Map phases** — `WorkflowPhase` variant → equivalent string name (e.g., `Prd` → `"prd"`)
7. **Add new fields** — `pipeline_type: Some("feature")`, `phase_pool` based on status, `description: None`, `last_phase_commit: None`
8. **Bump version** — `schema_version: 2`
9. **Atomic write** — Write to temp file, rename over original (existing pattern)
10. **Return v2 data** — Continue with migrated backlog

**Edge cases:**
- Empty backlog (no items) → bump version, write empty v2 format
- Items in Blocked status → `blocked_from_status` also mapped through the status table
- Stale v1 PhaseResult files (reference `WorkflowPhase` enum values) → parse as strings naturally (YAML doesn't care about enum vs string)
- `Researching` items with default pipeline (empty `pre_phases`) → migrate to `Scoping` with `phase: None`, `phase_pool: None`. On next `orchestrate run`, the scheduler detects a Scoping item with no phase and triggers guardrail check → auto-promote to Ready (since there are no pre_phases to execute)

#### Flow: Triage Execution

> Assigns pipeline type and initial scores to a New item. Hardcoded orchestrator logic, not config-driven.

1. **Select New item** — Scheduler includes `Triage(item_id)` in actions (lowest priority after InProgress and Scoping)
2. **Build triage prompt** — Context preamble (item title, description, available pipeline types from config keys) + triage instructions. No skill command — triage logic is embedded in the prompt.
3. **Spawn triage agent** — Uses a `max_concurrent` slot (non-destructive). Agent returns a `PhaseResult` with `pipeline_type` set and `updated_assessments` for size/risk/impact scores.
4. **Validate pipeline_type** — If the agent's `pipeline_type` is not a key in `PipelineConfigs`, block the item with reason "invalid pipeline_type: X, valid types: [...]".
5. **Apply triage result** — Coordinator receives `UpdateItem` commands: `SetPipelineType`, `UpdateAssessments`, `TransitionStatus(Scoping)`, `SetPhase(first_pre_phase, PhasePool::Pre)`.
6. **Handle empty pre_phases** — If the assigned pipeline has no `pre_phases`, skip directly to guardrail check → Ready (or Blocked if guardrails fail).

**Edge cases:**
- Agent returns no `pipeline_type` → block item with "triage did not assign pipeline_type"
- Agent returns `SubphaseComplete` or `Failed` → item stays `New`, enters retry
- Human-supplied `--pipeline` hint → included in triage prompt as a hint the agent can accept or override

#### Flow: Archive Completed Items

> Removes Done items from the backlog and records them in the worklog.

1. **Item reaches Done** — After the last main phase completes, `resolve_transition()` returns `TransitionStatus(Done)`
2. **Coordinator processes Done** — Updates item status
3. **Scheduler detects Done item** — On next snapshot, triggers `ArchiveItem` coordinator command
4. **Coordinator archives** — Removes item from backlog, writes worklog entry, saves backlog, commits

#### Flow: Item Lifecycle

> Complete lifecycle of a backlog item through the system.

```
orchestrate add "Build new feature"
        │
        ▼
      [New] ─── triage (hardcoded agent) ──→ assigns pipeline_type, scores
        │
        ▼
   [Scoping] ── pre_phases (research, etc.) ──→ refines scores, produces scoping artifacts
        │
        ▼ (guardrails pass)                     ▼ (guardrails fail)
    [Ready] ────────────────────────────────  [Blocked] (for human review)
        │
        ▼ (max_wip allows)
  [InProgress] ── phases (prd, design, build, etc.)
        │
        ▼ (all phases complete)
     [Done]

Any non-terminal state → [Blocked] (on retry exhaustion, staleness block, guardrail failure)
```

---

## Technical Decisions

### Key Decisions

#### Decision: Single Coordinator Actor (not Mutex-per-resource)

**Context:** Multiple concurrent tasks need serialized access to backlog (YAML file) and git index. These resources are coupled — completing a phase requires updating the backlog AND committing to git.

**Decision:** Single actor/coordinator owning both resources, communicating via bounded mpsc channels with oneshot replies.

**Rationale:** Eliminates deadlock by design (single consumer, no lock ordering). Cross-resource operations (update backlog + commit) are naturally atomic within a single message handler. The serial bottleneck is the design goal, not a limitation — the bottleneck is agent execution (minutes), not state operations (milliseconds).

**Consequences:** More boilerplate (message enum variants, handle methods) but simpler correctness reasoning. All 18 backlog save call sites and all git operations must route through the coordinator.

#### Decision: Loop-Driven Scheduler (not Event-Driven)

**Context:** The scheduler needs to make decisions after phase completions and handle multiple concurrent items.

**Decision:** Polling loop that requests snapshots, selects actions, spawns tasks, and awaits completions before looping.

**Rationale:** Simpler to reason about. Produces a consistent snapshot at each decision point. Easy to test (provide snapshot, assert selected actions). Event-driven would add complexity without performance benefit — phases take minutes, so milliseconds of scheduling latency is irrelevant.

**Consequences:** Small latency between phase completion and next scheduling decision (negligible). Snapshot copies backlog items on each iteration (acceptable for expected backlog sizes of 10-50 items).

#### Decision: Context Preamble Embedded in Prompt Text

**Context:** The orchestrator needs to pass structured context (item metadata, phase summaries, retry info) to agents before skill invocation. Options: file, stdin, CLI args, embedded in prompt.

**Decision:** Embed as structured markdown at the start of the prompt string, followed by the skill command. Delivered via `claude -p "<preamble>\n---\n<skill command>"`.

**Rationale:** Simplest mechanism — no additional I/O, no temp files, no special CLI flags. Claude naturally reads the preamble before processing the skill command. Context visible in logs for debugging.

**Consequences:** Context size limited by CLI argument length (practically not a concern for item metadata and phase summaries). Skills must tolerate a preamble prefix (they already process natural language).

#### Decision: Skill Autonomous Mode via Context Flag

**Context:** Skills invoked by the orchestrator must run without human interaction, but domain logic should not diverge between modes (PRD resolved decision).

**Decision:** Context preamble includes `Mode: autonomous`. Skills check this flag to skip interactive steps (interviews, preference prompts) and use sensible defaults.

**Rationale:** Least invasive approach. No skill duplication, no wrapper layers, no separate entrypoints. Skills already process natural language context; checking a mode flag is trivial. Domain logic stays identical.

**Consequences:** Skill authors must handle the autonomous flag for any interactive step. Skills that are inherently non-interactive require no changes.

#### Decision: Phased Async Migration (Signatures First, Concurrency Second)

**Context:** The entire codebase (~80 function signatures, ~187 tests) is synchronous. Async is a prerequisite for concurrent execution.

**Decision:** SPEC Phase 1 converts all signatures to `async fn` and sets up the tokio runtime while preserving strictly sequential behavior. Concurrency features are added in Phase 2+.

**Rationale:** De-risks migration by separating correctness (async compiles and runs identically to sync) from concurrency (tasks actually overlap). Each phase is independently testable. Well-validated pattern per tech research.

**Consequences:** Two large diffs (Phase 1 is ~260 mechanical changes across source + tests), but Phase 1 is low-risk because behavior doesn't change. Sequential correctness is verified before any concurrent code is introduced.

#### Decision: Process Registry with `std::sync::Mutex<HashSet<Pid>>`

**Context:** With concurrent subprocess spawning, the existing single-PGID tracking is insufficient. Need to track all active child process groups for clean shutdown.

**Decision:** Global registry of PGIDs using `Arc<std::sync::Mutex<HashSet<Pid>>>`. Signal handler remains an atomic flag setter. Async runtime polls flag and iterates registry for kills.

**Rationale:** Extends the existing pattern naturally. Uses `std::sync::Mutex` (not tokio's) because registry operations are fast (insert/remove PID, iterate for kill) with no I/O under the lock — no `.await` while held.

**Consequences:** `unsafe` required for `pre_exec`/`setpgid` (already exists in codebase). Unix-only (already a constraint). Existing `kill_process_group()` reused for each PGID.

#### Decision: CLI-Only Agent Invocation (Claude API Deferred)

**Context:** The tech research identified Claude CLI multi-instance bugs as the highest risk to concurrent execution. The Claude API avoids shared CLI state entirely.

**Decision:** v2 continues with CLI-only agent invocation. Claude API support is explicitly deferred to future work.

**Rationale:** Adding API support requires a second `AgentRunner` implementation, API key management, token tracking, and a different output collection mechanism. The complexity is significant and orthogonal to the pipeline engine redesign. If CLI concurrency issues prove blocking in practice, API support can be added as a new `AgentRunner` implementation without architectural changes (the trait abstraction supports this).

**Consequences:** `max_concurrent > 1` remains risky with current Claude CLI. Users who need reliable concurrency must wait for either CLI fixes or API support in a future version.

#### Decision: `spawn_blocking` for Git Operations in Coordinator

**Context:** Git operations (subprocess calls) are synchronous and take 10-100ms. The coordinator runs on the async runtime.

**Decision:** Coordinator wraps git calls in `tokio::spawn_blocking` to avoid blocking the async runtime.

**Rationale:** Git operations exceed the 10-100μs threshold for blocking (per Alice Ryhl's guidelines). `spawn_blocking` offloads to a dedicated thread pool. The coordinator awaits the result without blocking other tasks.

**Consequences:** Slightly more verbose code in coordinator. No functional difference — operations are still serialized because the coordinator processes one command at a time.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Coordinator bottleneck | All state operations serialized through one actor | Deadlock-free, race-free shared state | Bottleneck is agent execution (minutes), not state ops (ms) |
| Async contagion | Every function signature changes | Concurrent phase execution capability | One-time migration cost; tokio is the standard async runtime |
| Loss of enum exhaustiveness | String phases lose compile-time checking | Configurable pipelines from TOML | Preflight validation catches config errors at startup |
| Large migration diff | ~260 locations change for WorkflowPhase removal | Arbitrary pipeline shapes without code changes | Mechanical change with high test coverage |
| `max_concurrent: 1` default | Sequential behavior out of the box | Safety against Claude CLI multi-instance bugs | Users opt into concurrency explicitly; known issues documented |
| Message boilerplate | ~10 coordinator command variants + handle methods | Type-safe, self-documenting coordinator API | Written once; compile errors catch misuse |
| Snapshot copying | Backlog items cloned on each scheduler iteration | Consistent point-in-time view for scheduling | Expected 10-50 items; clone cost is negligible |
| Destructive head-of-line blocking | ALL phases (including non-destructive on other items) blocked while a destructive phase runs | Simple, safe mutual exclusion — no risk of conflicting codebase modifications | v2 limitation: relaxing to "destructive exclusive to destructive only" is future work. Conservative choice is safer and easier to relax later. |

---

## Alternatives Considered

### Alternative 1: Mutex-Based Shared State

**Summary:** Use `Arc<tokio::sync::Mutex<BacklogFile>>` and `Arc<tokio::sync::Mutex<GitOps>>` instead of an actor/coordinator.

**How it would work:**
- Wrap BacklogFile in a tokio Mutex, clone Arc to each executor task
- Wrap git operations in a separate tokio Mutex
- Tasks lock the appropriate mutex, perform operations, release

**Pros:**
- Less boilerplate (no message types, no handle struct)
- More familiar pattern for developers new to actors
- Direct function calls instead of message passing

**Cons:**
- Deadlock risk: cross-resource operations (update backlog + git commit) require locking both mutexes. Lock ordering discipline required to avoid deadlock. One mistake = production deadlock.
- No natural backpressure: tasks pile up waiting for locks with no visibility into queue depth
- `tokio::sync::Mutex` held across `.await` points (git subprocess calls) — holds the lock for the entire subprocess duration, blocking all other tasks wanting that resource
- Can't batch operations: each task locks and commits independently
- Harder to test: must mock both mutexes and verify no deadlock patterns

**Why not chosen:** The coupling between backlog and git makes separate mutexes risky. A single lock wrapping both would work but is equivalent to the coordinator pattern with more footguns and less structure. The actor pattern is battle-tested (Alice Ryhl's guide) and a better fit for coupled resources.

### Alternative 2: Event-Driven Scheduler

**Summary:** Instead of a polling loop, use an event-driven model where phase completions trigger scheduling decisions via a channel.

**How it would work:**
- Phase completion events sent to a scheduler channel
- Scheduler wakes on events, makes decisions, spawns new tasks
- No periodic polling; purely reactive to completion events

**Pros:**
- Zero-latency response to completions
- More "async-native" feel
- No snapshot polling overhead

**Cons:**
- More complex state management: scheduler state evolves incrementally (apply delta from event) rather than being rebuilt from snapshots each iteration
- Harder to reason about ordering: events arrive asynchronously and can interleave in unexpected ways
- State consistency harder to maintain: must carefully handle event ordering and partial updates
- Race window between event arrival and state query for new scheduling decisions
- Harder to test: must simulate event sequences rather than provide a snapshot and assert actions

**Why not chosen:** The loop-driven approach is simpler and produces a consistent snapshot at each decision point, making scheduling logic deterministic and easy to unit test (input snapshot → output actions). The performance difference is immaterial — phases take minutes to execute, so even 100ms of scheduling latency is invisible.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Concurrent Claude CLI instances unstable | Phases crash, lock contention, corrupt output when `max_concurrent > 1` | High (documented issues: #4014, #13499, #14124) | Default `max_concurrent: 1`. Log clear warnings when users set > 1. Surface CLI errors distinctly from phase logic errors. |
| Async migration introduces regressions | Tests pass but runtime behavior subtly changes | Medium | Phase 1 preserves sequential behavior — verify all 187 tests pass unchanged. Phase 2 adds concurrency with new tests specifically for concurrent behavior. |
| Coordinator becomes debugging bottleneck | Hard to trace which operation caused a state inconsistency | Low | Log every command received + processed with item ID, phase name, and result. Coordinator is the single source of truth — bugs are localized. |
| WorkflowPhase→string migration breaks YAML parsing | Existing BACKLOG.yaml files fail to load | Medium | V1 struct preserved in migration module. Comprehensive migration tests with real fixture files from existing projects. |
| Channel deadlock from dropped oneshot sender | Coordinator sends reply but receiver was dropped (task cancelled) | Low | Coordinator handles `send` errors gracefully (log and continue). Cancellation drops the receiver; coordinator's next `recv()` processes the next command. |
| Orphaned child processes after shutdown | Claude CLI processes consume resources after orchestrator exits | Low | `kill_on_drop(true)` as safety net on all children. PGID registry + SIGTERM/SIGKILL escalation covers the normal path. |
| Stale result files from v1 runs cause parse errors | Phase results referencing `WorkflowPhase` enum values fail to parse as new format | Low | PhaseResult phase field changes from enum to string. YAML serialization of enum variants produces strings (e.g., `phase: prd`), which parse identically as `String`. No actual format change needed. |

---

## Integration Points

### Existing Code Touchpoints

- `src/types.rs` — Remove `WorkflowPhase` enum + all 66 source-file references. Add 4 new fields to `BacklogItem`. Change `PhaseResult.phase` type.
- `src/pipeline.rs` — Refactored into `scheduler.rs` + `executor.rs`. Original file removed or reduced to re-exports during transition.
- `src/backlog.rs` — Save operations routed through coordinator at runtime. Load logic extended for migration detection. `phase_advancement` functions updated for string-based phases with config lookup.
- `src/agent.rs` — Trait becomes async. Both implementations updated. Process registry added.
- `src/config.rs` — `PipelineConfig`, `PhaseConfig`, `StalenessAction` structs added. `validate()` function for cross-field rules. `max_wip`/`max_concurrent` in `ExecutionConfig`.
- `src/prompt.rs` — Hardcoded phase→skill mapping replaced with config lookup. Context preamble builder added.
- `src/git.rs` — `get_head_sha()` and `is_ancestor()` functions added.
- `src/main.rs` — `#[tokio::main]`, async command handlers, `--description` flag on `add`, `init` writes default pipelines to TOML, `advance` validates against pipeline config.
- `src/worklog.rs` — Log writes routed through coordinator.

### External Dependencies

| Dependency | Status | Purpose |
|------------|--------|---------|
| `tokio` | New | Async runtime, subprocess management, channels, timers |
| `tokio-util` | New | `CancellationToken`, `TaskTracker` for structured concurrency |
| `async-trait` | New (maybe unnecessary) | `#[async_trait]` for `AgentRunner` trait. Note: Rust >= 1.75 supports native `async fn` in traits. If `dyn AgentRunner` object safety is needed, `async-trait` or manual `Box<dyn Future>` return is still required. Evaluate during implementation — may be unnecessary. |
| `nix` | Existing | POSIX signal/process operations (SIGTERM, SIGKILL, setpgid) |
| `toml` | Existing | Config parsing (extended for pipeline tables) |
| `serde` / `serde_yaml` | Existing | Serialization for config and backlog |
| `signal-hook` | Existing | Signal handler registration |

**Removed dependency:**
- `wait-timeout` — Replaced by `tokio::time::timeout`

---

## Open Questions

- [ ] **Bounded channel capacity tuning:** Starting with 32 based on expected `max_concurrent` ranges. May need adjustment if real workloads show backpressure issues. Deferred to implementation — easy to change.
- [x] **Skill probe timeout:** ~~Resolved:~~ Single probe agent with 60-second hardcoded timeout (not tied to `phase_timeout_minutes`).
- [ ] **`pipeline.rs` removal strategy:** Delete and replace with `scheduler.rs` + `executor.rs` in one phase, or incrementally extract? One-shot replacement is cleaner for the final state but harder to review. Incremental is safer for git blame but creates messy intermediate commits.
- [ ] **Relaxed destructive exclusion (future):** v2 blocks all phases during destructive execution. Future work could allow non-destructive phases to overlap with destructive phases (only destructive-to-destructive exclusion).

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD must-have requirements
- [x] Key flows are documented (orchestration loop, phase execution, preflight, staleness, shutdown, migration, item lifecycle)
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] Alternatives considered with rationale for rejection
- [x] Open questions are minor and can be resolved during spec phase

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-11 | Initial design draft | Architecture: coordinator + scheduler + executor. Medium mode: 2 alternatives evaluated (mutex-based state, event-driven scheduler). 7 key decisions documented. |
| 2026-02-11 | Self-critique + auto-fix | 7 critic agents found 94 issues. Auto-fixed ~20 items: added triage flow, SubphaseComplete, circuit breaker, ItemUpdate/PhasePool/SchedulerAction types, run-tracking data structure, phase transition ownership, coordinator commands (worklog/archive/followups/unblock), fixed select_actions priority ordering, staleness:block preflight check, migration edge case, Claude API deferral decision, CLI flag coverage. 3 directional items + quality items presented for review. |
| 2026-02-11 | Directional decisions resolved | (1) Skill probes: single agent with full skill list instead of one-per-skill. (2) Keep full async architecture as designed. (3) Destructive exclusion: keep strict (block all), document as v2 limitation with relaxation as future work. |
| 2026-02-11 | Design finalized | Self-critique complete. ~20 auto-fixes applied, 3 directional decisions resolved with human input, 9 quality items triaged as SPEC-level or already addressed. All review checklist items satisfied. Status → Complete. |
