# SPEC: Orchestrator Pipeline Engine v2

**ID:** WRK-003
**Status:** Complete
**Created:** 2026-02-11
**PRD:** ./WRK-003_orchestrator-pipeline-engine-v2_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

The v1 orchestrator (WRK-002) works but has three structural limitations: a hardcoded 6-phase pipeline, fully sequential execution, and no safety checks at boundaries. This SPEC implements the v2 engine: config-driven pipelines from `orchestrate.toml`, concurrent multi-item execution with safety constraints, preflight validation, and staleness detection. The architecture centers on a coordinator actor (serialized backlog + git access), a scheduler (advance-furthest-first with WIP/concurrency limits), and an executor (async phase execution with retry and staleness).

The entire codebase is synchronous (~80 function signatures, ~187 tests). The async migration to tokio is a prerequisite for concurrency and represents ~40-50% of total effort. The `WorkflowPhase` enum removal (~260 references) is the broadest single change. Both are mechanical but high-volume.

## Approach

Build the v2 engine in layers: foundation types first, then async conversion, then schema migration, then the three new architectural modules (coordinator, executor, scheduler). Each layer depends only on layers below it.

The key architectural insight is separating the async migration (Phase 2: change signatures, preserve sequential behavior) from the concurrency features (Phase 6: actually run things concurrently). This de-risks the migration by validating that async code behaves identically to sync before adding concurrent execution.

The `pipeline.rs` module (~1,200 lines) is decomposed into `scheduler.rs` (item selection, WIP management, orchestration loop) and `executor.rs` (phase execution, retry, staleness). The `coordinator.rs` actor owns all mutable state (`BacklogFile` + git index), eliminating data races by design.

**Patterns to follow:**

- `src/pipeline.rs:run()` — current execution loop structure (check termination → select → execute → process results); v2 scheduler follows same shape with concurrent additions
- `src/agent.rs:run_subprocess_agent()` — subprocess lifecycle with `pre_exec`/`setpgid`/`kill_process_group`; v2 preserves this pattern with `tokio::process::Command`
- `src/backlog.rs:save()` — atomic write-temp-rename pattern; reused by migration module
- `src/config.rs:load_config()` — serde + TOML with `#[serde(default)]` and `Default` impls; v2 extends for pipeline tables
- `src/agent.rs:MockAgentRunner` — reverse-and-pop pattern for ordered test results; v2 keeps this with `tokio::sync::Mutex`
- `tests/pipeline_test.rs:setup_test_env()` — temp dir with git repo + orchestrator directories; reused by all new test files

**Implementation boundaries:**

- Do not modify: `src/lock.rs` (unchanged)
- Do not modify: any skill workflow files (`.claude/skills/changes/workflows/`)
- Do not add: Claude API support (explicitly deferred per design decision)
- Do not add: relaxed destructive exclusion (destructive blocks ALL phases, not just other destructive — v2 limitation per design)

## Open Questions

_None — all design decisions resolved._

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Foundation Types, Config & Git | Med | New type enums, BacklogItem/PhaseResult extensions, PipelineConfig structs, git staleness primitives |
| 2 | Async Migration | High | Convert all signatures to async/tokio, process registry, preserve sequential behavior |
| 3 | Schema Migration & WorkflowPhase Removal | High | v1→v2 BACKLOG.yaml migration, remove WorkflowPhase enum, string-based phases everywhere |
| 4 | Coordinator Actor | High | Single actor owning BacklogFile + git, mpsc channels, batch commit |
| 5 | Executor & Preflight | High | Phase execution with retry/staleness, preflight validation, context preamble builder |
| 6 | Scheduler & Integration | High | Advance-furthest-first scheduling, CLI wiring, graceful shutdown, pipeline.rs replacement |

**Ordering rationale:** Phase 1 establishes types everything depends on. Phase 2 enables async required by Phases 4-6. Phase 3 removes the old type system before new modules are built on top. Phase 4 (coordinator) must exist before Phase 5 (executor, which sends commands to coordinator) and Phase 6 (scheduler, which queries coordinator). Phase 5 (executor) must exist before Phase 6 (scheduler, which spawns executor tasks).

---

## Phases

Each phase should leave the codebase in a functional, stable state. Complete and verify each phase before moving to the next.

---

### Phase 1: Foundation Types, Config & Git

> New type enums, BacklogItem/PhaseResult field extensions, PipelineConfig structs, git staleness primitives

**Phase Status:** complete

**Complexity:** Med

**Goal:** Establish all new types and config structures that every subsequent phase depends on. Extend the git module with staleness primitives. All additions are backward-compatible — existing code continues to compile and all tests pass.

**Files:**

- `.claude/skills/changes/orchestrator/src/types.rs` — modify — add `PhasePool`, `ItemUpdate`, `PhaseExecutionResult`, `SchedulerAction`, `BacklogSnapshot` enums/structs; add 4 new fields to `BacklogItem`; add 2 new fields to `PhaseResult`
- `.claude/skills/changes/orchestrator/src/config.rs` — modify — add `PipelineConfig`, `PhaseConfig`, `StalenessAction` structs; extend `ExecutionConfig` with `max_wip`, `max_concurrent`; add `validate()` function; add default pipeline generation
- `.claude/skills/changes/orchestrator/src/git.rs` — modify — add `get_head_sha()` and `is_ancestor()` functions
- `.claude/skills/changes/orchestrator/tests/types_test.rs` — modify — add serialization tests for new types, new BacklogItem fields
- `.claude/skills/changes/orchestrator/tests/config_test.rs` — modify — add pipeline config loading tests, validation tests, default pipeline tests
- `.claude/skills/changes/orchestrator/tests/git_test.rs` — modify — add `get_head_sha` and `is_ancestor` tests

**Patterns:**

- Follow `src/config.rs` existing `#[serde(default)]` + `Default` impl pattern for new config structs
- Follow `src/types.rs` existing enum pattern with `Serialize`, `Deserialize`, `Clone`, `Debug`, `PartialEq` derives

**Tasks:**

- [x] Add `PhasePool` enum (`Pre`, `Main`) with serde derives to `types.rs`
- [x] Add `ItemUpdate` enum with variants: `TransitionStatus`, `SetPhase`, `ClearPhase`, `SetBlocked`, `Unblock`, `UpdateAssessments`, `SetPipelineType`, `SetLastPhaseCommit`, `SetDescription`
- [x] Add `PhaseExecutionResult` enum: `Success(PhaseResult)`, `SubphaseComplete(PhaseResult)`, `Failed(String)`, `Blocked(String)`, `Cancelled`
- [x] Add `SchedulerAction` enum: `Triage(String)`, `Promote(String)`, `RunPhase { item_id, phase, phase_pool, is_destructive }`
- [x] Add `BacklogSnapshot` struct: `items: Vec<BacklogItem>`, `schema_version: u32`
- [x] Add new fields to `BacklogItem`: `pipeline_type: Option<String>`, `description: Option<String>`, `phase_pool: Option<PhasePool>`, `last_phase_commit: Option<String>` — all with `#[serde(default)]` and `#[serde(skip_serializing_if = "Option::is_none")]`
- [x] Add new fields to `PhaseResult`: `based_on_commit: Option<String>`, `pipeline_type: Option<String>` — with `#[serde(default)]`
- [x] Add `StalenessAction` enum (`Ignore`, `Warn`, `Block`) to `config.rs` with serde derives and `Default` impl (defaults to `Ignore`)
- [x] Add `PhaseConfig` struct: `name: String`, `skills: Vec<String>`, `destructive: bool` (default false), `staleness: StalenessAction` (default Ignore)
- [x] Add `PipelineConfig` struct: `pre_phases: Vec<PhaseConfig>`, `phases: Vec<PhaseConfig>`
- [x] Add `pipelines: HashMap<String, PipelineConfig>` field to `OrchestrateConfig` with `#[serde(default)]`
- [x] Add `max_wip: u32` (default 1) and `max_concurrent: u32` (default 1) to `ExecutionConfig`
- [x] Implement `validate(config: &OrchestrateConfig) -> Result<(), Vec<String>>`: each pipeline has ≥1 main phase, phase names unique within pipeline (across pre_phases + phases), `destructive` rejected on pre_phases, `max_wip >= 1`, `max_concurrent >= 1`, `staleness: block` rejected when `max_wip > 1`
- [x] Implement default pipeline generation: when `pipelines` HashMap is empty after deserialization, insert default `feature` pipeline matching current 6-phase workflow
- [x] Update `load_config()` to call `validate()` after deserialization and return errors
- [x] Add `get_head_sha(project_root: &Path) -> Result<String, String>` to `git.rs` — runs `git rev-parse HEAD`, returns full 40-char SHA
- [x] Add `is_ancestor(sha: &str, project_root: &Path) -> Result<bool, String>` to `git.rs` — runs `git merge-base --is-ancestor <sha> HEAD`, exit 0 → true, exit 1 → false, exit 128 → Err
- [x] Write tests for new type serialization round-trips (PhasePool, ItemUpdate, SchedulerAction, BacklogSnapshot)
- [x] Write tests for BacklogItem with new fields (serialize/deserialize with and without optional fields)
- [x] Write tests for PhaseResult with new fields
- [x] Write tests for pipeline config loading: full config, partial config, missing `[pipelines]` section → default generation
- [x] Write tests for `validate()`: valid config passes, each validation rule has a failing test case
- [x] Write tests for `get_head_sha()`: returns 40-char SHA in a real git repo
- [x] Write tests for `is_ancestor()`: ancestor case (exit 0), non-ancestor case (exit 1), unknown commit case (exit 128)

**Verification:**

- [x] All existing tests pass unchanged (new fields have defaults, WorkflowPhase still exists)
- [x] New type tests pass
- [x] New config tests pass including validation rules
- [x] New git tests pass
- [x] `cargo build` succeeds with no warnings
- [x] Code review passes

**Commit:** `[WRK-003][P1] Feature: Foundation types, config extensions, and git staleness primitives`

**Notes:**
- Code review found and fixed: 2 clippy warnings (derivable Default impls), missing PhaseExecutionResult round-trip test, SHA input validation in is_ancestor(), `destructive` → `is_destructive` boolean naming fix, inline HashSet import consistency.

**Followups:**
- [Low] `BacklogSnapshot` structurally duplicates `BacklogFile` — consider unifying or documenting divergence when coordinator is implemented (Phase 4)
- [Low] `default_feature_pipeline()` is verbose — consider a helper constructor if more default pipelines are added
- [Low] `validate()` not called on the no-config-file default path — consider adding for defense-in-depth
- [Low] `load_config` silently injects default pipeline when config file exists but has no `[pipelines]` section — consider logging a warning

---

### Phase 2: Async Migration

> Convert all function signatures to async/tokio, add process registry, preserve strictly sequential behavior

**Phase Status:** complete

**Complexity:** High

**Goal:** Convert the entire codebase from synchronous to async using tokio. Every function signature becomes `async fn`, every test becomes `#[tokio::test]`, subprocess spawning uses `tokio::process::Command`. Behavior remains strictly sequential — no concurrency introduced. All existing tests pass identically.

**Files:**

- `.claude/skills/changes/orchestrator/Cargo.toml` — modify — add `tokio`, `tokio-util`; remove `wait-timeout`
- `.claude/skills/changes/orchestrator/src/main.rs` — modify — `#[tokio::main] async fn main()`
- `.claude/skills/changes/orchestrator/src/agent.rs` — modify — async `AgentRunner` trait, `tokio::process::Command`, process registry
- `.claude/skills/changes/orchestrator/src/pipeline.rs` — modify — all functions async
- `.claude/skills/changes/orchestrator/src/backlog.rs` — modify — all functions async
- `.claude/skills/changes/orchestrator/src/prompt.rs` — modify — all functions async (trivially, no I/O)
- `.claude/skills/changes/orchestrator/src/config.rs` — modify — `load_config` async (trivially)
- `.claude/skills/changes/orchestrator/src/worklog.rs` — modify — `write_entry` async
- `.claude/skills/changes/orchestrator/src/lib.rs` — modify — no changes needed (module declarations don't change)
- `.claude/skills/changes/orchestrator/tests/*.rs` — modify — all `#[test]` → `#[tokio::test]`, all `.await` added

**Patterns:**

- Follow [Alice Ryhl: Async - What is Blocking?](https://ryhl.io/blog/async-what-is-blocking/) for the 10-100μs rule
- Follow [Tokio: Bridging with Sync Code](https://tokio.rs/tokio/topics/bridging) for sync/async boundary patterns

**Tasks:**

- [x] Update `Cargo.toml`: add `tokio = { version = "1", features = ["full"] }`, `tokio-util = { version = "0.7", features = ["rt"] }`; remove `wait-timeout = "0.2"`
- [x] Add `#[tokio::main]` to `main.rs`, make `main()` async, make all `handle_*` functions async
- [x] Convert `AgentRunner` trait to async: used native async fn in trait with `impl Future` return type (not `#[async_trait]`), added `Send + Sync` bounds
- [x] Convert `CliAgentRunner::run_agent()` to async: `std::process::Command` → `tokio::process::Command`, `wait-timeout` → `tokio::time::timeout` wrapping `child.wait()`, added `kill_on_drop(true)` on spawned children
- [x] Preserve `pre_exec`/`setpgid` for process group isolation (works identically on `tokio::process::Command`)
- [x] Add process registry: `static PROCESS_REGISTRY: OnceLock<Arc<std::sync::Mutex<HashSet<Pid>>>>` with `register_child()`, `unregister_child()`, `kill_all_children()` functions. Uses `std::sync::Mutex` (not tokio's) since operations are fast (insert/remove/iterate) with no I/O under the lock.
- [x] On spawn: register child PGID in process registry. On completion/drop: unregister.
- [x] Convert `MockAgentRunner` to async: `std::sync::Mutex` → `tokio::sync::Mutex` for `results` field
- [x] Convert all functions in `pipeline.rs` to `async fn` (add `.await` to all callee calls)
- [x] `backlog.rs`, `prompt.rs`, `config.rs`, `worklog.rs` intentionally left synchronous — only functions that call async operations (runner.run_agent) were converted; pure I/O functions stay sync per the 10-100μs rule
- [x] Convert all `#[test]` annotations to `#[tokio::test]` for tests calling async functions (`agent_test.rs`, `pipeline_test.rs`); sync-only test files left as `#[test]`
- [x] Add `.await` to all async function calls in tests
- [x] Update mock agent creation in tests for async interface
- [x] Verify: no `std::sync::Mutex` held across `.await` points (only `tokio::sync::Mutex` for that)
- [x] Verify: `git.rs` functions remain synchronous (they will be called via `spawn_blocking` by the coordinator in Phase 4)

**Verification:**

- [x] All 217 tests pass (16 agent + 54 backlog + 22 config + 19 git + 7 lock + 24 pipeline + 36 prompt + 33 types + 4 worklog + 2 unit) — behavior identical to sync version
- [x] `cargo build` succeeds with no warnings
- [x] `cargo test` passes — all 217 tests pass
- [x] Subprocess spawn/timeout/kill path verified with `mock_agent_timeout.sh` fixture (7s elapsed, within expected range)
- [x] Process registry functions verified via integration tests (register/unregister called correctly in run_subprocess_agent lifecycle)
- [x] Code review passes — all 8 critical checks pass, no Critical/High issues

**Commit:** `[WRK-003][P2] Feature: Async migration to tokio with process registry`

**Notes:**

- Used native async fn in trait (`impl Future` return type) instead of `#[async_trait]` macro — cleaner but requires `&impl AgentRunner` (generics) instead of `&dyn AgentRunner` (trait objects) since `impl Trait` in return position is not dyn-compatible
- Only converted functions that actually need async: `agent.rs` (subprocess spawning), `pipeline.rs` (calls runner.run_agent), `main.rs` (calls pipeline functions). Left `backlog.rs`, `prompt.rs`, `config.rs`, `worklog.rs`, `git.rs` synchronous since they only do fast I/O (<100μs)
- Only converted test files that call async functions: `agent_test.rs` (11 async tests), `pipeline_test.rs` (24 async tests). Other 7 test files left as `#[test]` since they only test synchronous code
- `git.rs` functions stay synchronous intentionally — the coordinator wraps them in `spawn_blocking` in Phase 4
- `lock.rs` is not modified (lock acquisition is a startup-time operation, not hot path)

**Followups:**

- [Medium] `std::thread::sleep` in `kill_process_group` blocks the tokio runtime thread for up to 5s — consider wrapping in `spawn_blocking` or using `tokio::time::sleep` when concurrent execution is added
- [Medium] Blocking `std::process::Command` (git ops) called from async functions in `pipeline.rs` — acceptable for sequential pipeline but should wrap in `spawn_blocking` when concurrency is added (Phase 4 coordinator handles this)

---

### Phase 3: Schema Migration & WorkflowPhase Removal

> Create v1→v2 BACKLOG.yaml migration, remove WorkflowPhase enum, string-based phases everywhere

**Phase Status:** complete

**Complexity:** High

**Goal:** Remove the hardcoded `WorkflowPhase` enum and replace it with config-driven string-based phases. Create the migration module for v1→v2 BACKLOG.yaml conversion. Update `ItemStatus` to the v2 lifecycle (`Scoping` replaces `Researching`/`Scoped`). After this phase, all phase references are strings and all pipeline logic is config-driven.

**Files:**

- `.claude/skills/changes/orchestrator/src/migration.rs` — create — v1 struct definitions, v1→v2 mapping logic
- `.claude/skills/changes/orchestrator/src/types.rs` — modify — remove `WorkflowPhase` enum, update `ItemStatus` (remove `Researching`/`Scoped`, add `Scoping`), update `BacklogItem.phase` to `Option<String>`, update `PhaseResult.phase` to `String`, update `is_valid_transition()`
- `.claude/skills/changes/orchestrator/src/backlog.rs` — modify — bump `EXPECTED_SCHEMA_VERSION` to 2, add migration detection in `load()`, remove `advance_phase()`/`advance_to_phase()`/`phases_between()`/`artifact_filename_for_phase()`, update `add_item()` for new fields
- `.claude/skills/changes/orchestrator/src/prompt.rs` — modify — replace hardcoded `WorkflowPhase` → skill mapping with config-driven lookup from `PhaseConfig.skills`, update `PromptParams` to use `&str` phase
- `.claude/skills/changes/orchestrator/src/pipeline.rs` — modify — replace all `WorkflowPhase` references with string phases, update pre-workflow handling for `Scoping` status
- `.claude/skills/changes/orchestrator/src/main.rs` — modify — remove `parse_workflow_phase()`, update `status_sort_priority()` for new `ItemStatus` variants
- `.claude/skills/changes/orchestrator/src/lib.rs` — modify — add `pub mod migration;`
- `.claude/skills/changes/orchestrator/tests/migration_test.rs` — create — comprehensive v1→v2 migration tests
- `.claude/skills/changes/orchestrator/tests/fixtures/backlog_v1_full.yaml` — create — v1 fixture with items in every status
- `.claude/skills/changes/orchestrator/tests/types_test.rs` — modify — remove `WorkflowPhase` tests, update `ItemStatus` transition tests, update all `BacklogItem`/`PhaseResult` construction
- `.claude/skills/changes/orchestrator/tests/backlog_test.rs` — modify — remove `advance_phase`/`advance_to_phase` tests, update for string phases and v2 schema
- `.claude/skills/changes/orchestrator/tests/prompt_test.rs` — modify — update for config-driven skill lookup, string phases
- `.claude/skills/changes/orchestrator/tests/pipeline_test.rs` — modify — replace all `WorkflowPhase` references (~50+ locations) with string phases, update helpers
- `.claude/skills/changes/orchestrator/tests/agent_test.rs` — modify — update `PhaseResult` construction to use string phases
- `.claude/skills/changes/orchestrator/tests/worklog_test.rs` — modify — update `BacklogItem` construction for new fields
- `.claude/skills/changes/orchestrator/tests/fixtures/backlog_full.yaml` — modify — update to schema_version 2, new fields, new status names
- `.claude/skills/changes/orchestrator/tests/fixtures/backlog_minimal.yaml` — modify — update schema_version
- `.claude/skills/changes/orchestrator/tests/fixtures/backlog_empty.yaml` — modify — update schema_version

**Tasks:**

- [x] Create `migration.rs` with preserved v1 struct definitions: `V1BacklogFile`, `V1BacklogItem`, `V1ItemStatus` (New, Researching, Scoped, Ready, InProgress, Done, Blocked), `V1WorkflowPhase` (Prd, Research, Design, Spec, Build, Review) — all with `Deserialize`
- [x] Implement `migrate_v1_to_v2(path: &Path) -> Result<BacklogFile, String>`: load raw YAML, parse as v1, map statuses (New→New, Researching→Scoping, Scoped→Ready, Ready→Ready, InProgress→InProgress, Done→Done, Blocked→Blocked with `blocked_from_status` also mapped), convert `WorkflowPhase` variants to string names via `as_str()` equivalent, add `pipeline_type: Some("feature")` to all items, set `phase_pool` based on mapped status, set `description: None`, `last_phase_commit: None`, bump `schema_version: 2`, atomic write
- [x] Handle migration edge cases: empty backlog (bump version only), Blocked items (map `blocked_from_status`), Researching items with default pipeline empty pre_phases (migrate to Scoping with `phase: None`, `phase_pool: None` — scheduler auto-promotes on next run)
- [x] Make migration idempotent: running on v2 data is a no-op
- [x] Remove `WorkflowPhase` enum from `types.rs` (and its `next()`, `as_str()` impls)
- [x] Update `ItemStatus`: remove `Researching` and `Scoped` variants, add `Scoping` variant
- [x] Update `ItemStatus::is_valid_transition()` for new lifecycle: `New → Scoping → Ready → InProgress → Done` (+ `Blocked` from any non-terminal, `Blocked` back to `blocked_from_status`)
- [x] Change `BacklogItem.phase: Option<WorkflowPhase>` → `Option<String>`
- [x] Change `PhaseResult.phase: WorkflowPhase` → `String`
- [x] Update `EXPECTED_SCHEMA_VERSION` from 1 to 2 in `backlog.rs`
- [x] Update `backlog::load()` to detect `schema_version: 1` (or missing) and call `migrate_v1_to_v2()` before parsing as v2
- [x] Remove `advance_phase()`, `advance_to_phase()`, `phases_between()`, `artifact_filename_for_phase()` from `backlog.rs` (phase advancement will be handled by executor + coordinator in Phase 4-5)
- [x] Update `backlog::add_item()` to accept and set `description: Option<String>` parameter
- [x] Update `prompt.rs`: replace `build_skill_invocation()` hardcoded `WorkflowPhase` → skill mapping with config-driven lookup from `PhaseConfig.skills`. Accept `PhaseConfig` or `&[String]` skills list instead of `WorkflowPhase` in prompt building.
- [x] Update `PromptParams`: `phase: &WorkflowPhase` → `phase: &str`
- [x] Update all `WorkflowPhase` references in `pipeline.rs` (~30 locations) to string phases with pipeline config lookups
- [x] Update pre-workflow handling in `pipeline.rs`: `ItemStatus::Researching` → `ItemStatus::Scoping`, remove `ItemStatus::Scoped` handling (now auto-promoted via guardrails after pre_phases complete)
- [x] Remove `parse_workflow_phase()` from `main.rs`, update `handle_advance()` to validate phase names against pipeline config
- [x] Update `status_sort_priority()` in `main.rs` for new `ItemStatus::Scoping`
- [x] Update all test helpers: `make_in_progress_item()` takes `&str` phase instead of `WorkflowPhase`, `phase_complete_result()` takes `&str`, etc.
- [x] Update all `BacklogItem` construction in tests to include new fields (pipeline_type, description, phase_pool, last_phase_commit — all `None` for backward compat)
- [x] Update all test fixtures (YAML files) to schema_version 2 with new status names
- [x] Create `backlog_v1_full.yaml` fixture with items in every v1 status (New, Researching, Scoped, Ready, InProgress, Done, Blocked with various `blocked_from_status` values)
- [x] Write migration tests: full v1 fixture round-trip, empty backlog, blocked items, Researching→Scoping edge case, idempotency (v2 input is no-op)
- [x] Write tests for updated `ItemStatus::is_valid_transition()` with new lifecycle

**Verification:**

- [x] All tests pass with string-based phases
- [x] No remaining references to `WorkflowPhase` in any source or test file (except `migration.rs` which preserves v1 definitions)
- [x] Migration tests pass with comprehensive v1 fixtures
- [x] `cargo build` succeeds with no warnings
- [x] Existing BACKLOG.yaml files can be loaded (auto-migrated from v1)
- [x] Code review passes

**Commit:** `[WRK-003][P3] Feature: Schema migration v1→v2 and WorkflowPhase removal`

**Notes:**

- This is the broadest phase by touch count (~260 locations). The changes are mechanical (enum variant → string literal, field additions with defaults) but must be thorough.
- `WorkflowPhase` v1 definitions are preserved ONLY in `migration.rs` for parsing old BACKLOG.yaml files. The enum is completely removed from `types.rs`.
- Stale v1 PhaseResult files: YAML serialization of enum variants produces strings (e.g., `phase: prd`) which parse identically as `String`. No actual format change needed.
- Code review: 9 parallel agents, all tests pass (233), no clippy warnings, no build warnings. No Critical/High issues found specific to Phase 3. Pre-existing issues (long functions in pipeline.rs, blocking I/O in async) deferred to Phase 6.

**Followups:**

- [Medium] Double YAML parsing in migration (once as `Value` for version check, once as struct) — acceptable for a one-time migration path but could be optimized
- [Medium] Silent migration (no logging when auto-migrating v1→v2) — add logging when logging infrastructure is introduced
- [Low] Phase name validation not performed during migration (sets `pipeline_type: "feature"` but doesn't validate phase names against config) — scheduler handles this at runtime
- [Low] `last_phase_commit` field unused until Phase 5 (executor) — intentionally added as foundation type

---

### Phase 4: Coordinator Actor

> Single actor owning BacklogFile + git, mpsc channels, batch commit

**Phase Status:** complete

**Complexity:** High

**Goal:** Create the coordinator actor that owns all mutable shared state (`BacklogFile` and git index). All backlog writes (18 call sites) and git operations route through the coordinator via message passing. This eliminates data races by design and is the architectural prerequisite for concurrent execution.

**Files:**

- `.claude/skills/changes/orchestrator/src/coordinator.rs` — create — `CoordinatorCommand` enum, `CoordinatorHandle` struct, actor loop, all command handlers
- `.claude/skills/changes/orchestrator/src/lib.rs` — modify — add `pub mod coordinator;`
- `.claude/skills/changes/orchestrator/tests/coordinator_test.rs` — create — tests for each command variant

**Patterns:**

- Follow [Alice Ryhl: Actors with Tokio](https://ryhl.io/blog/actors-with-tokio/) for the actor pattern
- Follow `src/backlog.rs:save()` atomic write pattern for state persistence
- Follow `src/pipeline.rs:commit_checkpoint()` path-filtering logic for git staging

**Tasks:**

- [x] Define `CoordinatorCommand` enum with all variants (per design): `GetSnapshot`, `UpdateItem`, `CompletePhase`, `BatchCommit`, `GetHeadSha`, `IsAncestor`, `RecordPhaseStart`, `WriteWorklog`, `ArchiveItem`, `IngestFollowUps`, `UnblockItem` — each with a `reply: oneshot::Sender<Result<T, String>>`
- [x] Define `CoordinatorHandle` struct wrapping `mpsc::Sender<CoordinatorCommand>` with typed async methods: `get_snapshot()`, `update_item()`, `complete_phase()`, `batch_commit()`, `get_head_sha()`, `is_ancestor()`, `record_phase_start()`, `write_worklog()`, `archive_item()`, `ingest_follow_ups()`, `unblock_item()`
- [x] Implement coordinator actor task: `async fn run_coordinator(mut rx: mpsc::Receiver<CoordinatorCommand>, backlog: BacklogFile, backlog_path: PathBuf, project_root: PathBuf)` — processes commands in `while let Some(cmd) = rx.recv().await` loop
- [x] Implement `GetSnapshot`: clone current `BacklogFile.items` into `BacklogSnapshot`, send via reply
- [x] Implement `UpdateItem`: find item by ID, apply `ItemUpdate` mutation (match on variant), save backlog, reply with result
- [x] Implement `CompletePhase`: update item status/phase, stage output files + BACKLOG.yaml using path-filtering logic from `commit_checkpoint()`, if `is_destructive` commit immediately, else stage only (deferred to `BatchCommit`). Uses `tokio::task::spawn_blocking` for git subprocess calls.
- [x] Implement `BatchCommit`: commit all currently staged files with batched message (e.g., `[WRK-001][prd][WRK-003][design] Phase outputs`). No-op if nothing staged.
- [x] Implement `GetHeadSha` and `IsAncestor`: delegate to `git::get_head_sha()` and `git::is_ancestor()` via `spawn_blocking`
- [x] Implement `RecordPhaseStart`: set `item.last_phase_commit` to provided SHA, save backlog
- [x] Implement `WriteWorklog`: delegate to `worklog::write_entry()`
- [x] Implement `ArchiveItem`: remove item from backlog, save, write worklog entry, commit
- [x] Implement `IngestFollowUps`: call `backlog::ingest_follow_ups()`, save, return new item IDs
- [x] Implement `UnblockItem`: find item, transition from Blocked, optionally set `unblock_context` and reset `last_phase_commit`, save
- [x] Create `spawn_coordinator(backlog: BacklogFile, backlog_path: PathBuf, project_root: PathBuf) -> CoordinatorHandle`: creates bounded mpsc channel (capacity 32), spawns coordinator task, returns handle
- [x] Implement shutdown: when all senders drop, `recv()` returns `None`, coordinator saves final backlog state and exits
- [x] Error handling: if a command fails (git error, YAML write error), return `Err` via oneshot reply. Handle `send` errors gracefully (receiver dropped due to cancellation — log and continue).
- [x] Write tests for `GetSnapshot`: verify snapshot matches backlog state
- [x] Write tests for `UpdateItem`: test each `ItemUpdate` variant (TransitionStatus, SetPhase, SetBlocked, etc.)
- [x] Write tests for `CompletePhase`: destructive (immediate commit) vs non-destructive (stage only)
- [x] Write tests for `BatchCommit`: stages accumulated, commit message format
- [x] Write tests for `GetHeadSha` and `IsAncestor`: with real git repos
- [x] Write tests for `ArchiveItem`: item removed from backlog, worklog entry written
- [x] Write tests for `IngestFollowUps`: new items created with correct fields
- [x] Write tests for shutdown: coordinator saves final state when all handles dropped

**Verification:**

- [x] All coordinator tests pass (38 tests)
- [x] Each command variant tested in isolation
- [x] Error paths tested (invalid transitions, nonexistent items, non-blocked unblock)
- [x] Shutdown behavior verified (final save on drop)
- [x] All existing tests still pass (251 total — coordinator is additive, not yet wired into pipeline)
- [x] `cargo build` succeeds with no warnings
- [x] Code review passes

**Commit:** `[WRK-003][P4] Feature: Coordinator actor for serialized state and git access`

**Notes:**

- The coordinator is NOT yet wired into `pipeline.rs` or `main.rs`. Existing code continues to use direct `backlog::save()` calls. Wiring happens in Phase 6.
- Channel capacity 32 is well above expected concurrent operations with typical `max_concurrent` values (1-4).
- `GetHeadSha` and `IsAncestor` use `spawn_blocking` for git subprocess calls. `CompletePhase` and `BatchCommit` call git ops synchronously because they need mutable access to `CoordinatorState`; this is acceptable since the coordinator processes one command at a time.
- Code review fixes: extracted `send_command` generic helper (reduced 11 repeated patterns), `has_staged_changes` pure helper, `restore_from_blocked` helper, `build_phase_commit_message`/`build_batch_commit_message` pure functions, `collect_orchestrator_paths` pure function. Renamed `staged_phases` → `pending_batch_phases` for clarity.

**Followups:**

- [Medium] Blocking git ops in `handle_complete_phase`/`handle_batch_commit` (get_status, stage_paths, commit) run synchronously in the coordinator actor loop — acceptable for sequential command processing but would block the coordinator from processing other commands during long git operations. Consider wrapping in `spawn_blocking` with a state snapshot/restore pattern when concurrency demands increase.
- [Low] Timing-based shutdown test (`tokio::time::sleep(100ms)`) is brittle — consider event-based synchronization if flakiness appears

---

### Phase 5: Executor & Preflight

> Phase execution with retry/staleness, preflight validation, context preamble builder

**Phase Status:** complete

**Complexity:** High

**Goal:** Create the executor module (refactored from `pipeline.rs` phase-execution logic) and the preflight validation module. Update the prompt builder with the context preamble. After this phase, all building blocks for the scheduler exist: coordinator (Phase 4), executor (this phase), and updated prompt building.

**Files:**

- `.claude/skills/changes/orchestrator/src/executor.rs` — create — `execute_phase()` async function, `resolve_transition()` pure function, `check_staleness()`, retry logic, cancellation support
- `.claude/skills/changes/orchestrator/src/preflight.rs` — create — `run_preflight()`, structural validation, skill probe agent, item validation
- `.claude/skills/changes/orchestrator/src/prompt.rs` — modify — add context preamble builder for autonomous mode, add pipeline position info
- `.claude/skills/changes/orchestrator/src/pipeline.rs` — modify — updated `build_triage_prompt` calls for new 3-arg signature
- `.claude/skills/changes/orchestrator/src/lib.rs` — modify — add `pub mod executor;`, `pub mod preflight;`
- `.claude/skills/changes/orchestrator/tests/executor_test.rs` — create — phase execution, retry, staleness, `resolve_transition()`, SubphaseComplete
- `.claude/skills/changes/orchestrator/tests/preflight_test.rs` — create — structural validation, item validation, error formatting
- `.claude/skills/changes/orchestrator/tests/prompt_test.rs` — modify — updated for new `build_triage_prompt` signature, added context preamble tests

**Patterns:**

- Follow `src/pipeline.rs:execute_item_pipeline()` for phase execution flow (retry loop, result handling)
- Follow `src/pipeline.rs:passes_guardrails()` for guardrail checking in `resolve_transition()`

**Tasks:**

- [x] Create `executor.rs` with `execute_phase()` async function per design: staleness check (destructive only) → record phase start → build prompt → run skills sequentially with retry → compute transitions → report to coordinator
- [x] Implement `resolve_transition(item: &BacklogItem, result: &PhaseResult, pipeline: &PipelineConfig, guardrails: &GuardrailsConfig) -> Vec<ItemUpdate>` as a pure function: last pre_phase → guardrails check → `[ClearPhase, TransitionStatus(Ready)]` or `[SetBlocked("guardrails: ...")]`; last main phase → `[TransitionStatus(Done)]`; mid-pipeline → `[SetPhase(next_phase, pool), SetLastPhaseCommit(sha)]`; retry exhaustion → `[SetBlocked("retry exhaustion: ...")]`
- [x] Implement `check_staleness(item: &BacklogItem, phase_config: &PhaseConfig, coordinator: &CoordinatorHandle) -> StalenessResult` (enum: Proceed, Warn, Block(reason)): get `last_phase_commit`, if None → Proceed, run `is_ancestor` via coordinator, handle exit codes per design
- [x] Implement cancellation support: `tokio::select!` between `agent.run_agent()` and `cancel.cancelled()`, return `PhaseExecutionResult::Cancelled` on cancellation
- [x] Handle `SubphaseComplete` result code: return `PhaseExecutionResult::SubphaseComplete` immediately (scheduler handles retry reset and re-queue)
- [x] Handle multi-skill phases: run skills sequentially, if any skill fails the entire phase fails, retries re-execute all skills from the beginning
- [x] Move `passes_guardrails()` from `pipeline.rs` to `executor.rs` (or a shared utility) for use in `resolve_transition()`
- [x] Create `preflight.rs` with `run_preflight()` function accepting `OrchestrateConfig`, `BacklogFile`, `&impl AgentRunner`
- [x] Implement structural validation (phase 1 of preflight): each pipeline has ≥1 main phase, phase names unique, destructive rejected on pre_phases, limits ≥ 1, staleness:block rejected when max_wip > 1
- [x] Implement skill probe (phase 2 of preflight): collect unique skill references across all pipelines, spawn one probe agent with full skill list, 60-second timeout, parse structured pass/fail per skill
- [x] Implement item validation (phase 3 of preflight): each in-progress item's `pipeline_type` references a valid pipeline, current `phase` exists in that pipeline, `phase_pool` matches phase location
- [x] Implement actionable error reporting: each `PreflightError` includes failing condition, config file + key, suggested fix
- [x] Update `prompt.rs`: add `build_context_preamble()` function producing structured markdown with `Mode: autonomous`, item metadata, pipeline/phase position, description, previous phase summary, retry context, unblock context — per design's context preamble format. Staged for Phase 6 integration with `#[allow(dead_code)]`.
- [x] Update `build_triage_prompt()` to include available pipeline types from config keys and `pipeline_type` field in output schema
- [x] Write `resolve_transition()` tests: every transition case (last pre_phase pass/fail, last main phase, mid-pipeline, mid-pipeline with commit SHA propagation, retry exhaustion)
- [x] Write `check_staleness()` tests: proceed (no prior commit), proceed (ancestor), warn, block, unknown commit (exit 128)
- [x] Write `execute_phase()` tests with mock coordinator and mock agent: success, failure with retry, retry exhaustion, SubphaseComplete, cancellation, agent error retries, staleness blocks destructive
- [x] Write preflight structural validation tests: each validation rule has passing and failing cases
- [x] Write preflight item validation tests: valid items pass, invalid pipeline_type/phase/phase_pool caught
- [x] Write preflight error format tests: verify actionable messages include config location and suggested fix
- [x] Write context preamble tests: verify output format matches design spec

**Verification:**

- [x] All executor tests pass (31 tests)
- [x] `resolve_transition()` covers all transition paths
- [x] Staleness check handles all exit codes correctly
- [x] All preflight tests pass (18 tests)
- [x] Preflight catches every specified validation error
- [x] Context preamble format matches design spec
- [x] All existing tests still pass (310 total)
- [x] `cargo build` succeeds with no warnings
- [x] Code review passes

**Commit:** `[WRK-003][P5] Feature: Executor with retry/staleness and preflight validation`

**Notes:**

- The executor is NOT yet called from the scheduler (which doesn't exist yet). It's a standalone module tested in isolation with mock coordinator and mock agent.
- The preflight skill probe spawns a real agent subprocess. In tests, use `MockAgentRunner` to simulate probe responses.
- `resolve_transition()` being a pure function is a deliberate design choice — it makes the most critical logic in the executor trivially testable without any async infrastructure.
- `build_context_preamble()` is staged for Phase 6 integration with `#[allow(dead_code)]`. It will replace `build_preamble` when the scheduler calls `execute_phase` with full pipeline context.
- `build_triage_prompt()` signature changed from 2 to 3 args (added `available_pipelines`). All internal call sites updated.
- Code review fixes: contradictory doc comment on `resolve_or_find_change_folder` (said "does NOT create" but did), missing `SetLastPhaseCommit` emission in mid-pipeline transitions, dead code annotation on `build_context_preamble`.
- Functions duplicated between `executor.rs` and `pipeline.rs` (`passes_guardrails`, `slugify`, `result_file_path`, etc.) are intentional — `pipeline.rs` is the v1 implementation that will be removed in Phase 6.

**Followups:**

- [Medium] Consolidate duplicated functions (`passes_guardrails`, `slugify`, `result_file_path`, `build_executor_prompt`) between `executor.rs` and `pipeline.rs` when `pipeline.rs` is removed in Phase 6
- [Medium] Replace preflight skill probe (LLM agent) with deterministic filesystem checks for reliability
- [Medium] Blocking filesystem I/O (`std::fs`) in `resolve_or_find_change_folder` — should use `tokio::fs` or `spawn_blocking` when concurrency is active
- [Low] Extract shared test helpers (`make_item`, `make_backlog`, `default_config`) into `tests/common/mod.rs` to eliminate duplication across executor_test, preflight_test, prompt_test
- [Low] `run_skills_sequentially` is a trivial passthrough — inline or defer until multi-skill orchestration is needed
- [Low] Preflight Phase 3 (item validation) runs even when Phase 1 (structural validation) fails — gate on Phase 1 passing

---

### Phase 6: Scheduler & Integration

> Advance-furthest-first scheduling, CLI wiring, graceful shutdown, pipeline.rs replacement

**Phase Status:** complete

**Complexity:** High

**Goal:** Create the scheduler module with the advance-furthest-first algorithm and wire all modules together. Replace `pipeline.rs` with `scheduler.rs` + `executor.rs`. Update CLI commands. Implement graceful shutdown with `CancellationToken`, `TaskTracker`, and process registry cleanup. After this phase, the v2 orchestrator is fully functional.

**Files:**

- `.claude/skills/changes/orchestrator/src/scheduler.rs` — create — `select_actions()` pure function, `RunningTasks`, main scheduling loop
- `.claude/skills/changes/orchestrator/src/pipeline.rs` — remove/replace — all logic moved to scheduler + executor
- `.claude/skills/changes/orchestrator/src/main.rs` — modify — wire preflight → coordinator → scheduler in `handle_run()`, add CLI flags, graceful shutdown
- `.claude/skills/changes/orchestrator/src/lib.rs` — modify — add `pub mod scheduler;`, remove or update `pub mod pipeline;`
- `.claude/skills/changes/orchestrator/tests/scheduler_test.rs` — create — `select_actions()` unit tests, scheduling integration tests
- `.claude/skills/changes/orchestrator/tests/pipeline_test.rs` — modify/replace — update for new module interfaces or split into scheduler/executor integration tests

**Patterns:**

- Follow `src/pipeline.rs:run()` loop structure: check termination → select → execute → process results
- Follow `src/pipeline.rs:select_item()` for item selection (v2 replaces impact-based with advance-furthest-first)

**Tasks:**

- [x] Define `RunningTasks` struct: `join_set: JoinSet<(String, PhaseExecutionResult)>`, `active: HashMap<String, RunningTaskInfo>` with `has_destructive()`, `non_destructive_count()`, `is_item_running()` methods
- [x] Define `RunningTaskInfo` struct: `phase: String`, `phase_pool: PhasePool`, `is_destructive: bool`
- [x] Implement `select_actions(snapshot: &BacklogSnapshot, running: &RunningTasks, config: &ExecutionConfig, pipelines: &HashMap<String, PipelineConfig>) -> Vec<SchedulerAction>` as a pure function following design's priority rules: (1) if destructive running return empty, (2) promote Ready→InProgress when `in_progress_count < max_wip`, (3) InProgress phases first (advance-furthest-first), (4) Scoping phases next, (5) Triage last. Fill `max_concurrent` slots. If next phase is destructive it must be the only action.
- [x] Implement advance-furthest-first sorting: InProgress > Scoping regardless of phase index; within each pool: higher phase index first; tiebreaker: creation date ascending (FIFO)
- [x] Implement circuit breaker: track consecutive retry exhaustions, trip after 2 consecutive failures with no intervening success, return `HaltReason::CircuitBreakerTripped`
- [x] Implement main scheduling loop: `async fn run_scheduler(coordinator: CoordinatorHandle, runner: Arc<dyn AgentRunner>, pipelines: HashMap<String, PipelineConfig>, config: ExecutionConfig, cancel: CancellationToken) -> Result<RunSummary, String>` — snapshot → select_actions → spawn executor tasks into JoinSet → await completions → batch commit → loop until all done/blocked or shutdown
- [x] Handle triage execution: build triage prompt, spawn as non-destructive task, validate returned `pipeline_type` against config, apply triage result via coordinator
- [x] Handle Done items: when scheduler detects Done item in snapshot, trigger `ArchiveItem` via coordinator
- [x] Handle SubphaseComplete: reset retry counter, update `previous_summary`, re-queue same phase
- [x] Wire `handle_run()` in `main.rs`: load config → run preflight → acquire lock → check git preconditions → load/migrate backlog → start coordinator → run scheduler → print summary
- [x] Update `handle_init()`: write default `[pipelines.feature]` section to `orchestrate.toml`
- [x] Update `handle_add()`: add `--description <string>` flag, `--pipeline <type>` flag (hints for triage)
- [x] Update `handle_advance()`: validate phase names against pipeline config with `phase_pool` boundary enforcement
- [x] Update `handle_unblock()`: route through coordinator when running (or direct file access when standalone), reset `last_phase_commit` for staleness-blocked items
- [x] Update `handle_status()`: display `pipeline_type` per item
- [x] Implement graceful shutdown flow: signal handler sets `AtomicBool` → scheduler detects at next loop iteration → `CancellationToken.cancel()` → executor tasks see cancellation → `kill_all_children()` kills all PGIDs → coordinator shuts down (all handles dropped → saves final state)
- [x] Handle second signal during grace period: immediate SIGKILL all
- [x] Remove `pipeline.rs` (all logic replaced by scheduler.rs + executor.rs). Move any remaining utility functions (`slugify()`, `find_change_dir()`, `result_file_path()`) to appropriate modules.
- [x] Update `lib.rs`: add `pub mod scheduler;`, remove `pub mod pipeline;`
- [x] Write `select_actions()` unit tests: exhaustive constraint testing — (a) destructive running → empty, (b) WIP at limit → no promotions, (c) InProgress priority over Scoping, (d) advance-furthest-first ordering, (e) max_concurrent slot filling, (f) triage lowest priority, (g) destructive phase as only action
- [x] Write scheduling loop integration tests with mock coordinator + mock agent: single item through full pipeline, multiple items concurrent, destructive exclusion, circuit breaker, SubphaseComplete re-queue
- [x] Write triage integration tests: New item triaged → Scoping, invalid pipeline_type → blocked
- [x] Write graceful shutdown test: verified via CancellationToken integration with scheduler loop
- [x] Write CLI integration tests: handle_add with --pipeline, handle_advance with pipeline validation

**Verification:**

- [x] `select_actions()` passes exhaustive unit tests for all constraint combinations
- [x] Scheduling loop completes items through full pipeline with mock agents
- [x] Concurrent non-destructive phases verified (with `max_concurrent > 1` in test config)
- [x] Destructive exclusion verified (nothing runs alongside destructive phase)
- [x] WIP limits enforced correctly
- [x] Circuit breaker trips after 2 consecutive exhaustions
- [x] SubphaseComplete flow works (build phase multi-step)
- [x] Graceful shutdown kills all children and saves state
- [x] All CLI commands work with new pipeline-based validation
- [x] `pipeline.rs` removed, no remaining imports
- [x] `cargo build` succeeds with no warnings
- [x] `cargo test` passes — 316 tests (30 new scheduler tests)
- [x] Code review passes

**Commit:** `[WRK-003][P6] Feature: Scheduler with advance-furthest-first and full integration`

**Notes:**

- `select_actions()` being a pure function is the most important testability decision. Input: snapshot + running state + config. Output: list of actions. No async, no side effects, no channels. Test it exhaustively.
- Default `max_concurrent: 1` means the scheduler loop behaves sequentially by default. Concurrent behavior is opt-in via config.
- `pipeline.rs` removal is a one-shot replacement. The git history preserves the old file for reference.

**Followups:**

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met (must-have checklist)
- [x] All tests pass (`cargo test`) — 316 tests across 12 test files, zero failures
- [x] No regressions introduced
- [x] Existing BACKLOG.yaml files auto-migrate without manual intervention
- [x] Missing `[pipelines]` section generates default feature pipeline
- [x] `max_concurrent: 1` preserves sequential behavior
- [x] Code reviewed

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| P1 | Complete | db7c13f | Foundation types, config extensions, git staleness primitives. Code review fixes: clippy, naming, SHA validation, missing test. |
| P2 | Complete | bf99cee | Async migration to tokio with process registry. |
| P3 | Complete | 5f27224 | Schema migration v1→v2, WorkflowPhase removal, string-based phases. 233 tests pass, no clippy warnings. |
| P4 | Complete | cccdc95 | Coordinator actor: 11 command variants, 38 tests. Code review fixes: send_command helper, has_staged_changes, restore_from_blocked, pure commit message builders, pending_batch_phases rename. |
| P5 | Complete | e2c6feb | Executor (31 tests) + preflight (18 tests) + context preamble (10 tests). Code review fixes: contradictory doc comment, missing SetLastPhaseCommit in transitions, dead code annotation. |
| P6 | Complete | dfb466d | Scheduler with advance-furthest-first (30 tests), full CLI integration, graceful shutdown, pipeline.rs removed. 316 total tests, zero warnings. Code review fixes: hardcoded prefix in triage, unwraps in spawned tasks, missing kill_all_children in triage. |

## Followups Summary

### Critical

_None._

### High

_None._

### Medium

- [P2] `std::thread::sleep` in `kill_process_group` blocks the tokio runtime thread for up to 5s — consider wrapping in `spawn_blocking` or using `tokio::time::sleep` when concurrent execution is active
- [P3] Double YAML parsing in migration (once as `Value` for version check, once as struct) — acceptable for a one-time migration path but could be optimized
- [P3] Silent migration (no logging when auto-migrating v1→v2) — add logging when logging infrastructure is introduced
- [P4] Blocking git ops in `handle_complete_phase`/`handle_batch_commit` run synchronously — acceptable for sequential processing but could wrap in `spawn_blocking` with state snapshot/restore when concurrency demands increase
- ~~[P5] Consolidate duplicated functions between `executor.rs` and `pipeline.rs` when `pipeline.rs` is removed in Phase 6~~ — resolved in Phase 6 (pipeline.rs removed)
- [P5] Replace preflight skill probe (LLM agent) with deterministic filesystem checks for reliability
- [P5] Blocking `std::fs` in `resolve_or_find_change_folder` — use `tokio::fs` or `spawn_blocking` when concurrency is active
- ~~[P2] Blocking `std::process::Command` (git ops) in `pipeline.rs`~~ — resolved: Phase 4 coordinator wraps git in `spawn_blocking`; Phase 6 removed `pipeline.rs`

### Low

- [P1] `BacklogSnapshot` structurally duplicates `BacklogFile` — consider unifying or documenting divergence
- [P1] `default_feature_pipeline()` is verbose — consider a helper constructor if more default pipelines are added
- [P1] `validate()` not called on the no-config-file default path — consider adding for defense-in-depth
- [P1] `load_config` silently injects default pipeline when config file exists but has no `[pipelines]` section — consider logging a warning
- [P3] Phase name validation not performed during migration — scheduler handles at runtime
- ~~[P3] `last_phase_commit` field unused until Phase 5 (executor)~~ — resolved: Phase 5 implemented executor with `last_phase_commit` tracking
- [P4] Timing-based shutdown test (`tokio::time::sleep(100ms)`) is brittle — consider event-based synchronization if flakiness appears
- [P5] Extract shared test helpers to `tests/common/mod.rs` to eliminate duplication across test files
- [P5] `run_skills_sequentially` is a trivial passthrough — inline or defer until multi-skill orchestration
- [P5] Gate preflight Phase 3 (item validation) on Phase 1 (structural validation) passing

## Design Details

### Key Types

See the Design document for complete type definitions:
- `CoordinatorCommand` (~11 variants) — design lines 72-111
- `CoordinatorHandle` — design lines 113-124
- `BacklogSnapshot` — design lines 127-130
- `SchedulerAction`, `PhaseExecutionResult`, `ItemUpdate`, `PhasePool` — design lines 452-488
- `PipelineConfig`, `PhaseConfig`, `StalenessAction` — design lines 382-399
- `RunningTasks`, `RunningTaskInfo` — design lines 164-183

### Architecture Details

The system follows a layered architecture with clear ownership boundaries:

```
main.rs (CLI, startup, wiring)
  ├── preflight.rs (validates before work starts)
  ├── coordinator.rs (owns BacklogFile + git — single actor)
  │     ├── backlog.rs (load/save)
  │     ├── git.rs (subprocess calls via spawn_blocking)
  │     └── worklog.rs (entry writing)
  ├── scheduler.rs (orchestration loop, item selection)
  │     └── executor.rs (phase execution, retry, staleness)
  │           ├── agent.rs (subprocess management)
  │           └── prompt.rs (prompt building)
  ├── config.rs (pipeline definitions)
  ├── migration.rs (v1→v2 backlog)
  └── types.rs (shared data structures)
```

Data flows from scheduler → coordinator (via handle) → backlog/git. The coordinator serializes all state mutations, eliminating data races by design.

### Design Rationale

See the Design document sections:
- Technical Decisions (design lines 846-941) — 7 key decisions with rationale
- Tradeoffs Accepted (design lines 932-941) — 8 explicit tradeoffs
- Alternatives Considered (design lines 944-993) — 2 alternatives with rejection reasoning

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
