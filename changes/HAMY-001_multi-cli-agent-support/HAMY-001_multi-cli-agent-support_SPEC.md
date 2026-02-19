# SPEC: Multi-CLI Agent Support

**ID:** HAMY-001
**Status:** Draft
**Created:** 2026-02-17
**PRD:** ./HAMY-001_multi-cli-agent-support_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

Phase-golem currently hardcodes `claude` as the only AI CLI tool for executing pipeline phases. The `CliAgentRunner` is a unit struct that constructs `Command::new("claude")` with Claude-specific arguments. This change adds a `CliTool` enum and `AgentConfig` struct so users can configure which CLI tool runs their phases via `[agent]` in `phase-golem.toml`. Default behavior remains `claude` with no config changes required.

The design is complete (see DESIGN.md). Key architectural decisions: method-based dispatch on `CliTool` enum, `CliAgentRunner` becomes a parameterized struct, `AgentRunner` trait is unchanged, `MockAgentRunner` is unaffected.

## Approach

Add a `CliTool` enum (with `Claude` and `OpenCode` variants) and `AgentConfig` struct to `config.rs`. The enum owns all tool-specific invocation knowledge via `build_args()`, `binary_name()`, and `display_name()` methods. `CliAgentRunner` becomes `CliAgentRunner { tool: CliTool, model: Option<String> }`, constructed from `config.agent` at each call site. The config loading pipeline gains a normalization step for model strings and validation for flag-prefix rejection.

The change boundary is narrow: config parsing (new types), command construction (inside `CliAgentRunner::run_agent`), and call sites (`handle_run`, `handle_triage`, `handle_init`). Everything downstream — subprocess management, result file reading, scheduling, execution — is untouched.

**Patterns to follow:**

- `src/config.rs:42-49` (`StalenessAction` enum) — derive/serde pattern for `CliTool` enum (NOTE: `CliTool` uses `rename_all = "lowercase"`, not `snake_case` like `StalenessAction`, because multi-word variants like `OpenCode` must serialize as `opencode` not `open_code`)
- `src/config.rs:33-40` (`ExecutionConfig`) — `#[serde(default)]` struct pattern for `AgentConfig`
- `src/config.rs:162-220` (`validate()`) — error accumulator pattern for model validation
- `src/config.rs:267-296` (`load_config()`) — config loading flow for normalization insertion
- `tests/config_test.rs` — TOML string → `load_config()` → assert field values pattern for tests

**Implementation boundaries:**

- Do not modify: `AgentRunner` trait signature, `MockAgentRunner`, `run_subprocess_agent()`, `read_result_file()`, `PhaseResult` struct
- Do not modify: `src/scheduler.rs` (uses `Arc<impl AgentRunner>` generically — unaffected)
- Do not modify: `src/coordinator.rs`, `src/prompt.rs`, `src/types.rs`, `src/backlog.rs`, `src/git.rs`, `src/preflight.rs`
- Do not refactor: The dual `load_config`/`load_config_at` pattern (just add normalization to both)
- After completing this change, update the PRD open questions to mark OpenCode invocation pattern as resolved (per tech research findings)

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Config Foundation & Init Template | Medium | Add `CliTool` enum, `AgentConfig` struct, `PhaseConfig` serde fixes, normalization, validation, config loading updates, `handle_init` template fixes, and all config tests |
| 2 | Agent Runner & Integration | Medium | Parameterize `CliAgentRunner`, update `verify_cli_available` and `run_agent`, reorder startup in `handle_run`/`handle_triage`, construct runner from config, post-result validation |
| 3 | Observability | Low | Add startup and per-phase logging, update log messages to use configured tool's display name |

**Ordering rationale:** Phase 1 defines the types that Phase 2 consumes (`CliTool` imported by `agent.rs`) and co-locates the `handle_init` template fix with `deny_unknown_fields` to maintain atomicity. Phase 2 must update both `agent.rs` and `main.rs` call sites together since parameterizing `CliAgentRunner` breaks compilation of call sites; it also adds post-result validation since `executor.rs` is being modified and the validation is correctness logic tied to the agent runner. Phase 3 is pure observability/docs that builds on the functional changes from Phases 1-2.

---

## Phases

Each phase should leave the codebase in a functional, stable state. Complete and verify each phase before moving to the next.

---

### Phase 1: Config Foundation & Init Template

> Add `CliTool` enum, `AgentConfig` struct, config integration, normalization, validation, `handle_init` template fixes, and tests

**Phase Status:** complete

**Complexity:** Medium

**Goal:** Define all new config types, wire them into the config loading pipeline with normalization and validation, fix the `PhaseConfig` `destructive`/`is_destructive` serde bug, update the `handle_init` template to use `is_destructive` and include the `[agent]` section, and thoroughly test everything. After this phase, `PhaseGolemConfig` has an `agent: AgentConfig` field that loads correctly from TOML (or defaults when absent), and `handle_init` generates parseable configs under `deny_unknown_fields`.

**Files:**

- `src/config.rs` — modify — Add `CliTool` enum with methods, `AgentConfig` struct, `agent` field on `PhaseGolemConfig`, `normalize_agent_config()` helper, model validation in `validate()`, normalization calls in `load_config()`/`load_config_at()`, `#[serde(alias = "destructive")]` + `#[serde(deny_unknown_fields)]` on `PhaseConfig`, `#[serde(deny_unknown_fields)]` on `AgentConfig`
- `src/main.rs` — modify — Update `handle_init` template: add `[agent]` section, fix `destructive` → `is_destructive` field names in generated TOML
- `tests/config_test.rs` — modify — Add tests for CliTool methods, AgentConfig deserialization, normalization, validation, backwards compatibility, deny_unknown_fields, PhaseConfig alias, init template round-trip parse

**Patterns:**

- Follow `StalenessAction` enum (config.rs:42-49) for `CliTool` derive set and serde attributes
- Follow `ExecutionConfig` (config.rs:33-40, 104-114) for `AgentConfig` serde defaults
- Follow existing `validate()` error accumulation pattern for model validation

**Tasks:**

- [x] Add `CliTool` enum to `config.rs` with `Claude` and `OpenCode` variants. Derives: `Default, Deserialize, Clone, Debug, PartialEq, Eq`. Attributes: `#[serde(rename_all = "lowercase")]` (NOT `snake_case` — `snake_case` would serialize `OpenCode` as `open_code` instead of `opencode`), `#[default]` on `Claude`. Implement methods: `binary_name() -> &str`, `display_name() -> &str`, `build_args(&self, prompt: &str, model: Option<&str>) -> Vec<String>`, `version_args() -> Vec<&str>`, `install_hint() -> &str` (returns install suggestion URL/command, e.g., `"Install: https://docs.anthropic.com/en/docs/claude-code"` for Claude, `"Install: https://github.com/opencode-ai/opencode"` for OpenCode).
- [x] Add `AgentConfig` struct to `config.rs` with fields `cli: CliTool` and `model: Option<String>`. Derives: `Default, Deserialize, Clone, Debug, PartialEq`. Attributes: `#[serde(default, deny_unknown_fields)]`.
- [x] Add `pub agent: AgentConfig` field to `PhaseGolemConfig` (between `execution` and `pipelines`)
- [x] Add `#[serde(alias = "destructive")]` to `PhaseConfig.is_destructive` AND `#[serde(deny_unknown_fields)]` to `PhaseConfig` struct — these MUST be applied together in the same edit. Without the alias, `deny_unknown_fields` would reject existing configs that use `destructive` instead of `is_destructive`. Before applying, audit all existing test TOML fixtures in `tests/config_test.rs` (and any other test files) for unknown fields in `PhaseConfig` blocks that would now be rejected.
- [x] Add `normalize_agent_config(config: &mut PhaseGolemConfig)` helper function. Trims `config.agent.model` and normalizes empty/whitespace-only strings to `None`.
- [x] Add model validation to `validate()`: reject `config.agent.model` values that do not match the allowlist pattern `^[a-zA-Z0-9._/-]+$` (per PRD). This rejects whitespace, control characters, empty strings, and flag-like prefixes (`--`, `-`) as defense-in-depth. Use a simple character-by-character check (or `regex` crate if already a dependency; otherwise avoid adding a dep for this). Error message: `"agent.model contains invalid characters (allowed: letters, digits, '.', '_', '/', '-')"`. Note: this is stricter than just rejecting flag prefixes — it also rejects spaces, special chars, etc.
- [x] Call `normalize_agent_config(&mut config)` in `load_config()` — insert after `toml::from_str` (line 279), before `populate_default_pipelines` (line 282)
- [x] Call `normalize_agent_config(&mut config)` in `load_config_at()` — insert after `toml::from_str` (line 248), before `populate_default_pipelines` (line 251)
- [x] Update `handle_init()` template in `main.rs`: Add `[agent]` section between `[execution]` and `[pipelines.feature]` with commented defaults:
  ```toml
  [agent]
  # cli = "claude"          # AI CLI tool: "claude", "opencode"
  # model = ""              # Model override (e.g., "opus", "sonnet")
  ```
- [x] Fix `handle_init()` template in `main.rs`: Change all `destructive = false` to `is_destructive = false` and `destructive = true` to `is_destructive = true` in the generated pipeline TOML. There are 5 phases with `destructive = false` and 1 with `destructive = true` in the main `phases` array (the `pre_phases` entry does not have a `destructive` field, which is fine — serde will use the default). Count and verify each occurrence (6 total replacements) before committing. NOTE: Also added `is_destructive = false` to the pre_phases entry because `deny_unknown_fields` on `PhaseConfig` requires all required fields to be present (the round-trip test caught this).
- [x] Write tests for `CliTool`: `binary_name()` returns correct values, `display_name()` returns correct values, `build_args()` for Claude without model, `build_args()` for Claude with model, `build_args()` for OpenCode without model, `build_args()` for OpenCode with model, `version_args()` returns correct values for both variants, `install_hint()` returns non-empty string for both variants, `default()` is `Claude`, serde deserialization from TOML strings (`"claude"` → `Claude`, `"opencode"` → `OpenCode`), serde rejects invalid values (e.g., `"gpt"` → error). Test `build_args()` with a prompt containing newlines, quotes, and special characters to verify the args vector is correct.
- [x] Write tests for `AgentConfig` deserialization: full `[agent]` section parses, partial section (only `model`) defaults `cli` to Claude, missing `[agent]` section defaults to `{cli: Claude, model: None}`, invalid `cli` value produces parse error, `deny_unknown_fields` rejects typos (e.g., `cli_tool = "claude"`)
- [x] Write tests for normalization: empty string `""` → `None`, whitespace `"  "` → `None`, tab/newline → `None`, valid string preserved, `None` stays `None`. Include a test that loads config via `load_config_from(Some(path), root)` (the `load_config_at` path) with `model = "  "` and asserts normalization to `None` — ensures both config loading paths are covered.
- [x] Write test for load_config no-file defaults: call `load_config()` when no `phase-golem.toml` exists and verify `config.agent` defaults to `AgentConfig { cli: CliTool::Claude, model: None }`.
- [x] Write tests for validation (allowlist `^[a-zA-Z0-9._/-]+$`): model with internal hyphens accepted (e.g., `claude-opus-4`), model with dots accepted (e.g., `gpt-4.1`), model with slashes accepted (e.g., `openai/gpt-4o`), model starting with `-` rejected, model starting with `--` rejected, model with spaces rejected (e.g., `"opus 4"`), model with special chars rejected (e.g., `"model;rm"`), `None` model passes validation
- [x] Write tests for `PhaseConfig` backward compat: `destructive = true` parses as `is_destructive = true` via alias, `deny_unknown_fields` rejects unknown keys. Also verify that `CliTool` and `AgentConfig` are accessible as `phase_golem::config::CliTool` and `phase_golem::config::AgentConfig` from the test crate (confirms public export through the crate boundary).
- [x] Write automated init template round-trip test: extract or replicate the `handle_init` template TOML string, parse it via `toml::from_str::<PhaseGolemConfig>()`, and assert success. This catches any `deny_unknown_fields` violations from stale field names (e.g., leftover `destructive` instead of `is_destructive`). Also assert the parsed config contains `[agent]` defaults (`cli = Claude, model = None`).

**Verification:**

- [x] `cargo build` succeeds
- [x] `cargo test` passes (all existing tests + new config tests)
- [x] TOML without `[agent]` section still loads with defaults (`cli=Claude, model=None`)
- [x] TOML with `[agent]` section parses correctly
- [x] TOML with `destructive = false` in phase config still parses (alias works)
- [x] TOML with unknown key in `[agent]` section produces deserialization error
- [x] `handle_init` generated TOML parses successfully via `load_config` (automated round-trip test passes)
- [x] `handle_init` generated TOML uses `is_destructive` (not `destructive`) and includes `[agent]` section
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[HAMY-001][P1] Feature: Add CliTool enum, AgentConfig struct, config pipeline updates, and init template fixes`

**Notes:**

The `CliTool::build_args()` implementation for each variant:
- **Claude:** `["--dangerously-skip-permissions", "--model", "<model>", "-p", "<prompt>"]` (omit `--model` pair if model is `None`)
- **OpenCode:** `["run", "--model", "<model>", "--quiet", "<prompt>"]` (omit `--model` pair if model is `None`)

The `normalize_agent_config` helper is called from both `load_config` and `load_config_at` because both paths parse TOML independently. The `load_config_from` function dispatches to one of these two, so it is transitively covered — no changes needed there. The third path in `load_config` (missing file → defaults) does NOT need normalization because `PhaseGolemConfig::default()` already has `model: None`.

**Important ordering:** `normalize_agent_config` must be called BEFORE `validate()` in both loading paths. Normalization transforms `model = "  "` into `None`, which then passes validation. If validation runs first, the whitespace-only string would be rejected by the allowlist pattern. The correct pipeline order is: `toml::from_str` → `normalize_agent_config` → `populate_default_pipelines` → `validate()`.

Line numbers in task descriptions (e.g., "line 279", "line 248") are hints based on the current source and will shift as code is added. Use the textual anchors ("after `toml::from_str`", "before `populate_default_pipelines`") as the primary reference.

The `serde(alias = "destructive")` is a permanent backward-compatibility alias, not a temporary migration shim. It cannot be removed without a migration step for existing configs.

The `handle_init` template fix (`destructive` → `is_destructive`) and `[agent]` section are co-located in Phase 1 with the `deny_unknown_fields` change to maintain the SPEC invariant that each phase leaves the codebase functional. Without co-location, `phase-golem init` would generate unparseable configs between Phase 1 and when the template fix lands.

The OpenCode invocation pattern (`opencode run [--model provider/model] [--quiet] <prompt>`) is based on tech research findings documented in `HAMY-001_multi-cli-agent-support_TECH_RESEARCH.md`. During implementation, verify that `opencode --version` returns exit 0; if it does not, use an alternative for `version_args()` (e.g., `["--help"]` or simply check binary existence on PATH).

`deny_unknown_fields` on `AgentConfig` is a deliberate strictness choice for a new, small struct — it catches typos in `[agent]` config blocks. This is stricter than other config sections (which do not use `deny_unknown_fields`) and means future field additions require a new release. This trade-off is accepted; the benefit of typo detection outweighs the minor extensibility cost.

**Followups:**

- Pre-existing bug found and fixed: `handle_init` template pre_phases entry was missing `is_destructive` field, which would fail parsing with `deny_unknown_fields`. Added `is_destructive = false` to the pre_phases entry.
- Model validation splits error message into two cases (invalid characters vs. leading hyphen) for clearer user feedback. This deviates slightly from the single error message in the SPEC but improves UX.
- The SPEC's allowlist regex `^[a-zA-Z0-9._/-]+$` doesn't actually reject flag-like prefixes since `-` is in the character class. Added an explicit `starts_with('-')` check for defense-in-depth. This is stricter than the regex alone.

---

### Phase 2: Agent Runner, Integration & Post-Result Validation

> Parameterize `CliAgentRunner`, update call sites in `handle_run` and `handle_triage`, add post-result validation, update integration test

**Phase Status:** complete

**Complexity:** Medium

**Goal:** Convert `CliAgentRunner` from a unit struct to a parameterized struct carrying `(CliTool, Option<String>)`. Convert `verify_cli_available` to an instance method. Update `run_agent` to use `self.tool.build_args()`. Update all call sites in `handle_run` and `handle_triage` to construct the runner from config, reorder startup sequences so config loads before CLI verification. Add post-result `item_id`/`phase` validation as a pure helper function called from `execute_phase`. After this phase, running `phase-golem run` with `[agent] cli = "claude"` (or no `[agent]` section) works identically to current behavior, and `cli = "opencode"` attempts to invoke the `opencode` binary.

**Files:**

- `src/agent.rs` — modify — Parameterize `CliAgentRunner`, convert `verify_cli_available` to `&self`, update `run_agent` to use `self.tool.build_args()`, add `use crate::config::CliTool;`
- `src/executor.rs` — modify — Add `validate_result_identity()` pure helper function and call it from `execute_phase` after `run_workflows_sequentially` returns `Ok(phase_result)`
- `src/main.rs` — modify — Reorder `handle_run` startup (verify after config load), construct `CliAgentRunner::new(config.agent.cli.clone(), config.agent.model.clone())`, same for `handle_triage`
- `tests/agent_integration_test.rs` — modify — Update `CliAgentRunner` construction and `verify_cli_available` call

**Patterns:**

- Follow `handle_run` existing startup flow (lines 277-293) — reorder within the same structure, preserving lock acquisition and git checks
- Follow `handle_triage` existing startup flow (lines 684-703) — same reordering

**Tasks:**

- [x] Add `use crate::config::CliTool;` import to `agent.rs`
- [x] Change `CliAgentRunner` from `pub struct CliAgentRunner;` to `pub struct CliAgentRunner { pub tool: CliTool, pub model: Option<String> }`. Add `pub fn new(tool: CliTool, model: Option<String>) -> Self` constructor.
- [x] Convert `verify_cli_available()` from static method to instance method (`&self`). Use `self.tool.binary_name()` for the command, `self.tool.version_args()` for args, and `self.tool.display_name()` in error messages. Include `self.tool.install_hint()` in the error message when the CLI is not found (e.g., `"OpenCode CLI not found on PATH. Install: https://github.com/opencode-ai/opencode"`).
- [x] Update `run_agent()` implementation: replace `Command::new("claude")` with `Command::new(self.tool.binary_name())`, replace hardcoded `.args(["--dangerously-skip-permissions", "-p", prompt])` with `.args(self.tool.build_args(prompt, self.model.as_deref()))`
- [x] Update `handle_run()` in `main.rs`: Move CLI verification from before config load to after. New flow: (1) install signal handlers, (2) acquire lock, (3) check git preconditions, (4) load config, (5) construct `CliAgentRunner::new(config.agent.cli.clone(), config.agent.model.clone())`, (6) call `runner.verify_cli_available()`. Update runner wrapping: `Arc::new(runner)` (runner already constructed).
- [x] Update `handle_triage()` in `main.rs`: Same reorder as `handle_run`. New flow: (1) install signal handlers, (2) acquire lock, (3) check git preconditions, (4) load config, (5) construct `CliAgentRunner::new(config.agent.cli.clone(), config.agent.model.clone())`, (6) call `runner.verify_cli_available()`. Lock acquisition and git checks remain before config load, matching `handle_run`'s pattern. **Important:** Do NOT wrap the runner in `Arc` for `handle_triage` — it is used directly via `&self` reference in a sequential loop, unlike `handle_run` which wraps in `Arc::new()` for the scheduler.
- [x] Update `tests/agent_integration_test.rs`: Change `CliAgentRunner::verify_cli_available()` to `let runner = CliAgentRunner::new(CliTool::Claude, None); runner.verify_cli_available()`. Change `let runner = CliAgentRunner;` to `let runner = CliAgentRunner::new(CliTool::Claude, None);`. Add `use phase_golem::config::CliTool;` import.
- [x] Add a config-to-runner integration test: parse a TOML string with `[agent] cli = "opencode"` and `model = "gpt-4"`, construct `CliAgentRunner::new(config.agent.cli, config.agent.model)`, and assert `runner.tool == CliTool::OpenCode` and `runner.model == Some("gpt-4")`. This verifies the full TOML → `AgentConfig` → `CliAgentRunner` pipeline end-to-end.
- [x] Add `validate_result_identity()` pure helper function in `executor.rs`: `pub fn validate_result_identity(result: &PhaseResult, expected_item_id: &str, expected_phase: &str) -> Result<(), String>`. Returns `Ok(())` if `result.item_id == expected_item_id` and `result.phase == expected_phase`. Returns `Err(descriptive message)` on mismatch (e.g., `"Result mismatch: expected item_id=WRK-001, got=WRK-002"`). This applies to ALL `ResultCode` variants — even a `Failed` result should have correct identity metadata.
- [x] Call `validate_result_identity()` in `execute_phase()` after `run_workflows_sequentially` returns `Ok(phase_result)`, BEFORE the match on `phase_result.result`. On `Err`, immediately return `PhaseExecutionResult::Failed(err_msg)` — this is an immediate return from the function, NOT a loop iteration failure. It must be placed so that retry logic does NOT re-execute for identity mismatches (non-retryable corruption).
- [x] Write unit tests for `validate_result_identity()`: (a) matching `item_id` and `phase` returns `Ok`, (b) mismatched `item_id` returns `Err` with descriptive message, (c) mismatched `phase` returns `Err`, (d) both mismatched returns `Err`.

**Verification:**

- [x] `cargo build` succeeds
- [x] `cargo test` passes (all existing tests + updated integration test compiles)
- [x] `cargo test --test agent_integration_test --no-run` compiles the integration test without running it (it is `#[ignore]`)
- [x] Verify `spawn_triage` in `scheduler.rs` requires zero code changes — it receives `Arc<impl AgentRunner>` generically and should work with the parameterized `CliAgentRunner` without modification
- [x] Manual test: `phase-golem run` with no `[agent]` config — skipped (autonomous mode, no manual testing available; verified via code inspection that default config path constructs `CliAgentRunner::new(CliTool::Claude, None)` and `build_args` produces `["--dangerously-skip-permissions", "-p", prompt]`)
- [x] Manual test: `phase-golem run` with `[agent] cli = "opencode"` — skipped (autonomous mode; verified via code inspection and config-to-runner integration test that OpenCode config flows through correctly)
- [x] Manual test: `phase-golem triage` with `[agent] cli = "opencode"` — skipped (autonomous mode; verified via code inspection that `handle_triage` constructs runner from `config.agent` identically to `handle_run`)
- [x] `validate_result_identity` tests pass: matching identity returns `Ok`; mismatched `item_id` returns `Err`; mismatched `phase` returns `Err`
- [x] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[HAMY-001][P2] Feature: Parameterize CliAgentRunner with configurable CLI tool and post-result validation`

**Notes:**

The reordering in `handle_run` changes the user-facing failure order: if both the config file and CLI binary are missing, the user now sees a config error (or defaults load) before the CLI-not-found error, rather than CLI-not-found first. This is acceptable since config loading provides useful defaults.

The `"[pre] Verifying Claude CLI..."` log line (currently at `main.rs:284`) should be preserved and moved to immediately before the new `runner.verify_cli_available()` call during the reorder. Phase 3 will update its text to use the tool's display name; for now, keep it as-is.

In `handle_triage`, the current ordering is: (1) signal handlers, (2) verify CLI, (3) acquire lock, (4) git checks, (5) load config. The new ordering should be: (1) signal handlers, (2) acquire lock, (3) git checks, (4) load config, (5) construct runner, (6) verify CLI. Lock acquisition and git checks remain before config load, matching `handle_run`'s pattern.

**DESIGN doc contradiction:** The Design doc explicitly states post-result validation is "DEFERRED to a separate change", but this SPEC includes it in Phase 2. This is intentional — the PRD lists post-result validation as a Must Have, and the Design doc's deferral note is outdated. When implementing, follow this SPEC, not the Design doc's deferral note. Phase 3 includes a task to update the Design doc to reflect this decision.

**Followups:**

- The `"[pre] Verifying Claude CLI..."` log message in `handle_run` is still hardcoded. Phase 3 will update it to use `config.agent.cli.display_name()`.
- Code review suggested considering `CliAgentRunner::new(agent_config)` instead of threading individual fields — already captured in Followups Summary (Low priority).
- Code review suggested explicit error types for non-retryable validation failures (e.g., `ValidationError::IdentityMismatch`) — deferred as low-priority; the current `Result<(), String>` is adequate for the single validation check.

---

### Phase 3: Observability

> Add startup logging, per-phase logging, update log message references, update PRD/Design docs

**Phase Status:** not_started

**Complexity:** Low

**Goal:** Add observability so users can see which CLI tool and model are being used. Replace hardcoded "Claude" references in log messages with the configured tool's display name. Add per-phase execution logging. Update the PRD and Design doc to reflect resolved open questions and decisions.

**Files:**

- `src/main.rs` — modify — Add startup logging in `handle_run`/`handle_triage`, update log messages at CLI verification steps
- `src/executor.rs` — modify — Add per-phase log line with CLI tool and model (reads from `config.agent` which is already passed to `execute_phase`)
- `changes/HAMY-001_multi-cli-agent-support/HAMY-001_multi-cli-agent-support_PRD.md` — modify — Mark resolved open questions
- `changes/HAMY-001_multi-cli-agent-support/HAMY-001_multi-cli-agent-support_DESIGN.md` — modify — Update post-result validation deferral note to reflect inclusion in Phase 2

**Tasks:**

- [ ] Add startup logging in `handle_run()` after runner construction and verification: log the resolved agent config. Format: `log_info!("[config] Agent: {} (model: {})", tool_display_name, model_or_default)` where `model_or_default` is `config.agent.model.as_deref().unwrap_or("default")`. Example output: `[config] Agent: Claude CLI (model: sonnet)` or `[config] Agent: Claude CLI (model: default)`. If `cli = OpenCode`, also log: `[config] Note: OpenCode CLI support is experimental.`
- [ ] Add resolved binary path logging in `handle_run()` after runner verification: use `which::which(config.agent.cli.binary_name())` (if the `which` crate is available) or `std::process::Command::new("which").arg(binary_name).output()` to resolve and log the full path. Format: `log_info!("[config] Binary: {}", resolved_path)`. If resolution fails, log a warning but do not error (the verify step already confirmed the binary works). Check if the `which` crate is already a dependency before using it; if not, use the shell `which` command via `Command`.
- [ ] Add same startup logging in `handle_triage()` after runner construction.
- [ ] Update the verification log message in `handle_run()`: change `"[pre] Verifying Claude CLI..."` to use `config.agent.cli.display_name()` (e.g., `"[pre] Verifying Claude CLI..."` or `"[pre] Verifying OpenCode CLI..."`). Note: this line must reference the config, so it appears after config load.
- [ ] Update the verification log message in `handle_triage()`: same change as `handle_run()` — use `config.agent.cli.display_name()` instead of hardcoded "Claude CLI".
- [ ] Add per-phase log line in `executor.rs:execute_phase()`: before the retry loop (before line 321), add `log_info!("[{}][{}] Using {} (model: {})", item.id, phase_config.name.to_uppercase(), config.agent.cli.display_name(), config.agent.model.as_deref().unwrap_or("default"))`. This uses the existing `config: &PhaseGolemConfig` parameter — no signature change needed.
- [ ] Update the PRD's open questions: mark the OpenCode invocation pattern question as resolved with: "Resolved: `opencode run [--model provider/model] [--quiet] <prompt>` per tech research." Mark the model selection question as resolved with: "Resolved: OpenCode uses `--model provider/model` format; Claude uses `--model name` format. Both passed via `build_args()`."
- [ ] Update the Design doc's post-result validation note (which says "deferred to a separate change") to reflect that it was un-deferred and included in Phase 2 of this SPEC.

**Verification:**

- [ ] `cargo build` succeeds
- [ ] `cargo test` passes (no regressions)
- [ ] Manual test: `phase-golem run` logs agent config at startup (e.g., `[config] Agent: Claude CLI (model: default)`)
- [ ] Manual test: `phase-golem run` logs the resolved binary path at startup (e.g., `[config] Binary: /usr/local/bin/claude`)
- [ ] Manual test: Each phase execution logs the CLI tool and model
- [ ] Verify PRD open questions are updated with resolution notes
- [ ] Verify Design doc post-result validation deferral note is updated
- [ ] Code review passes (`/code-review` → fix issues → repeat until pass)

**Commit:** `[HAMY-001][P3] Feature: Add agent observability and update docs`

**Notes:**

The per-phase logging in `executor.rs` requires no signature changes because `execute_phase` already receives `config: &PhaseGolemConfig`, which now contains `config.agent.cli` and `config.agent.model` from Phase 1.

**Followups:**

---

## Final Verification

- [ ] All phases complete
- [ ] All PRD success criteria met:
  - [ ] User can configure CLI tool in `phase-golem.toml` via `[agent]` section
  - [ ] `AgentRunner` trait signature unchanged; `CliAgentRunner` parameterized; `MockAgentRunner` unaffected
  - [ ] Correct command/arguments for configured CLI tool (unit tested per tool)
  - [ ] Default behavior remains `claude` when no CLI configured (both absent section and omitted field)
  - [ ] `verify_cli_available()` accepts configured CLI tool, returns error with tool name
  - [ ] All entry points updated: `handle_run`, `handle_triage` (construct from config)
  - [ ] CLI tool field deserializes as enum via serde (invalid values = deser error)
  - [ ] Effective agent config logged at startup
  - [ ] Existing tests pass
  - [ ] `handle_init` includes `[agent]` section with defaults
  - [ ] Log messages use configured tool's display name
  - [ ] Per-phase execution logs include CLI tool and model
- [ ] Tests pass
- [ ] No regressions introduced
- [ ] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| Phase 1 | complete | pending | Config foundation, init template fixes, 66 config tests pass |
| Phase 2 | complete | pending | Parameterized CliAgentRunner, post-result validation, 581 tests pass |

## Followups Summary

### Critical

### High

(none — post-result validation moved into Phase 2)

### Medium

- [ ] OpenCode smoke test — Before graduating OpenCode from experimental: verify (1) `opencode run` produces valid PhaseResult JSON, (2) `--quiet` doesn't suppress file output, (3) `--model` works or fails gracefully.
- [ ] Consider extracting shared post-parse pipeline (`normalize` → `populate_default_pipelines` → `validate`) into a single helper called by both `load_config` and `load_config_at` to eliminate duplication.
- [ ] Add config validation warning when `model` is set for a CLI tool that doesn't support `--model` (PRD constraint). Currently academic since both Claude and OpenCode support `--model`, but should be implemented when a tool without model support is added.

### Low

- [ ] Add Codex CLI as a third `CliTool` variant — tech research confirmed it's equally mature with `codex exec` for non-interactive use. Low marginal cost (one enum variant + one `build_args` match arm).
- [ ] ACP (Agent Client Protocol) integration as a future `AcpAgentRunner` — provides universal protocol support for 10+ tools but has a fundamental stdin/stdout conflict with current architecture that needs separate design work.
- [ ] Add `--prompt-file` support for large prompts that might approach argv limits (~2MB on Linux).
- [ ] Consider changing `CliAgentRunner::new(tool, model)` to `CliAgentRunner::new(agent_config)` — the current struct duplicates `AgentConfig` fields. If `AgentConfig` grows (e.g., for per-phase overrides in HAMY-001b), every new field must be threaded through separately.

## Design Details

### Key Types

```rust
// In src/config.rs

#[derive(Default, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CliTool {
    #[default]
    Claude,
    OpenCode,
}

impl CliTool {
    pub fn binary_name(&self) -> &str { /* "claude" | "opencode" */ }
    pub fn display_name(&self) -> &str { /* "Claude CLI" | "OpenCode CLI" */ }
    pub fn build_args(&self, prompt: &str, model: Option<&str>) -> Vec<String> { /* full args */ }
    pub fn version_args(&self) -> Vec<&str> { /* ["--version"] */ }
    pub fn install_hint(&self) -> &str { /* install URL/command per variant */ }
}

#[derive(Default, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    pub cli: CliTool,
    pub model: Option<String>,
}

// In src/agent.rs

pub struct CliAgentRunner {
    pub tool: CliTool,
    pub model: Option<String>,
}
```

### Architecture Details

See `HAMY-001_multi-cli-agent-support_DESIGN.md` for the full system design, data flow, key flows, and resolved questions.

### Design Rationale

See the Design doc's Technical Decisions section for rationale on: method-based dispatch, trait unchanged, instance method verification, field-level serde defaults, model normalization/validation separation, post-result validation deferral, and deny_unknown_fields decisions.

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
