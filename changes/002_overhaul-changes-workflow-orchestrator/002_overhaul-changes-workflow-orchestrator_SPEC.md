# SPEC: Overhaul Changes Workflow with Orchestrator

**ID:** 002
**Status:** Approved
**Created:** 2026-02-11
**PRD:** ./002_overhaul-changes-workflow-orchestrator_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

The current changes workflow handles individual changes well but has no system for managing work across changes. Follow-ups dead-end, there's no structured triage, and no way for an AI agent to autonomously work through a queue. This SPEC implements a Rust CLI binary (`orchestrate`) that drives the existing changes workflow pipeline autonomously — managing a YAML backlog, spawning fresh Claude subprocesses per phase, parsing structured results, and committing git checkpoints.

The design doc defines 10 components (CLI, Backlog Manager, Pipeline Engine, Agent Runner, Prompt Builder, Git Manager, Config, Lock Manager, Work Log, plus pre-workflow agents), 7 flows, and a two-tier architecture (pre-workflow triage/research + workflow pipeline). This is the first Rust code in the project.

## Approach

Build the orchestrator bottom-up: shared types first, then leaf modules (config, lock, backlog, git, worklog), then agent infrastructure (agent runner, prompt builder), then the pipeline engine, and finally the CLI layer. Each tier depends only on the tier below it, ensuring no circular dependencies and enabling thorough testing at each level.

The agent runner is the highest-risk component (subprocess management, process groups, signal handling). The pipeline engine is the most complex (state machine with retry, circuit breaker, guardrails, follow-up triage). Both get dedicated phases with focused testing.

The naming convention migration (11 skill files, `NNN_` → `WRK-001_`) happens last to avoid disrupting the existing workflow while the orchestrator is being built.

**Patterns to follow:**

- `.claude/skills/changes/workflows/internal/implement-spec-autonomous.sh` — the existing bash orchestrator being replaced; demonstrates `claude -p` invocation, retry logic, and marker-based result parsing
- `.claude/skills/changes/workflows/internal/implement-spec-autonomous-auto-loop.md` — the single-phase agent prompt; demonstrates how skills are invoked in autonomous mode
- `.claude/skills/rust-style/SKILL.md` — Rust style guide: modern module style (no `mod.rs`), enum-based state machines, clone liberally, traits as interfaces, pure functions
- `changes/002_.../ideas/BACKLOG.yaml` — existing backlog template; reference schema
- `changes/002_.../ideas/IDEA_TEMPLATE.md` — existing idea file template; will be moved to skills folder

**Implementation boundaries:**

- Do not modify: any existing skill workflow files (`.claude/skills/changes/workflows/*/`) until Phase 7
- Do not modify: `.claude/skills/changes/SKILL.md` or `workflow-guide.md` until Phase 7
- Do not refactor: existing template files beyond the naming convention changes in Phase 7
- Do not add: async runtime (tokio) — synchronous subprocess management only

## Open Questions

- [ ] Exact autonomous prompt wrapper wording — the structure (preamble + skill + suffix) is defined in the design, but exact phrasing needs tuning with real skill invocations. Phase 5 drafts initial versions; Phase 6 tests them. Should we include a "fallback to stdout markers" mode if JSON file writing proves unreliable?
- [ ] Should `orchestrate triage` run items through both triage and research sequentially, or only triage? Design leans triage-only. Leaning: triage-only for v1, add `orchestrate research` later if needed.

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Project Scaffolding & Core Types | Med | Cargo project, all shared types/enums with serde, config loading, lock manager |
| 2 | Backlog Manager | Med | Full CRUD on BACKLOG.yaml with atomic writes, ID generation, state transitions |
| 3 | Git Manager, Worklog, and Simple CLI | Med | Git operations, worklog writing, `init`/`add`/`status`/`advance`/`unblock` commands |
| 4 | Agent Runner | High | Subprocess spawning, process groups, timeouts, signal handling, result parsing |
| 5 | Prompt Builder | Med | Autonomous prompt construction for all phases including pre-workflow agents |
| 6 | Pipeline Engine & Run Command | High | Full orchestration loop with retry, circuit breaker, guardrails, and `run`/`triage` commands |
| 7 | Naming Convention Migration | Low | Update 11 skill/template files from `NNN_` to `WRK-001_` format |

**Ordering rationale:** Strict bottom-up dependency order. Types (P1) → leaf modules (P2-P3) → agent infra (P4-P5) → pipeline (P6) → skill updates (P7). The agent runner (P4) is isolated as its own phase because it's the highest technical risk — subprocess management with process groups and signal handling. The naming migration (P7) is last to avoid disrupting the existing workflow during development.

---

## Phases

Each phase should leave the codebase in a functional, stable state. Complete and verify each phase before moving to the next.

---

### Phase 1: Project Scaffolding & Core Types

> Cargo project setup, all shared types with serde derives, config loading, and lock manager

**Complexity:** Med

**Goal:** Establish the Rust project structure with all shared data types that every other module depends on, plus the two simplest leaf modules (config and lock).

**Files:**

- `.claude/skills/changes/orchestrator/Cargo.toml` — create — project manifest with all dependencies
- `.claude/skills/changes/orchestrator/src/main.rs` — create — entry point with module declarations (stub `fn main`)
- `.claude/skills/changes/orchestrator/src/types.rs` — create — all shared enums and structs
- `.claude/skills/changes/orchestrator/src/config.rs` — create — load/validate `orchestrate.toml`
- `.claude/skills/changes/orchestrator/src/lock.rs` — create — PID-based lock file management
- `.claude/skills/changes/orchestrator/src/types/` — create directory — if types module needs sub-files
- `.claude/skills/changes/orchestrator/tests/types_test.rs` — create — serialization round-trip tests
- `.claude/skills/changes/orchestrator/tests/config_test.rs` — create — config parsing tests
- `.claude/skills/changes/orchestrator/tests/lock_test.rs` — create — lock behavior tests

**Tasks:**

- [x] Create `Cargo.toml` with dependencies: `clap` (v4, derive), `serde` + `serde_derive`, `serde_yaml_ng`, `serde_json`, `toml`, `tempfile`, `nix`, `signal-hook`, `wait-timeout`, `fslock`, `chrono`
- [x] Create `src/main.rs` with module declarations for all planned modules (types, config, lock, backlog, git, worklog, prompt, agent, pipeline) and a stub `fn main()`
- [x] Define all enums in `src/types.rs`: `ItemStatus` (New, Researching, Scoped, Ready, InProgress, Done, Blocked), `WorkflowPhase` (Prd, Research, Design, Spec, Build, Review), `ResultCode` (SubphaseComplete, PhaseComplete, Failed, Blocked), `BlockType` (Clarification, Decision), `SizeLevel` (Small, Medium, Large), `DimensionLevel` (Low, Medium, High) — all with `Serialize`, `Deserialize`, `Clone`, `Debug`, `PartialEq`
- [x] Define `BacklogItem` struct with all fields from the design's BACKLOG.yaml schema (id, title, status, phase, size, complexity, risk, impact, requires_human_review, origin, blocked_from_status, blocked_reason, blocked_type, unblock_context, tags, dependencies, created, updated) — use `Option<T>` for nullable fields, `#[serde(default)]` for optional fields
- [x] Define `BacklogFile` struct with `schema_version: u32` and `items: Vec<BacklogItem>`
- [x] Define `PhaseResult` struct with fields: item_id, phase, result (ResultCode), summary, context, updated_assessments (optional), follow_ups (Vec<FollowUp>)
- [x] Define `FollowUp` struct with fields: title, context, suggested_size (optional), suggested_risk (optional)
- [x] Define `UpdatedAssessments` struct with optional size, complexity, risk, impact fields
- [x] Define `OrchestrateConfig` struct with project (prefix), guardrails (max_size, max_complexity, max_risk), execution (phase_timeout_minutes, max_retries, default_phase_cap) sections — all with sensible defaults via `Default` impl
- [x] Implement `ItemStatus::is_valid_transition(&self, to: &ItemStatus) -> bool` as an explicit transition validation function
- [x] Implement `WorkflowPhase::next(&self) -> Option<WorkflowPhase>` for sequential phase advancement
- [x] Implement config loading: read `orchestrate.toml` from project root, deserialize, apply defaults for missing fields
- [x] Implement lock manager: `try_acquire(path) -> Result<LockGuard>` using fslock, write PID to lock file, check for stale locks (PID alive check), RAII release on drop. Create `.orchestrator/` directory if it doesn't exist before acquiring lock
- [x] Write tests: YAML serialization round-trips for all types (serialize → deserialize → assert equal)
- [x] Write tests: JSON serialization round-trips for PhaseResult
- [x] Write tests: TOML parsing for valid/invalid/partial config files
- [x] Write tests: status transition validation (all valid transitions succeed, invalid transitions fail)
- [x] Write tests: phase sequencing (next phase, terminal phase)
- [x] Write tests: lock acquisition, stale lock detection, concurrent lock prevention

**Verification:**

- [x] `cargo build` succeeds with zero warnings
- [x] `cargo test` passes all type serialization, config, and lock tests
- [x] All enums have exhaustive serde round-trip coverage
- [x] Config loads with defaults when file is missing
- [x] Lock prevents concurrent acquisition
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[002][P1] Feature: Project scaffolding, core types, config, and lock manager`

**Notes:**

Use Rust 2018+ module style (file alongside directory, not `mod.rs`). Follow the rust-style skill guide: clone liberally, enum-based state machines, `#[serde(rename_all = "snake_case")]` for YAML compatibility.

**Followups:**

- [Low] Consider using `chrono::DateTime<Utc>` for `BacklogItem.created`/`updated` instead of `String` — `chrono` with serde is already in deps. Deferred because it changes the serialization format and can be done as a single migration with other type refinements.
- [Low] Consider grouping `blocked_*` fields into `Option<BlockedInfo>` struct to make illegal states unrepresentable. Deferred because it changes the YAML serialization format.
- [Low] Consider introducing a typed error enum instead of `Result<T, String>` — becomes more valuable as more modules are implemented.
- [Low] Consider adding `BacklogItem.id` validation (restrict to `[a-zA-Z0-9_-]`) before the agent runner and git modules are implemented, to prevent potential command injection via malformed IDs.

---

### Phase 2: Backlog Manager

> Full CRUD on BACKLOG.yaml with atomic writes, ID generation, and state machine transitions

**Complexity:** Med

**Goal:** Implement the backlog manager that handles all lifecycle operations on BACKLOG.yaml — the central data store for the orchestrator.

**Files:**

- `.claude/skills/changes/orchestrator/src/backlog.rs` — create — all CRUD and lifecycle operations
- `.claude/skills/changes/orchestrator/tests/backlog_test.rs` — create — comprehensive unit tests
- `.claude/skills/changes/orchestrator/tests/fixtures/` — create directory — sample YAML fixtures

**Tasks:**

- [x] Implement `load(path) -> Result<BacklogFile>` — read BACKLOG.yaml, validate schema_version, deserialize with lenient parsing (warn on unknown fields, don't error)
- [x] Implement `save(path, backlog) -> Result<()>` — atomic write via write-temp-rename: `NamedTempFile::new_in(parent_dir)`, serialize to YAML, `sync_all()`, `persist(path)`
- [x] Implement `generate_next_id(backlog, prefix) -> String` — find highest numeric suffix across all items, increment, format as `{prefix}-{NNN}` (e.g., `WRK-014`). Zero-pad to 3 digits minimum
- [x] Implement `add_item(backlog, title, size, risk) -> BacklogItem` — create new item with status `New`, generated ID, timestamps
- [x] Implement `transition_status(item, new_status) -> Result<()>` — validate transition using `is_valid_transition`, update status field, handle blocked transitions (save `blocked_from_status`), handle unblock (restore from `blocked_from_status`, clear blocked fields)
- [x] Implement `advance_phase(item) -> Result<()>` — validate item is `InProgress`, advance to next sequential phase via `WorkflowPhase::next()`
- [x] Implement `advance_to_phase(item, target_phase) -> Result<()>` — validate prerequisites exist (artifact files for skipped phases), set phase
- [x] Implement `update_assessments(item, assessments) -> ()` — merge non-None assessment fields from PhaseResult into item
- [x] Implement `archive_item(backlog, item_id, worklog_path) -> Result<()>` — prune item from BACKLOG.yaml first, then write worklog entry (crash between = item stays in backlog, safe)
- [x] Implement `ingest_follow_ups(backlog, follow_ups, origin, prefix) -> Vec<BacklogItem>` — generate IDs, create new items with status `New` and origin field set
- [x] Write tests: load valid BACKLOG.yaml with all field variations (full, minimal, with nulls)
- [x] Write tests: load BACKLOG.yaml with unknown fields (should warn, not error)
- [x] Write tests: load BACKLOG.yaml with wrong schema_version (should error)
- [x] Write tests: atomic write — verify file is either old or new version, never partial (write, verify, overwrite, verify)
- [x] Write tests: ID generation — empty backlog, sequential IDs, gap handling, zero-padding
- [x] Write tests: all valid status transitions succeed, all invalid transitions return errors
- [x] Write tests: blocked/unblock cycle from every possible status
- [x] Write tests: phase advancement (sequential, skip with prerequisites, skip without prerequisites fails)
- [x] Write tests: assessment update merging (partial updates, full updates)
- [x] Write tests: archive — item removed from YAML, worklog entry created
- [x] Write tests: follow-up ingestion — correct IDs, correct origin, correct status

**Verification:**

- [x] `cargo test` passes all backlog tests (aim for 30+ test cases)
- [x] Atomic write verified via explicit test
- [x] State machine has 100% transition coverage
- [x] YAML round-trip fidelity confirmed (no field loss, no reordering issues)
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[002][P2] Feature: Backlog manager with CRUD, atomic writes, and state machine`

**Notes:**

The backlog manager is the most depended-upon module. Get the state machine right here — every other module assumes transitions are validated. Use `#[serde(default)]` on all optional fields for forward compatibility.

**Followups:**

- [Low] Worklog `write_archive_worklog_entry` uses plain `fs::write` rather than atomic write-temp-rename. Phase 3 builds out the full worklog module and should address this.
- [Low] `generate_next_id` scans all items per call, making bulk `ingest_follow_ups` O(n²). Acceptable for v1 backlogs but consider caching max ID if scale becomes an issue.

---

### Phase 3: Git Manager, Worklog, and Simple CLI Commands

> Git precondition checks and commits, worklog writing, and the non-pipeline CLI commands

**Complexity:** Med

**Goal:** Complete all leaf modules and wire up the CLI framework with commands that don't require agent spawning: `init`, `add`, `status`, `advance`, `unblock`.

**Files:**

- `.claude/skills/changes/orchestrator/src/git.rs` — create — git operations (preconditions, staging, commits)
- `.claude/skills/changes/orchestrator/src/worklog.rs` — create — worklog entry writing
- `.claude/skills/changes/orchestrator/src/main.rs` — modify — add clap v4 subcommands and command handlers
- `.claude/skills/changes/orchestrator/tests/git_test.rs` — create — git integration tests in temp repos
- `.claude/skills/changes/orchestrator/tests/worklog_test.rs` — create — worklog formatting tests

**Tasks:**

- [x] Implement `git::check_preconditions() -> Result<()>` — verify git repo exists (`git rev-parse --git-dir`), clean working tree (`git status --porcelain` is empty), valid branch (not detached HEAD, no rebase/merge in progress via `git rev-parse`)
- [x] Implement `git::stage_paths(paths: &[&Path]) -> Result<()>` — `git add` with explicit paths only (never `-A` or `.`)
- [x] Implement `git::commit(message: &str) -> Result<String>` — `git commit -m`, return commit hash. If commit fails, return error (caller treats as phase failure)
- [x] Implement `git::get_status() -> Result<Vec<StatusEntry>>` — parse `git status --porcelain` output
- [x] Implement `worklog::write_entry(worklog_dir, item, phase, result_summary) -> Result<()>` — append entry to `_worklog/YYYY-MM.md` (newest at top), create file if missing. Format: datetime, item ID, title, phase, outcome, summary
- [x] Define clap v4 CLI struct with derive macros: `Init { prefix: String }`, `Run { target: Option<String>, cap: u32 }`, `Status`, `Add { title: String, size: Option<String>, risk: Option<String> }`, `Triage`, `Advance { item_id: String, to: Option<String> }`, `Unblock { item_id: String, notes: Option<String> }`
- [x] Implement `init` handler: create `_ideas/`, `_worklog/`, `changes/`, `.orchestrator/` directories; create `BACKLOG.yaml` (empty items, schema_version 1); create `orchestrate.toml` with defaults; append `.orchestrator/` to `.gitignore`
- [x] Implement `add` handler: load backlog, generate ID, create item, save backlog, print confirmation
- [x] Implement `status` handler: load backlog, sort items (in_progress first, then blocked, ready by impact desc, then scoped, researching, new), display formatted table with columns: ID, Status, Phase, Title, Impact, Size, Risk (fixed-width columns, truncate title to fit terminal)
- [x] Implement `advance` handler: load backlog, find item, validate state, advance phase (with prerequisite checks for skipping), save backlog, print confirmation
- [x] Implement `unblock` handler: load backlog, find item, validate it's blocked, restore pre-blocked status, save unblock notes to `unblock_context`, save backlog, print confirmation
- [x] Write tests: git precondition checks (clean tree passes, dirty tree fails, detached HEAD fails)
- [x] Write tests: git staging and commit in temp repos
- [x] Write tests: worklog entry formatting and file creation
- [ ] Write tests: init creates all expected files and directories, and verifies git repo exists (errors with clear message if not)
- [ ] Write tests: CLI argument parsing for each subcommand

**Verification:**

- [x] `cargo build --release` produces working binary
- [x] `orchestrate init` creates correct directory structure and files
- [x] `orchestrate add "Test item"` adds to BACKLOG.yaml and prints confirmation
- [x] `orchestrate status` displays formatted table
- [x] `orchestrate advance` and `orchestrate unblock` work correctly
- [x] All git operations use `--porcelain` for parseable output
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[002][P3] Feature: Git manager, worklog, init/add/status/advance/unblock commands`

**Notes:**

Shell out to `git` CLI for all git operations (design decision). Use `--porcelain` flags for parseable output. The `run` and `triage` commands are stubs at this point — they require the agent runner and pipeline engine from later phases.

**Followups:**

- [Low] Introduce typed error enum (`OrchestrateError`) instead of `Result<T, String>` — deferred because it affects all modules; more valuable once agent runner and pipeline engine introduce more error variants.
- [Low] Add `FromStr` impls for `SizeLevel`, `DimensionLevel`, `WorkflowPhase` instead of parsing in `main.rs` — cleaner separation of concerns, enables clap value parsing.
- [Low] Worklog writes are not atomic (uses `fs::write` directly) — acceptable for v1 since worklog is append-only informational data, not critical state. Consider atomic writes if data loss is observed.
- [Low] Add CLI handler integration tests (`orchestrate init`, `orchestrate add`, etc.) — currently covered by manual verification and unit tests on underlying modules. Worth adding as the CLI surface grows.
- [Low] Consolidate worklog writing between `backlog::archive_item` (uses inline `write_archive_worklog_entry`) and `worklog::write_entry` — two different formatting approaches for the same concept.

---

### Phase 4: Agent Runner

> Subprocess spawning with process group isolation, timeout enforcement, signal handling, and result parsing

**Complexity:** High

**Goal:** Build the agent runner that spawns Claude subprocesses, enforces timeouts, handles signals, and reads structured results. This is the highest-risk infrastructure component.

**Files:**

- `.claude/skills/changes/orchestrator/src/agent.rs` — create — agent runner with subprocess lifecycle
- `.claude/skills/changes/orchestrator/tests/agent_test.rs` — create — tests with mock subprocesses
- `.claude/skills/changes/orchestrator/tests/fixtures/mock_agent_success.sh` — create — mock script that writes result JSON and exits 0
- `.claude/skills/changes/orchestrator/tests/fixtures/mock_agent_fail.sh` — create — mock script that exits 1
- `.claude/skills/changes/orchestrator/tests/fixtures/mock_agent_timeout.sh` — create — mock script that hangs (for timeout tests)
- `.claude/skills/changes/orchestrator/tests/fixtures/mock_agent_bad_json.sh` — create — mock script that writes invalid JSON

**Tasks:**

- [x] Define `AgentRunner` trait: `fn run_agent(&self, prompt: &str, result_path: &Path, timeout: Duration) -> Result<PhaseResult>` — enables mock for pipeline testing
- [x] Implement `CliAgentRunner` (the real implementation): verify `claude` CLI is available (run `claude --version`, fail fast with actionable error if not found), then spawn `claude --dangerously-skip-permissions -p "<prompt>"` as subprocess
- [x] Spawn subprocess in new process group via `process_group(0)` for isolation
- [x] Implement timeout enforcement via `wait-timeout` crate: `child.wait_timeout(duration)`. On timeout: SIGTERM process group via `nix::sys::signal::killpg`, wait 5 seconds, SIGKILL if still alive
- [x] Set up signal handling with `signal-hook`: register SIGTERM and SIGINT handlers that set an `AtomicBool` shutdown flag. Main loop checks flag after subprocess completes
- [x] On shutdown signal: kill subprocess process group, propagate shutdown to caller
- [x] After subprocess exit: check for stale result file at path, delete with warning if exists before reading
- [x] Read result JSON from `.orchestrator/phase_result_{ID}_{PHASE}.json`, validate filename matches expected `{expected_ID}_{expected_PHASE}` pattern, validate structure (all required fields present), deserialize to PhaseResult
- [x] Handle edge cases: non-zero exit with valid result JSON → log warning, respect agent's result code. Non-zero exit without result file → return FAILED. Zero exit without result file → return FAILED
- [x] Delete result file after successful read to prevent stale data
- [x] Implement `MockAgentRunner` for use in pipeline tests — returns predefined PhaseResult values from a configurable sequence
- [x] Write tests: happy path — mock script writes valid JSON, runner returns correct PhaseResult
- [x] Write tests: subprocess failure — mock script exits 1 without JSON, runner returns FAILED
- [x] Write tests: timeout — mock script hangs, runner kills after timeout, returns FAILED
- [x] Write tests: malformed JSON — mock script writes invalid JSON, runner returns FAILED
- [x] Write tests: stale result file cleanup — pre-existing file at path is deleted before spawn
- [x] Write tests: valid JSON with non-zero exit — runner respects result code from JSON

**Verification:**

- [x] All agent runner tests pass
- [x] Process group kill confirmed via test (subprocess descendants are cleaned up)
- [x] Timeout enforcement confirmed (test completes in ~timeout duration, not hanging)
- [x] Signal handler installed without panics
- [x] MockAgentRunner returns predefined results correctly for test sequences (verified via dedicated test)
- [x] Agent runner returns clear error when Claude CLI not found on PATH
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[002][P4] Feature: Agent runner with process groups, timeouts, and signal handling`

**Notes:**

This is the highest technical risk phase. The `wait-timeout` crate simplifies timeout enforcement significantly vs. a manual watchdog thread. Test with mock shell scripts, not real Claude invocations — that validation happens in Phase 6 integration testing. The `AgentRunner` trait is critical: it enables all pipeline tests to run without spawning real subprocesses.

**Followups:**

- [Low] Consider capturing subprocess stderr to a bounded buffer and including the tail in error messages for diagnosability. Currently stdout/stderr are both null'd, making agent failures hard to debug.
- [Low] `MockAgentRunner` lives in production source (`src/agent.rs`) rather than behind `#[cfg(test)]` or in a test-support crate. Acceptable for now since pipeline tests in Phase 6 need it, but consider moving to a shared test utilities module if more mocks accumulate.
- [Low] The global `shutdown_flag()` singleton cannot be reset, making the orchestrator non-restartable within a single process. Fine for CLI usage; worth revisiting if the library is ever used in a long-running process.

---

### Phase 5: Prompt Builder

> Construct autonomous prompts for all pipeline phases and pre-workflow agents

**Complexity:** Med

**Goal:** Build the prompt builder that wraps skill invocations with autonomous instructions and structured output requirements. Also define the triage and research agent prompt templates.

**Files:**

- `.claude/skills/changes/orchestrator/src/prompt.rs` — create — prompt construction for all phases
- `.claude/skills/changes/orchestrator/tests/prompt_test.rs` — create — prompt assembly tests

**Tasks:**

- [x] Implement `build_prompt(phase, item, config, result_path, previous_summary, unblock_notes, failure_context) -> String` — constructs the full prompt string. `failure_context: Option<String>` carries the previous failure summary when retrying a phase
- [x] Implement autonomous preamble section: context (running autonomously, no user input), item info (ID, title), previous phase summary, current assessments, unblock notes if resuming from blocked, failure context if retrying. Include instruction: "Record questions you would normally ask in an 'Assumptions' section of the artifact, documenting decisions made without human input"
- [x] Implement skill invocation section: map `WorkflowPhase` to the correct skill command (`/changes:0-prd:create-prd`, `/changes:1-tech-research:tech-research`, etc.) with correct arguments (change folder path, mode)
- [x] Implement structured output suffix: JSON schema for PhaseResult, result file path, instructions to write the file, instruction to record assumptions in artifacts, instruction to report follow-ups
- [x] Implement `build_triage_prompt(item, result_path) -> String` — custom prompt for triage agent: assess size/complexity/risk/impact, decide routing (idea file or direct promote), create idea file if needed, set requires_human_review
- [x] Implement `build_research_prompt(item, idea_file_path, result_path) -> String` — custom prompt for research agent: read idea file, research problem space, update idea file, determine if completion criteria met
- [x] Implement `build_build_prompt(item, spec_path, result_path) -> String` — specialized build prompt that tells agent to find next incomplete SPEC phase (`- [ ]` tasks), execute it, mark tasks `- [x]`, return SUBPHASE_COMPLETE or PHASE_COMPLETE
- [x] Write tests: prompt includes failure_context when retrying
- [x] Write tests: prompt includes assumptions instruction in autonomous preamble
- [x] Write tests: prompt assembly for each workflow phase contains correct skill command
- [x] Write tests: prompt includes result file path in suffix
- [x] Write tests: prompt includes previous summary when provided
- [x] Write tests: prompt includes unblock notes when provided
- [x] Write tests: triage prompt contains assessment instructions
- [x] Write tests: build prompt references SPEC path and checkbox conventions

**Verification:**

- [x] All prompt builder tests pass
- [x] Each workflow phase maps to the correct skill command
- [x] Prompts contain the JSON schema in the suffix
- [x] Result file path is correctly embedded in prompts
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[002][P5] Feature: Prompt builder for all pipeline and pre-workflow phases`

**Notes:**

The exact prompt wording will need iteration based on real agent testing. This phase creates the initial versions; Phase 6 validates them. The prompt structure follows the design: `[Autonomous Preamble] + [Skill Invocation] + [Structured Output Suffix]`. Pre-workflow agents (triage, research) use custom prompts rather than skill wrappers.

**Followups:**

- [Low] The `.replace("## Item", "## Item to Triage")` pattern in triage/research prompts is fragile — if item data ever contains `## Item`, the replacement would produce unintended results. Consider passing the item heading as a parameter to `build_preamble` instead.
- [Low] Research/design depth is hardcoded to "medium" in `build_skill_invocation` instead of being driven by item assessments. Consider deriving depth from size/complexity when downstream skills support variable depth levels.
- [Low] `build_build_prompt` accepts 6 positional parameters (3 of which are `Option<&str>`). Consider using a params struct similar to `PromptParams` if more optional parameters are added.

---

### Phase 6: Pipeline Engine & Run Command

> Full orchestration loop with item selection, phase execution, retry, circuit breaker, guardrails, follow-up triage, and the `run`/`triage` CLI commands

**Complexity:** High

**Goal:** Implement the core pipeline engine that wires all components together, plus the `run` and `triage` commands that exercise it.

**Files:**

- `.claude/skills/changes/orchestrator/src/pipeline.rs` — create — orchestration loop and all sub-logic
- `.claude/skills/changes/orchestrator/src/main.rs` — modify — wire `run` and `triage` commands to pipeline
- `.claude/skills/changes/orchestrator/tests/pipeline_test.rs` — create — pipeline tests with MockAgentRunner

**Tasks:**

- [x] Implement item selection: sort ready items by impact (descending), then created date (ascending, oldest first). Return highest-impact item. If `--target` specified, find that specific item
- [x] Implement artifact path resolution: map item_id to change folder path (`changes/{item_id}_{slug}/`), resolve PRD/RESEARCH/DESIGN/SPEC file paths using the `{item_id}_{slug}_{TYPE}.md` convention
- [x] Implement pre-workflow state transitions: triage agent result determines `new → researching` (needs idea file) or `new → scoped` (direct promote for small/low-risk items). Research agent determines `researching → scoped`. Define idea file path convention: `_ideas/{item_id}_{slug}.md`
- [x] Implement auto-promote logic: items in `scoped` status that pass all guardrail thresholds AND have `requires_human_review: false` auto-transition to `ready` with phase set to `Prd`
- [x] Implement `requires_human_review` check: during pre-workflow handling, if item has `requires_human_review: true`, block immediately with `block_type: Decision` before entering workflow pipeline
- [x] Implement pre-workflow handling for `--target` on pre-ready items: spawn triage if `new`, research if `researching`, evaluate guardrails and auto-promote if `scoped`
- [x] Implement phase execution loop (universal for all phases): build prompt → spawn agent → wait for result → handle result code
- [x] Implement `SUBPHASE_COMPLETE` handling: triage follow-ups, commit checkpoint (change folder + BACKLOG.yaml), loop back to same phase
- [x] Implement `PHASE_COMPLETE` handling: triage follow-ups, commit checkpoint, advance to next workflow phase
- [x] Implement `PHASE_COMPLETE` summary extraction: capture summary from PhaseResult and pass as `previous_summary` to the next phase's prompt builder
- [x] Implement `FAILED` handling: increment retry counter, if under max_retries re-run phase with fresh agent (pass previous failure summary as `failure_context` to prompt builder), if exhausted mark item blocked
- [x] Implement `BLOCKED` handling: mark item blocked with reason and type from result, store blocked_from_status, skip to next ready item
- [x] Implement circuit breaker: track consecutive items that exhaust retries. Counter increments when item exhausts all retries. Resets to 0 when any item completes a phase successfully. Trips when counter reaches 2 — halt run
- [x] Implement phase cap enforcement: count every agent spawn (including retries) toward `--cap N`. When cap reached, commit current work if applicable, exit cleanly
- [x] Implement guardrail re-evaluation: after each phase's assessment update, check item's size/complexity/risk against config thresholds. If exceeded, block for human review, preserve completed artifacts
- [x] Implement follow-up triage: parse follow_ups from PhaseResult, generate sequential IDs, add to backlog with status `new` and origin = `{item_id}/{phase}`
- [x] Implement archive: after review phase completes, prune item from BACKLOG.yaml, write worklog entry, commit `[WRK-001][ARCHIVE] Completed: {title}`
- [x] Implement the outer loop: after item completes/blocks, return to item selection. Exit when no actionable items remain or cap reached
- [x] Implement crash recovery: on startup with an `in_progress` item, log message ("Resuming WRK-001 at {phase}") and re-run current phase
- [x] Wire `run` command: acquire lock, check git preconditions, load config + backlog, call pipeline with target/cap, release lock, print summary
- [x] Wire `triage` command: find all `new` items, run triage agent for each sequentially, commit after each
- [x] Implement terminal output to stderr: `[WRK-001][PRD] Starting phase (attempt 1/3)` before each spawn, `[WRK-001][PRD] Result: PHASE_COMPLETE — {summary}` after completion, `Progress: 3/10 phases used` after each spawn, `Follow-ups: 2 new items added to backlog` when follow-ups are ingested, `Circuit breaker: 1/2 consecutive failures` when applicable
- [x] Write tests: item selection picks highest impact, tiebreaks by created date
- [x] Write tests: pre-workflow state transitions (new → researching, new → scoped, researching → scoped, scoped auto-promote)
- [x] Write tests: requires_human_review blocks item before pipeline entry
- [x] Write tests: previous_summary passed correctly between phases
- [x] Write tests: failure_context passed correctly on retry
- [x] Write tests: artifact path resolution maps item_id to correct file paths
- [x] Write tests: happy path — single item through all 6 phases (mock returns PHASE_COMPLETE for each)
- [x] Write tests: build sub-loop — mock returns SUBPHASE_COMPLETE 3 times then PHASE_COMPLETE
- [x] Write tests: retry — mock returns FAILED, then PHASE_COMPLETE on retry
- [x] Write tests: retry exhaustion — mock returns FAILED for max_retries+1, item becomes blocked
- [x] Write tests: circuit breaker — two consecutive items exhaust retries, pipeline halts
- [x] Write tests: BLOCKED — mock returns BLOCKED, item is blocked, pipeline moves to next item
- [x] Write tests: guardrail trip — mock returns updated assessments exceeding guardrails, item blocks
- [x] Write tests: follow-up triage — mock returns follow_ups, new items appear in backlog with correct origin
- [x] Write tests: phase cap — pipeline exits after cap agent spawns
- [x] Write tests: crash recovery — start with in_progress item, verify pipeline resumes at correct phase

**Verification:**

- [x] All pipeline tests pass with MockAgentRunner
- [x] `orchestrate run --cap 5` works end-to-end with mock (or exits cleanly with no items)
- [x] `orchestrate triage` processes new items
- [x] Circuit breaker halts after 2 consecutive exhaustions
- [x] Guardrail re-evaluation blocks items correctly
- [x] Follow-ups are ingested with correct IDs and origin
- [x] Terminal output shows progress information
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[002][P6] Feature: Pipeline engine with retry, circuit breaker, guardrails, and run/triage commands`

**Notes:**

This is the most complex phase. The pipeline engine should be built incrementally: start with the happy path (PHASE_COMPLETE loop), then layer on error handling (FAILED + retry), then BLOCKED, then SUBPHASE_COMPLETE, then circuit breaker, then guardrails. Each addition should compile and pass existing tests before proceeding.

All tests use `MockAgentRunner`. Real Claude invocations are integration testing done after this SPEC.

**Followups:**

- [Medium] **Archive-before-commit crash safety gap** — `archive_item` removes the item from BACKLOG.yaml and writes to disk before the git commit. If the process crashes between save and commit, the item is lost from the backlog but not recorded in git. Consider restructuring so the commit happens immediately after the backlog save.
- [Medium] **In-progress item with no phase fatally errors** — If a hand-edited backlog has an item in `in_progress` status with no `phase` field, the pipeline returns a fatal error. Consider defaulting to `Prd` phase or blocking the item gracefully.
- [Low] **GuardrailTripped does not reset consecutive_exhaustions** — A guardrail trip between two retry exhaustions still counts toward the circuit breaker threshold. Guardrail trips are a different failure mode and should arguably reset the counter.
- [Low] **Item ID validation for filesystem paths** — Item IDs are interpolated into file paths without sanitization. While IDs are normally generated safely, hand-edited YAML could contain path traversal characters. Consider validating IDs match `[a-zA-Z0-9_-]+`.
- [Low] **run_triage has no phase cap or circuit breaker** — The standalone triage command processes all `New` items with no backpressure. A large backlog with a slow/failing agent would run indefinitely.
- [Low] **Worklog write failure silently ignored** — The `let _ =` pattern in the PhaseComplete handler swallows worklog write errors (disk full, permission denied).
- [Low] **Function complexity** — `execute_item_pipeline` (~300 lines) and `handle_pre_workflow` (~130 lines) are complex. Consider extracting phase result handling into separate functions if more result codes are added.

---

### Phase 7: Naming Convention Migration

> Update 11 skill/template files from `NNN_` to `WRK-001_` format

**Complexity:** Low

**Goal:** Migrate all existing skill and template files to the new `WRK-001_descriptive-name` naming convention so the orchestrator and interactive skills use the same format.

**Files:**

- `.claude/skills/changes/SKILL.md` — modify — update naming convention examples, folder structure, AI naming instructions
- `.claude/skills/changes/workflow-guide.md` — modify — update naming convention references throughout
- `.claude/skills/changes/workflows/0-prd/create-prd.md` — modify — update folder/file creation instructions
- `.claude/skills/changes/workflows/0-prd/interview-prd.md` — modify — update file path references
- `.claude/skills/changes/workflows/0-prd/discovery-research.md` — modify — update file path references
- `.claude/skills/changes/workflows/1-tech-research/tech-research.md` — modify — update file path references
- `.claude/skills/changes/workflows/2-design/design.md` — modify — update file path references
- `.claude/skills/changes/workflows/3-spec/create-spec.md` — modify — update file path references
- `.claude/skills/changes/templates/spec-template.md` — modify — update naming convention in template
- `.claude/skills/changes/templates/design-template.md` — modify — update naming convention in template
- `.claude/skills/changes/templates/tech-research-template.md` — modify — update naming convention in template
- `.claude/skills/changes/templates/idea-template.md` — create — move idea file template to skills folder (from `changes/002_.../ideas/IDEA_TEMPLATE.md`)

**Tasks:**

- [x] In `SKILL.md`: replace all `NNN_featurename` examples with `WRK-001_featurename` format. Update folder structure examples. Update "Before Creating a Change" instructions to use configurable prefix. Update valid/invalid examples
- [x] In `workflow-guide.md`: replace all `NNN_featurename` references with `WRK-001_featurename`. Update directory structure example. Update command examples
- [x] In `create-prd.md`: update folder/file creation instructions to use `WRK-001_` format
- [x] In `interview-prd.md`: update file path references
- [x] In `discovery-research.md`: update file path references
- [x] In `tech-research.md`: update file path references
- [x] In `design.md`: update file path references
- [x] In `create-spec.md`: update file path references
- [x] In `spec-template.md`: update `NNN` references to `WRK-001` style in header and examples
- [x] In `design-template.md`: update `NNN` references
- [x] In `tech-research-template.md`: update `NNN` references
- [x] Copy idea file template from `changes/002_.../ideas/IDEA_TEMPLATE.md` to `.claude/skills/changes/templates/idea-template.md`
- [x] In `spec-template.md`: add machine-parseable phase status line (`**Phase Status:** not_started | in_progress | complete`) to each phase section template — required for orchestrator crash recovery reconciliation
- [x] Remove or update `changes/_TEMPLATE_NNN_featurename/` folder since `orchestrate init` replaces its function
- [x] Verify each modified file is internally consistent (no mixed old/new naming)

**Verification:**

- [x] All 11 files updated with consistent `WRK-001_` naming
- [x] No remaining `NNN_featurename` references in modified files (grep to verify)
- [x] Idea file template exists at `.claude/skills/changes/templates/idea-template.md` with required sections (problem statement, proposed approach, size/complexity/risk assessment)
- [x] Spec template includes machine-parseable `**Phase Status:**` line in each phase section
- [x] `_TEMPLATE_NNN_featurename/` folder removed or updated
- [x] Existing change folders (like `002_overhaul-changes-workflow-orchestrator/`) are NOT renamed (only convention in instructions changes, not existing artifacts)
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[002][P7] Docs: Migrate naming convention from NNN_ to WRK-001_ across 11 skill files`

**Notes:**

This is a text-replacement task, not a code change. Existing change folders are NOT renamed — the PRD says "clean break" means new items use the new convention, not that old items are migrated. The `_TEMPLATE_NNN_featurename/` folder in `changes/` should be updated or removed since the orchestrator's `init` command replaces it.

**Followups:**

---

## Final Verification

- [x] All phases complete (7/7 — see Execution Log)
- [x] All PRD success criteria met (41/41 Must Have, 2/3 Should Have)
- [x] Tests pass (187 tests, 0 failures, 0 warnings)
- [x] No regressions introduced (git status clean, existing workflow unchanged)
- [x] Code reviewed (each phase reviewed per Execution Log)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| P1 | Complete | [002][P1] | 39 tests, 0 warnings, 0 clippy. Code review: fixed TOCTOU lock race, PhaseResult.phase typing, config defaults, is_valid_transition simplification, added Eq/must_use. |
| P2 | Complete | [002][P2] | 54 backlog tests (93 total), 0 warnings, 0 clippy. Code review: fixed unblock_context not cleared, TOCTOU in dir creation, added unblock_context test. Moved tempfile to regular deps for atomic writes. |
| P3 | Complete | [002][P3] | 14 git + 4 worklog tests (107 total), 0 warnings, 0 clippy. Code review: fixed UTF-8 truncation panic, TOML prefix injection, consolidated git API (removed dual `_in` pattern), simplified handle_unblock, deduplicated rev-parse and gitignore reads. 2 SPEC tasks deferred (init integration tests, CLI arg parsing tests). |
| P4 | Complete | [002][P4] | 16 agent tests (127 total), 0 warnings, 0 clippy. Code review: replaced unconditional 5s sleep with polling kill_process_group, eliminated TOCTOU races in stale file deletion and result file read, removed unnecessary PhaseResult clone, added SAFETY comment for unsafe pre_exec block, extracted named constants for grace period/poll interval. |
| P5 | Complete | [002][P5] | 36 prompt tests (163 total), 0 warnings, 0 clippy. Code review: fixed format_assessments lowercasing bug (added Display impls for SizeLevel/DimensionLevel), replaced Debug-as-serialization-contract with WorkflowPhase::as_str(), extracted shared build_preamble helper (eliminated 4x duplication), removed unused config field from PromptParams, fixed wildcard test imports, added partial-assessment and assessment-exclusion tests. |
| P6 | Complete | [002][P6] | 24 pipeline tests (187 total), 0 warnings, 0 clippy. Code review: extracted CIRCUIT_BREAKER_THRESHOLD constant, replaced sort_by+first with max_by for item selection, removed redundant continue in StillPreWorkflow, fixed case-insensitive blocked_type detection with default Clarification, added staged-changes check in commit_checkpoint, removed misleading attempt counters from pre-workflow log messages. 7 items deferred to followups. |
| P7 | Complete | [002][P7] | 16 files updated (11 SPEC-listed + 5 additional found by code review: critique-prd, critique-design, critique-spec, change-review, implement-spec-autonomous). 1 new file (idea-template.md). _TEMPLATE_NNN_featurename/ already absent. All `NNN_` references replaced with `WRK-001_`. Phase Status line added to spec-template.md. |

## Followups Summary

_Aggregated from all 7 phases during change review (2026-02-11)._

### Critical

(none)

### High

(none)

### Medium

- [ ] **Archive-before-commit crash safety gap** (P6) — `archive_item` removes item from BACKLOG.yaml and saves to disk before the git commit. Process crash between save and commit loses the item from the backlog without git recording it. Consider restructuring so the commit happens immediately after the backlog save.
- [ ] **In-progress item with no phase fatally errors** (P6) — A hand-edited backlog with an `in_progress` item missing the `phase` field causes a fatal pipeline error. Consider defaulting to Prd phase or blocking the item gracefully.
- [ ] **Test infrastructure helpers** — Create shared test utilities: `setup_temp_repo()` for git integration tests, fixture loader for YAML/JSON test data. Would reduce boilerplate across 9 test files.

### Low

- [ ] **Typed error enum** (P1, P3) — Replace `Result<T, String>` with `OrchestrateError` enum across all modules for structured error handling and better caller ergonomics.
- [ ] **Item ID validation** (P1, P6) — IDs interpolated into file paths without sanitization. Hand-edited YAML could contain path traversal characters. Validate IDs match `[a-zA-Z0-9_-]+`.
- [ ] **chrono::DateTime for timestamps** (P1) — Use `chrono::DateTime<Utc>` for BacklogItem created/updated instead of String. Chrono is already a dependency.
- [ ] **Group blocked fields into Option\<BlockedInfo\>** (P1) — Make illegal states unrepresentable by combining blocked_from_status, blocked_reason, blocked_type into a single optional struct.
- [ ] **Worklog writes not atomic** (P2, P3) — Uses `fs::write` directly instead of write-temp-rename. Acceptable for append-only informational data.
- [ ] **generate_next_id O(n²) for bulk ingestion** (P2) — Scans all items per call in `ingest_follow_ups`. Consider caching max ID if scale becomes an issue.
- [ ] **FromStr impls for enums** (P3) — Add `FromStr` for SizeLevel, DimensionLevel, WorkflowPhase instead of parsing in main.rs. Enables clap value parsing.
- [ ] **CLI handler integration tests** (P3) — Deferred SPEC tasks: init creates expected files/dirs, CLI argument parsing. Currently covered by manual verification + unit tests.
- [ ] **Consolidate worklog writing** (P3) — Two different approaches: `backlog::archive_item` inline writes vs `worklog::write_entry`. Should use one.
- [ ] **Capture subprocess stderr** (P4) — stdout/stderr currently null'd, making agent failures hard to debug. Consider bounded buffer with tail in error messages.
- [ ] **MockAgentRunner in production source** (P4) — Lives in `src/agent.rs` rather than behind `#[cfg(test)]` or a test-support crate. Pipeline tests need it, but could move to shared test module.
- [ ] **Global shutdown_flag non-restartable** (P4) — AtomicBool singleton cannot be reset. Fine for CLI; revisit if library use is needed.
- [ ] **Fragile .replace() in prompts** (P5) — `## Item` replacement could match content in item data. Consider parameterized heading in `build_preamble`.
- [ ] **Research/design depth hardcoded** (P5) — Hardcoded to "medium" in `build_skill_invocation`. Consider deriving from item assessments.
- [ ] **build_build_prompt positional params** (P5) — 6 positional parameters (3 optional). Consider params struct if more are added.
- [ ] **GuardrailTripped doesn't reset circuit breaker** (P6) — A guardrail trip between two retry exhaustions still counts toward the circuit breaker. Arguably a different failure mode.
- [ ] **run_triage has no cap or circuit breaker** (P6) — Processes all `New` items with no backpressure. Large backlog with failing agent runs indefinitely.
- [ ] **Worklog write failure silently ignored** (P6) — `let _ =` pattern swallows disk full / permission denied errors.
- [ ] **Function complexity** (P6) — `execute_item_pipeline` (~300 lines) and `handle_pre_workflow` (~130 lines). Consider extracting sub-functions if more result codes are added.
- [ ] **Rate limit / backoff handling** — Agent runner treats API rate limits as FAILED (retry handles). Consider explicit detection and exponential backoff.
- [ ] **Config validation on init** (PRD Should Have) — `orchestrate init` creates config but does not validate that the skills folder structure is correct.

## Design Details

### Key Types

```rust
// All enums derive: Serialize, Deserialize, Clone, Debug, PartialEq
// Use #[serde(rename_all = "snake_case")] for YAML compatibility

enum ItemStatus {
    New, Researching, Scoped, Ready, InProgress, Done, Blocked,
}

enum WorkflowPhase {
    Prd, Research, Design, Spec, Build, Review,
}

enum ResultCode {
    SubphaseComplete, PhaseComplete, Failed, Blocked,
}

enum BlockType {
    Clarification, Decision,
}

enum SizeLevel {
    Small, Medium, Large,
}

enum DimensionLevel {
    Low, Medium, High,
}

struct BacklogItem {
    id: String,
    title: String,
    status: ItemStatus,
    phase: Option<WorkflowPhase>,
    size: Option<SizeLevel>,
    complexity: Option<DimensionLevel>,
    risk: Option<DimensionLevel>,
    impact: Option<DimensionLevel>,
    requires_human_review: bool,
    origin: Option<String>,
    blocked_from_status: Option<ItemStatus>,
    blocked_reason: Option<String>,
    blocked_type: Option<BlockType>,
    unblock_context: Option<String>,
    tags: Vec<String>,
    dependencies: Vec<String>,
    created: String,
    updated: String,
}

struct BacklogFile {
    schema_version: u32,
    items: Vec<BacklogItem>,
}

struct PhaseResult {
    item_id: String,
    phase: String,
    result: ResultCode,
    summary: String,
    context: Option<String>,
    updated_assessments: Option<UpdatedAssessments>,
    follow_ups: Vec<FollowUp>,
}

struct FollowUp {
    title: String,
    context: Option<String>,
    suggested_size: Option<SizeLevel>,
    suggested_risk: Option<DimensionLevel>,
}

struct UpdatedAssessments {
    size: Option<SizeLevel>,
    complexity: Option<DimensionLevel>,
    risk: Option<DimensionLevel>,
    impact: Option<DimensionLevel>,
}

struct OrchestrateConfig {
    project: ProjectConfig,
    guardrails: GuardrailsConfig,
    execution: ExecutionConfig,
}

// AgentRunner trait for testability
trait AgentRunner {
    fn run_agent(&self, prompt: &str, result_path: &Path, timeout: Duration) -> Result<PhaseResult>;
}

// Prompt builder signature (failure_context for retries, previous_summary for phase continuity)
// fn build_prompt(phase, item, config, result_path, previous_summary: Option<&str>,
//                 unblock_notes: Option<&str>, failure_context: Option<&str>) -> String
```

### Architecture Details

The system follows the design document's architecture exactly:

```
orchestrate CLI (clap v4)
        │
  ┌─────┼─────────┐
  │     │          │
init   run     status/add/triage/advance/unblock
        │
  Pipeline Engine
  ┌─────┼─────────┐
  │               │
Pre-Workflow   Workflow Pipeline
(Triage,       (PRD → Research → Design →
 Research)      Spec → Build → Review)
  │               │
  └─────┬─────────┘
        │
  Agent Runner (subprocess per phase)
  ┌─────┼─────────┐
  │     │          │
Prompt  Git     Backlog
Builder Manager Manager
  │     │          │
claude  git     BACKLOG.yaml
-p      CLI     (atomic writes)
```

### Design Rationale

See the full design document at `./002_overhaul-changes-workflow-orchestrator_DESIGN.md` for:
- 10 technical decisions with context, rationale, and consequences
- 2 alternatives considered (async runtime, TOML backlog) with rejection reasons
- 6 risks with mitigations
- Tradeoffs matrix (sync-only, high commit volume, Rust toolchain required, etc.)

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
