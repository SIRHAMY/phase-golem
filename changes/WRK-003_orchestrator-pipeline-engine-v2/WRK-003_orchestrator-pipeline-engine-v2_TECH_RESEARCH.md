# Tech Research: Orchestrator Pipeline Engine v2

**ID:** WRK-003
**Status:** Complete
**Created:** 2026-02-11
**PRD:** ./WRK-003_orchestrator-pipeline-engine-v2_PRD.md
**Mode:** Medium

## Overview

Researching the technical landscape for migrating a synchronous Rust orchestrator to an async, concurrent pipeline engine. Key areas: tokio async migration patterns, concurrent subprocess management, serialized coordinator patterns for shared state, configurable pipeline schemas, advance-furthest-first scheduling, git staleness detection, and concurrent Claude CLI considerations.

## Research Questions

- [x] What are established patterns for migrating a synchronous Rust codebase to tokio async?
- [x] How should concurrent subprocess management work with tokio (process groups, SIGTERM/SIGKILL, graceful shutdown)?
- [x] What coordinator patterns work best for serialized access to shared mutable state (file writes, git operations) in async Rust?
- [x] What TOML schema patterns exist for defining configurable pipelines/workflows?
- [x] How do advance-furthest-first / drain-toward-completion schedulers compare to alternatives?
- [x] How does git commit ancestry checking work for staleness detection?
- [x] Are there known issues with running multiple concurrent Claude CLI instances?

---

## External Research

### Landscape Overview

The PRD touches seven distinct technical domains. Most are well-established with mature patterns (async Rust, subprocess management, pipeline schemas, scheduling, git ancestry). One area — concurrent Claude CLI instances — is a significant risk with multiple documented issues. The overall technical approach is sound and follows established patterns from CI/CD systems (GitLab, GitHub Actions) and async Rust best practices (tokio patterns, actor model).

### Common Patterns & Approaches

#### Pattern: Phased Async Migration (Sync → Tokio)

**How it works:** Convert all function signatures to `async fn` in one pass (Phase 1, preserving sequential behavior), then add concurrency features (Phase 2). Use `#[tokio::main]` at entry point. Bridge sync/async during migration with `block_on` (sync calling async) and `spawn_blocking` (async calling sync).

**When to use:** When the codebase is moderate size (~47 functions) and async is a prerequisite for new features.

**Tradeoffs:**
- Pro: De-risks migration by separating correctness (async signatures) from concurrency (spawning tasks)
- Pro: Each phase is independently testable
- Con: Large diff in Phase 1 (~260 locations including tests), but mechanical
- Con: Async contagion means all callers must change too

**Common technologies:** `tokio` (runtime), `tokio::process::Command` (replaces `std::process::Command`), `tokio::time::timeout` (replaces `wait-timeout` crate)

**References:**
- [Tokio: Bridging with Sync Code](https://tokio.rs/tokio/topics/bridging) — Official sync/async bridging patterns
- [Greptime: Bridge Async and Sync in Rust](https://greptime.com/blogs/2023-03-09-bridging-async-and-sync-rust) — Real-world experience report
- [Alice Ryhl: Async - What is Blocking?](https://ryhl.io/blog/async-what-is-blocking/) — The 10-100 microsecond rule for blocking detection
- [Tokio: Unit Testing](https://tokio.rs/tokio/topics/testing) — `#[tokio::test]` patterns

#### Pattern: Actor/Coordinator with mpsc Channels

**How it works:** A single "coordinator" task owns all mutable state (BacklogFile, git index). Other tasks send command messages via `mpsc::Sender<Command>`. The coordinator receives messages, performs operations, and replies via `oneshot::Sender<Result>`. Only the coordinator ever touches the state.

**When to use:** When multiple async tasks need serialized access to resources that require I/O (file writes, subprocess calls). When you need to avoid deadlock from multiple mutexes.

**Tradeoffs:**
- Pro: Eliminates deadlock risk (single consumer, no lock ordering issues)
- Pro: Natural backpressure via bounded channels
- Pro: Clean shutdown when all senders drop
- Con: More boilerplate (message types, handle structs)
- Con: Coordinator becomes a serial bottleneck (but that's the explicit design goal here)

**Common technologies:** `tokio::sync::mpsc`, `tokio::sync::oneshot`

**References:**
- [Alice Ryhl: Actors with Tokio](https://ryhl.io/blog/actors-with-tokio/) — Definitive guide by a tokio maintainer
- [Tokio: Shared State Tutorial](https://tokio.rs/tokio/tutorial/shared-state) — Mutex vs channels decision guide
- [Tokio: Channels Tutorial](https://tokio.rs/tokio/tutorial/channels) — mpsc and oneshot patterns

#### Pattern: CancellationToken + TaskTracker for Subprocess Lifecycle

**How it works:** Create a `CancellationToken` for shutdown signaling and `TaskTracker` to track spawned tasks. Each subprocess task clones the token and `select!`s between `child.wait()` and `token.cancelled()`. On shutdown: cancel token, `tracker.close()` + `tracker.wait()`.

**When to use:** Structured concurrency with clean lifecycle management for multiple child processes.

**Tradeoffs:**
- Pro: Idiomatic tokio, composable, guarantees all tasks complete
- Pro: Child tokens allow hierarchical cancellation
- Con: Requires `tokio-util` crate
- Con: Need separate PGID tracking for killing child process trees (not just tasks)

**Common technologies:** `tokio-util` (CancellationToken, TaskTracker), `nix` crate (SIGTERM/SIGKILL)

**References:**
- [Tokio: Graceful Shutdown](https://tokio.rs/tokio/topics/shutdown) — Official CancellationToken + TaskTracker guide
- [Rust Tokio Task Cancellation Patterns](https://cybernetist.com/2024/04/19/rust-tokio-task-cancellation-patterns/) — Pattern comparison

#### Pattern: Process Group (PGID) Tracking with Signal Escalation

**How it works:** Set each child as process group leader via `pre_exec(|| { libc::setpgid(0, 0); Ok(()) })`. Track PGIDs in `Arc<Mutex<HashSet<Pid>>>`. On shutdown: SIGTERM all PGIDs (`kill(-pgid, SIGTERM)`), wait 5s, SIGKILL survivors.

**When to use:** When child processes spawn their own children (as Claude CLI does) and you need to kill the entire process tree.

**Tradeoffs:**
- Pro: Kills entire process tree, not just direct child
- Pro: Already partially implemented in the codebase (`pre_exec`/`setpgid` in `CliAgentRunner`)
- Con: Requires `unsafe` for `pre_exec`
- Con: Unix-only (not portable to Windows)

**References:**
- [Tokio process module source](https://github.com/tokio-rs/tokio/blob/master/tokio/src/process/mod.rs) — tokio subprocess implementation
- [tokio-graceful-shutdown crate](https://github.com/Finomnis/tokio-graceful-shutdown) — High-level shutdown management

#### Pattern: Git Staleness via `merge-base --is-ancestor`

**How it works:** `git merge-base --is-ancestor <commit> HEAD` exits 0 if the commit is reachable from HEAD, 1 if not. Exit 128 = unknown commit (treat as stale).

**When to use:** Before destructive phases, to check if prior phase artifacts are based on code still in the current history.

**Tradeoffs:**
- Pro: Fast (uses commit graph), simple, correct
- Pro: Handles rebase detection automatically (rebased commits are no longer ancestors)
- Con: Must store full 40-character SHAs (abbreviated SHAs can become ambiguous)

**References:**
- [git-merge-base Documentation](https://git-scm.com/docs/git-merge-base) — Official docs for `--is-ancestor`

#### Pattern: Linear Phase Sequence in TOML

**How it works:** Phases defined as ordered array of inline tables with `name`, `skills`, `destructive` fields. Array order = execution order. No DAG.

**When to use:** When pipelines are inherently sequential and variation is which phases exist.

**Tradeoffs:**
- Pro: Very simple to understand, validate, and implement
- Pro: No cycle detection needed, no DAG complexity
- Con: No conditional skipping or branching (explicitly out of scope per PRD)

**References:**
- [GitLab CI: Pipeline Architectures](https://docs.gitlab.com/ci/pipelines/pipeline_architectures/) — Linear stage pattern
- [GitHub Actions: Workflow Syntax](https://docs.github.com/en/actions/reference/workflow-syntax-for-github-actions) — DAG model for comparison

### Technologies & Tools

#### Async Runtime & Concurrency

| Technology | Purpose | Pros | Cons | Used With Patterns |
|------------|---------|------|------|-------------------|
| [tokio](https://tokio.rs/) | Async runtime | Standard, well-supported, subprocess support | Async contagion, learning curve | All async patterns |
| [tokio-util](https://docs.rs/tokio-util) | CancellationToken, TaskTracker | Structured shutdown, task lifecycle | Extra dependency | Subprocess lifecycle |
| [nix](https://docs.rs/nix) | POSIX signal/process ops | Type-safe, already a dependency | Unix-only | Process group management |

#### Config & Validation

| Technology | Purpose | Pros | Cons | Used With Patterns |
|------------|---------|------|------|-------------------|
| [toml](https://docs.rs/toml) | TOML parsing | Already a dependency, mature | N/A | Pipeline config |
| serde `#[derive(Deserialize)]` + custom `validate()` | Schema validation | Type-safe parsing + semantic rules | Manual validation code | Pipeline config |

### Standards & Best Practices

1. **Async migration:** Separate async conversion (signatures, runtime) from concurrency features (spawning, channels). Validate sequential correctness before adding parallelism.
2. **Coordinator pattern:** Single actor owning both BACKLOG.yaml and git operations. Do not use separate mutexes for each resource — eliminates deadlock by design.
3. **Process groups:** Each child must be its own process group leader. Never use the orchestrator's own PGID for `kill(-pgid)`.
4. **Staleness:** `git merge-base --is-ancestor` with full 40-char SHAs. Treat exit code 128 (unknown commit) as stale.
5. **Scheduling:** Simple priority sort on `(status_rank, phase_index, creation_date)` is sufficient. No formal priority aging needed — WIP limits prevent starvation.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Blocking the async runtime | Sync operations >10-100μs between `.await` points starve other tasks | Use `spawn_blocking` for sync I/O, `tokio::process` for subprocesses |
| `block_on` inside async context | Panics ("Cannot start a runtime from within a runtime") | Only use at sync/async boundary (main entry point) |
| `std::sync::Mutex` across `.await` | Guard is not `Send` — compilation error | Use `tokio::sync::Mutex` for any lock held across await points |
| `#[tokio::test]` default single-threaded | Tasks expecting multi-threaded progress will hang | Use `#[tokio::test(flavor = "multi_thread")]` when needed |
| Dropping `Child` without awaiting | Process continues as orphan, not killed | Always `kill()` or set `kill_on_drop(true)` |
| Unbounded channels | No backpressure, memory grows if coordinator is slow | Always use bounded `mpsc::channel(capacity)` |
| Multiple Claude CLI instances | Shell state corruption, SQLite lock contention, API rate limit exhaustion | Default `max_concurrent: 1`; see Critical Areas |

### Key Learnings

- The phased async migration approach (signatures first, concurrency second) is well-validated and matches the PRD's recommendation
- Actor/coordinator with mpsc channels is the battle-tested solution for serializing shared state — superior to multiple mutexes
- Process group management with PGID tracking is already partially implemented and just needs to scale to a registry
- The TOML pipeline schema design is solid — serde + custom validation is the right approach (no need for heavy schema crates)
- Git ancestry checking is a simple, efficient primitive that directly solves staleness detection
- Concurrent Claude CLI is the highest-risk area with multiple open bugs

---

## Internal Research

### Existing Codebase State

The orchestrator is a single Rust binary (~1,600 lines of source across 9 files, ~187 tests across 6 test files). It is **fully synchronous** — 0 async functions across ~80 function signatures. All state management flows through mutable references (`&mut BacklogFile`), all subprocess spawning uses `std::process::Command`, and all file I/O is blocking.

**Relevant files/modules:**

| File | Lines | Purpose | WRK-003 Impact |
|------|-------|---------|----------------|
| `src/types.rs` | ~200 | Data types: `BacklogItem`, `WorkflowPhase`, `PhaseResult`, `ItemStatus` | Heavy — enum→string migration, new fields |
| `src/pipeline.rs` | ~1,200 | Pipeline execution, item selection, retry, commit | Heavy — concurrency, scheduler, coordinator |
| `src/backlog.rs` | ~360 | BACKLOG.yaml load/save, phase advancement | Heavy — coordinator routing, schema migration |
| `src/prompt.rs` | ~320 | Prompt building for agents | Medium — config-driven skill lookup |
| `src/agent.rs` | ~280 | AgentRunner trait, CLI/Mock implementations, signal handling | Medium — async, process registry |
| `src/main.rs` | ~400 | CLI commands (init, add, run, triage, status, advance, unblock) | Medium — async, pipeline config |
| `src/config.rs` | ~76 | Config loading (orchestrate.toml) | Medium — pipeline config structs |
| `src/git.rs` | ~153 | Git operations (status, stage, commit) | Light — add `get_head_sha`, `is_ancestor` |
| `src/worklog.rs` | ~50 | Work logging | Light — async, coordinator routing |
| `src/lock.rs` | ~90 | Lock file management | Unchanged |

**Existing patterns in use:**
- `WorkflowPhase` enum with exhaustive matching everywhere (66 source + 194 test locations)
- Mutable reference passing (`&mut BacklogFile` threaded through call chain)
- Atomic write-temp-rename for BACKLOG.yaml saves
- Single-process signal handling (AtomicBool flag + single PGID kill)
- `MockAgentRunner` with `std::sync::Mutex<Vec<Result>>` for testing

### WorkflowPhase Migration Scope

**66 occurrences across 5 source files, 194 across 6 test files = ~260 total locations.**

The PRD estimates ~90 changes — this undercounts test files significantly. The migration is mechanical (enum variant → string literal) but touches every file.

Key migration areas:
- `WorkflowPhase::next()` → pipeline config lookup
- `WorkflowPhase::as_str()` → the string IS the phase
- `phases_between()` → pipeline config range query
- `artifact_filename_for_phase()` → string-based filename
- `build_skill_invocation()` → config-driven skill mapping (currently hardcoded phase→skill)
- `parse_workflow_phase()` in main.rs → validate against pipeline config

### AgentRunner Trait (Async Migration Target)

```rust
pub trait AgentRunner {
    fn run_agent(&self, prompt: &str, result_path: &Path, timeout: Duration)
        -> Result<PhaseResult, String>;
}
```

Two implementations:
1. **`CliAgentRunner`** — spawns `claude --dangerously-skip-permissions -p "<prompt>"`, uses `std::process::Command`, `wait-timeout` crate, `pre_exec`/`setpgid` for process groups
2. **`MockAgentRunner`** — `std::sync::Mutex<Vec<Result>>`, pops results LIFO (reversed on construction for FIFO)

Async migration: trait method becomes `async fn`, `std::process::Command` → `tokio::process::Command`, `wait-timeout` → `tokio::time::timeout`, `std::sync::Mutex` → `tokio::sync::Mutex`.

### BacklogItem — Current vs Required

Current fields (17):
```
id, title, status, phase, size, complexity, risk, impact,
requires_human_review, origin, blocked_from_status, blocked_reason,
blocked_type, unblock_context, tags, dependencies, created, updated
```

New fields for WRK-003 (4):
- `pipeline_type: Option<String>` — which pipeline config (default: `"feature"`)
- `description: Option<String>` — free-form user description
- `phase_pool: Option<String>` — `"pre"` or `"main"`
- `last_phase_commit: Option<String>` — HEAD SHA for staleness detection

Changed types:
- `phase: Option<WorkflowPhase>` → `Option<String>`
- `status: ItemStatus` — `Researching` → `Scoping`, `Scoped` removed

### Backlog Save Call Sites — 18 Total

14 in `pipeline.rs` (status transitions, phase results, blocks, retries), 4 in `main.rs` (init, add, advance, unblock). All follow: `find_item_mut()` → mutate fields → `backlog::save()`. The load-mutate-save sequence is not atomic — the PRD's coordinator pattern serializes this.

### Current Git Operations

5 functions in `git.rs`: `is_git_repo`, `check_preconditions`, `stage_paths`, `commit`, `get_status`. All synchronous, all used by `commit_checkpoint()` in `pipeline.rs`.

**Missing for WRK-003:** `get_head_sha()` and `is_ancestor(sha)`.

### Current Signal Handling

- Global `AtomicBool` shutdown flag via `OnceLock<Arc<AtomicBool>>`
- SIGTERM/SIGINT handlers via `signal_hook::flag::register()`
- `kill_process_group(pgid)`: SIGTERM → poll 5s → SIGKILL
- Only tracks a **single** child process — needs registry for concurrent spawns

### Pipeline Scheduling — Current `select_item()`

1. If `--target` specified, return that item
2. Find first `InProgress` item (crash recovery)
3. Find highest-impact `Ready` item, FIFO tiebreak

For WRK-003: must change to advance-furthest-first, select multiple items up to `max_concurrent`/`max_wip`, and handle `Scoping` vs `InProgress` priority.

### Config — Current Structure

```rust
pub struct OrchestrateConfig {
    pub project: ProjectConfig,       // prefix: String
    pub guardrails: GuardrailsConfig, // max_size, max_complexity, max_risk
    pub execution: ExecutionConfig,   // phase_timeout_minutes, max_retries, default_phase_cap
}
```

For WRK-003: add `pipelines: HashMap<String, PipelineConfig>` and `max_wip`/`max_concurrent` to execution config. The `toml` crate (already a dependency) supports the required nested table structures.

### Reusable Components

| Component | Reusability | Notes |
|-----------|-------------|-------|
| `backlog::save()` atomic write pattern | Keep, wrap in coordinator | Write-temp-rename is correct |
| `kill_process_group()` | Keep as-is | SIGTERM/poll/SIGKILL pattern is correct |
| `shutdown_flag()` global singleton | Keep as-is | AtomicBool works in async |
| `commit_checkpoint()` logic | Refactor into coordinator message | Logic stays, access pattern changes |
| `GuardrailsConfig` + `passes_guardrails()` | Keep as-is | Used for auto-promotion |
| `LockGuard` / lock management | Keep as-is | Single-instance guarantee |
| Config loading pattern | Extend | Add `[pipelines]` deserialization |
| `slugify()`, `generate_next_id()` | Keep as-is | Utility functions |

### Constraints from Existing Code

- **Mutable reference threading:** `&mut BacklogFile` passes through the entire call chain — incompatible with concurrency. Must move to coordinator ownership.
- **187 tests all synchronous:** Every test needs `#[tokio::test]` annotation and updated mock setup.
- **`wait-timeout` crate:** Only works with `std::process::Child`. Replaced by `tokio::time::timeout`.
- **`pipeline.rs` is 1,200 lines:** The largest file and heaviest refactor target. May benefit from being split into scheduler + executor modules.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| ~90 changes for WorkflowPhase→string migration | ~260 locations across source + tests (66 source, 194 test) | Migration is ~3x larger than estimated. Still mechanical, but needs to be planned as a significant phase. |
| `max_concurrent > 1` for parallel non-destructive phases | Multiple open GitHub issues document Claude CLI crashes, lock contention, state corruption, and CPU accumulation with concurrent instances (#4014, #13287, #13499, #14124, #18998) | `max_concurrent > 1` is technically risky with current Claude CLI. Default of 1 is correct. Users should be explicitly warned. Consider Claude API as alternative path for true concurrency. |
| Skill validation via agent probes | No established pattern for "probe" agents exists — this is novel | Probe implementation needs careful design: timeout handling, what constitutes "can see and read" vs partial access, how to report partial failures |
| Pre-phases run during `Scoping` status | The status transition `Researching → Scoping` is a rename but `Scoped` is removed entirely | Need to verify no external tools/scripts depend on the `Scoped` status name |
| Concurrent scheduling with `max_wip` and `max_concurrent` | The current architecture passes `&mut BacklogFile` through the entire call chain — fundamentally incompatible with concurrency | The coordinator refactor is a prerequisite for any concurrency. Cannot be deferred or done incrementally. |
| Staleness config `staleness: block` is safe | With `max_wip > 1`, one item's commits can cascade-block all other items | PRD already flags this risk. Should default all phases to `ignore` and document that `block` is only safe with `max_wip: 1`. |

---

## Critical Areas

### Async Migration Scope (~40-50% of effort)

**Why it's critical:** Every subsequent feature (concurrency, coordinator, process registry) depends on the async foundation. If the migration introduces bugs, all downstream work is affected.

**Why it's easy to miss:** The PRD correctly identifies it as 40-50% of effort, but the test migration alone (~194 `WorkflowPhase` references + all tests needing `#[tokio::test]`) is a significant sub-effort that could be underestimated.

**What to watch for:**
- `std::sync::Mutex` vs `tokio::sync::Mutex` — any mutex held across `.await` must be tokio's
- `#[tokio::test]` default single-threaded — tests expecting concurrent task progress will hang
- `spawn_blocking` for any remaining sync I/O in the hot path
- The `wait-timeout` crate removal — `tokio::time::timeout` wrapping `child.wait()` is the replacement

### Coordinator as Architecture Prerequisite

**Why it's critical:** The current `&mut BacklogFile` threading pattern is fundamentally incompatible with concurrent task execution. The coordinator must be in place before any concurrent scheduling can work.

**Why it's easy to miss:** It might seem like "just" serializing writes, but it changes the ownership model of the entire pipeline execution path. Every call site that currently takes `&mut BacklogFile` must be refactored to send a message instead.

**What to watch for:**
- 18 backlog save call sites all need routing through coordinator
- `commit_checkpoint()` also routes through coordinator (combines backlog + git)
- Message type design: batch vs individual operations, error propagation via oneshot
- Bounded channel capacity: too small = backpressure stalls, too large = memory growth

### Concurrent Claude CLI Instances

**Why it's critical:** The `max_concurrent > 1` feature — a key value proposition of the PRD — depends on Claude CLI behaving correctly with multiple simultaneous instances. Research shows this is **not currently reliable**.

**Why it's easy to miss:** The PRD treats concurrency as an orchestrator-level concern, but the subprocess (Claude CLI) has its own shared state conflicts that the orchestrator cannot control.

**What to watch for:**
- Shell state corruption (GitHub #4014)
- SQLite lock contention (GitHub #14124)
- `history.jsonl.lock` contention (GitHub #15334)
- API rate limit exhaustion with multiple agents
- CPU accumulation from zombie processes (#11122)
- Consider: should the orchestrator support Claude API direct calls as an alternative to CLI spawning?

### WorkflowPhase → String Migration Breadth

**Why it's critical:** Touches every source file and every test file. A mistake in the migration could break the BACKLOG.yaml format or agent output parsing.

**Why it's easy to miss:** The enum removal seems simple, but the ripple effects are wide: exhaustive match arms become string comparisons (losing compile-time safety), `phases_between()` loses its hardcoded array, `artifact_filename_for_phase()` needs config context, and the prompt builder's hardcoded skill invocations all move to config.

**What to watch for:**
- Loss of compile-time exhaustiveness checking (enum → string)
- Phase name typos in config becoming runtime errors instead of compile errors
- V1 result file parsing: old `"phase": "prd"` must parse into `String` (it will, naturally)
- Test fixtures need updating (YAML backlog fixtures reference old status/phase names)

---

## Deep Dives

_None yet._

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| Should the orchestrator support Claude API direct calls as alternative to CLI? | CLI has documented multi-instance bugs. API calls avoid shared state issues entirely. | CLI-only (simpler), API-only (more reliable for concurrency), configurable (most flexible but most complex). Defer to design. |
| How to structure skills for autonomous vs human modes? | Orchestrator needs autonomous execution; humans need interactive mode. Must share domain logic. | Core + wrapper, mode flag, separate entrypoints. PRD defers to design. |
| What bounded channel capacity for the coordinator? | Too small stalls concurrent tasks; too large wastes memory. | Start with 32-64 (well above expected max_concurrent), tune later. |
| Should `pipeline.rs` be split into scheduler + executor? | At 1,200 lines it's already the largest file; concurrency adds significant logic. | Split: scheduler (item selection, WIP management) + executor (phase running, retry) + coordinator (state management). Three concerns, three modules. |
| How to handle `#[tokio::test]` thread flavor? | Default is single-threaded; concurrent tests may need multi-threaded. | Default to `#[tokio::test]` (single-thread) for unit tests, `#[tokio::test(flavor = "multi_thread")]` for integration tests that test concurrent behavior. |

### Recommended Approaches

#### Async Migration Strategy

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Phased (signatures first, concurrency second) | De-risks migration, independently testable phases | Two large diffs, two review cycles | Codebase has many callers (this one) |
| Big-bang (everything at once) | Single diff, single review | Higher risk, harder to bisect failures | Small codebase (<20 functions) |

**Initial recommendation:** Phased migration (PRD already recommends this). Phase 1 converts all signatures + tests + runtime setup while preserving sequential behavior. Phase 2 adds spawning, channels, coordinator.

#### Coordinator Architecture

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Single actor (mpsc channels) | No deadlock, natural backpressure, clean shutdown | Boilerplate for message types | Multiple resources need serialized access (this case) |
| `tokio::sync::Mutex` per resource | Simpler code, less boilerplate | Deadlock risk with multiple mutexes, can't batch operations | Single resource, simple access pattern |
| Separate actors per resource | Independent scaling, separation of concerns | Cross-resource operations need coordination protocol | Resources are truly independent (not this case — backlog + git are coupled) |

**Initial recommendation:** Single actor/coordinator owning both BacklogFile and git operations. The actor pattern from Alice Ryhl's guide is battle-tested and fits the PRD's requirements exactly. Cross-resource atomicity (save backlog + commit) comes naturally.

#### Subprocess Lifecycle Management

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| CancellationToken + TaskTracker + PGID registry | Idiomatic, structured shutdown, process tree kill | Requires `tokio-util`, PGID needs `unsafe` | Multiple concurrent subprocesses (this case) |
| Manual task tracking + signal flag | Minimal dependencies | More error-prone, no structured guarantees | Simple cases with 1-2 processes |

**Initial recommendation:** CancellationToken + TaskTracker from `tokio-util`, combined with the existing PGID/setpgid pattern (already implemented) extended to a `HashSet<Pid>` registry. The signal handler remains atomic flag (already implemented), async runtime polls and iterates registry.

#### Pipeline Config Validation

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Serde deserialization + custom `validate()` | Type-safe parsing, clear error messages, full control | Manual validation code | Semantic cross-field rules needed (this case) |
| `serde_valid` annotations | Declarative, less code | Limited expressiveness for cross-field rules | Simple field-level validation |
| `schematic` crate | Env var support, layered config, schema generation | Heavy dependency for simple config | Complex config with merging/layering |

**Initial recommendation:** Serde deserialization + custom `validate()` function. The validation rules (unique phase names, at least one main phase, `destructive` rejected on pre_phases, `max_wip >= 1`) are cross-field semantic checks that annotation-based validation can't express well.

#### Staleness Detection

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `git merge-base --is-ancestor` | Fast, correct, handles rebase detection | Must serialize through coordinator | Checking if a commit is still in current history (this case) |
| Exact HEAD match | Simplest to implement | Too strict — any new commit invalidates | Not recommended |
| Reflog-based detection | Can distinguish "rebased" from "never existed" | Reflogs are local, expire after 90 days | Not needed for this use case |

**Initial recommendation:** `git merge-base --is-ancestor` with full 40-char SHAs. Exit code 0 = not stale, 1 = stale, 128 = unknown commit (treat as stale). Route through coordinator like all git operations.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Alice Ryhl: Actors with Tokio](https://ryhl.io/blog/actors-with-tokio/) | Guide | Definitive actor/coordinator pattern — directly applicable to coordinator design |
| [Tokio: Bridging with Sync Code](https://tokio.rs/tokio/topics/bridging) | Docs | Official sync/async migration patterns |
| [Tokio: Graceful Shutdown](https://tokio.rs/tokio/topics/shutdown) | Docs | CancellationToken + TaskTracker patterns |
| [Alice Ryhl: Async - What is Blocking?](https://ryhl.io/blog/async-what-is-blocking/) | Guide | The 10-100μs rule for identifying blocking code |
| [Tokio: Shared State](https://tokio.rs/tokio/tutorial/shared-state) | Tutorial | Mutex vs channels decision framework |
| [git merge-base docs](https://git-scm.com/docs/git-merge-base) | Docs | `--is-ancestor` for staleness detection |
| [GitLab CI: Pipeline Architectures](https://docs.gitlab.com/ci/pipelines/pipeline_architectures/) | Docs | Linear stage patterns for pipeline design |
| [GitHub #13499: Multiple CLI instances](https://github.com/anthropics/claude-code/issues/13499) | Issue | Claude CLI multi-instance conflict documentation |
| [GitHub #4014: Shell state corruption](https://github.com/anthropics/claude-code/issues/4014) | Issue | Root cause of concurrent CLI state corruption |
| [Tokio: Unit Testing](https://tokio.rs/tokio/topics/testing) | Docs | `#[tokio::test]` patterns and thread flavor selection |
| [WRK-002 Design Doc](../002_overhaul-changes-workflow-orchestrator/002_overhaul-changes-workflow-orchestrator_DESIGN.md) | Internal | Current architecture decisions and patterns |
| [WRK-002 SPEC](../002_overhaul-changes-workflow-orchestrator/002_overhaul-changes-workflow-orchestrator_SPEC.md) | Internal | Current implementation phases and task structure |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-11 | Initial research started | Launched external + internal research agents in parallel |
| 2026-02-11 | External research complete | 7 topics researched: async migration, subprocess mgmt, coordinator patterns, pipeline schemas, scheduling, git staleness, Claude CLI concurrency |
| 2026-02-11 | Internal research complete | Full codebase audit: ~260 WorkflowPhase locations, 18 backlog save sites, 187 tests, all source modules mapped |
| 2026-02-11 | PRD analysis complete | 6 concerns identified, 4 critical areas documented, recommendations synthesized |
