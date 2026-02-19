# Change: Multi-CLI Agent Support for Phase-Golem

**Status:** Proposed
**Created:** 2026-02-15
**Updated:** 2026-02-18
**Author:** sirhamy

## Problem Statement

Phase-golem currently hardcodes `claude` as the only AI CLI tool for executing pipeline phases. The `CliAgentRunner` constructs `Command::new("claude")` with Claude-specific arguments (`--dangerously-skip-permissions -p`), making it impossible to use alternative AI CLI tools like OpenCode (an open-source AI coding CLI) or Codex CLI (OpenAI's AI coding CLI).

Different CLI tools offer different strengths — cost, speed, model access, and language/framework specializations — and phase-golem should allow users to choose which tool runs their pipeline phases. The growing availability of alternative AI coding CLIs makes this a practical need now.

## User Stories / Personas

- **Phase-Golem User** — Wants to configure which AI CLI tool phase-golem uses for agent execution, so they can switch between tools (claude code, opencode) based on preference, cost, or capability without modifying source code.

## Desired Outcome

When phase-golem runs pipeline phases, it should use a configurable AI CLI tool instead of always using `claude`. The user specifies a global default CLI tool and model in `phase-golem.toml` under `[agent]`. Phase-golem constructs the appropriate command with the correct arguments for the configured tool. If no CLI is specified, phase-golem defaults to `claude` (current behavior) for backwards compatibility.

Example config shape:
```toml
[agent]
cli = "claude"          # global default CLI (default: "claude")
model = "sonnet"        # global default model (omit to use CLI's own default)
```

## Success Criteria

### Must Have

- [ ] `PhaseGolemConfig` gains an `agent: AgentConfig` field with `#[serde(default)]`. `AgentConfig` is a new struct with `cli: CliTool` (default: `Claude`) and `model: Option<String>` (default: `None`), both implementing `Default`. The `CliTool` enum deserializes directly via serde; invalid values produce a deserialization error listing valid options.
- [ ] User can configure which AI CLI tool to use in `phase-golem.toml` via a global `[agent]` section
- [ ] The `AgentRunner` trait signature remains unchanged; `CliAgentRunner` becomes a parameterized struct (constructed with `AgentConfig`) instead of a unit struct. `MockAgentRunner` is unaffected.
- [ ] The orchestrator constructs the correct command and arguments for the configured CLI tool, verified by unit tests per tool type
- [ ] Default behavior remains `claude` when no CLI is explicitly configured — both when the `[agent]` section is entirely absent and when the `cli` field is omitted (backwards compatibility)
- [ ] `verify_cli_available()` accepts the configured CLI tool and checks it is present on PATH. Remains a synchronous (blocking) function, called during startup pre-checks before async task spawning. Returns an error including the tool name and an install suggestion if not found. For CLI tools not yet validated (e.g., `opencode`), returns an error at this stage: "OpenCode CLI support is not yet validated. See HAMY-001 open questions."
- [ ] All entry points that create `CliAgentRunner` are updated to construct it with the resolved global agent config: `handle_run` in `main.rs` (creates `Arc<CliAgentRunner>`) and `handle_triage` in `main.rs` (creates `CliAgentRunner`). Note: `spawn_triage` in `scheduler.rs` receives the runner as a parameter from `handle_run` and does not construct one.
- [ ] The `cli` field in TOML must be lowercase (e.g., `claude`, `opencode`); other casing produces a deserialization error. This is enforced via `#[serde(rename_all = "lowercase")]` or equivalent on the `CliTool` enum.
- [ ] Config validation rejects model strings that do not match the allowlist pattern `^[a-zA-Z0-9._/-]+$`. This rejects whitespace, control characters, and flag-like prefixes (`--`, `-`) as defense-in-depth. Invalid values produce a clear error at config load time identifying the specific validation failure.
- [ ] Configuring `model = ""` or `model = "  "` (empty or whitespace-only) is equivalent to omitting the model field entirely — normalized to `None` during post-deserialization in `load_config`.
- [ ] When a `model` is configured for a CLI tool whose enum variant has no model flag support, a `warn`-level log is emitted at config load time (e.g., "Model 'opus' configured but OpenCode CLI does not support model selection; model will be ignored"). Execution proceeds with the model value ignored during command construction.
- [ ] When running with no `[agent]` config section, log the effective agent configuration using the format: `"Agent: {display_name} (model: {model})"` when model is set, or `"Agent: {display_name} (default model)"` when model is `None`.
- [ ] After reading a result file in `execute_phase` (in `executor.rs`), assert that `result.item_id == expected_item_id` and `result.phase == expected_phase`, rejecting results with mismatches. A mismatch is treated as a non-retryable error (bypasses retry loop, immediately returns an error) since retrying with the same prompt will produce the same mismatch. The error message includes expected vs. actual values. This validation guards against result file mix-ups that become more likely when multiple CLI tools are in use.
- [ ] Fix pre-existing `destructive`/`is_destructive` field name mismatch: the `handle_init` template generates `destructive = false` but the `PhaseConfig` struct field is `is_destructive`. Fix by adding `#[serde(alias = "destructive")]` to the `is_destructive` field in `PhaseConfig`, and update the `handle_init` template to use the canonical field name `is_destructive`.
- [ ] Existing tests continue to pass (MockAgentRunner is unaffected)

### Should Have

- [ ] `handle_init` includes the `[agent]` section with defaults in generated `phase-golem.toml`. The generated TOML should show the default explicitly and comment out the optional model field for discoverability:
  ```toml
  [agent]
  cli = "claude"
  # model = "sonnet"  # Optional: omit to use CLI's own default
  ```
- [ ] Log messages reference the configured CLI tool's display name instead of hardcoded "Claude" (e.g., "Verifying OpenCode CLI..." instead of "Verifying Claude CLI..."). Display names are defined on the enum (e.g., `Claude` → "Claude CLI", `OpenCode` → "OpenCode CLI"). Note: `handle_init` should NOT list `opencode` as a suggested option until its invocation pattern is validated.
- [ ] Each phase execution log line includes the resolved CLI tool and model (e.g., `[WRK-001][BUILD] Using Claude CLI (model: opus)`)

### Nice to Have

_(None for this change — additional features are deferred to follow-up changes.)_

## Scope

### In Scope

- Adding `AgentConfig` struct (with `cli: CliTool` and `model: Option<String>`) and `CliTool` enum to the config module
- Adding an `agent: AgentConfig` field (with `#[serde(default)]`) to `PhaseGolemConfig`
- Adding an `[agent]` config section to `phase-golem.toml` with a global default CLI tool (enum: `claude`, `opencode`) and optional model
- Defining supported CLI tools and their invocation patterns as enum-associated data: command name, prompt-passing flags, model flag format (if supported), permission-skipping flag, display name
- Updating `CliAgentRunner` from a unit struct to a parameterized struct carrying `AgentConfig` — the `AgentRunner` trait signature remains unchanged
- Updating `verify_cli_available()` to accept the configured CLI tool. It becomes a method on the parameterized `CliAgentRunner` (using `self.config.cli`) or an associated function taking `&CliTool`. It remains a synchronous (blocking) function.
- Updating all call sites that instantiate `CliAgentRunner` (`handle_run` in `main.rs`, `handle_triage` in `main.rs`)
- Updating log messages that reference "Claude" to use the configured tool's display name
- Config validation: serde enum deserialization for `cli` (lowercase required); allowlist regex `^[a-zA-Z0-9._/-]+$` for model strings; empty/whitespace model normalization to `None` in `load_config`
- CLI tool selection constrained to a predefined enum/allowlist (no arbitrary command paths)
- Model is an optional string passed through to the CLI tool's model flag as-is (the CLI tool validates model names). Empty string or whitespace-only values are normalized to `None` during post-deserialization normalization in `load_config`, so the config struct never contains a semantically-empty model value. Note: model string format is tool-specific (e.g., claude uses bare names like `opus`, opencode may require provider-prefixed strings like `openai/gpt-4o`)
- Safe prompt passing via Rust's `Command::args()` API (no shell interpolation). Each CLI tool's prompt delivery mechanism is defined as part of its enum variant data (e.g., claude uses `-p <prompt>` as a single argument).
- Post-result validation in `execute_phase`: assert `result.item_id` and `result.phase` match expected values; non-retryable error on mismatch
- Fixing the pre-existing `destructive`/`is_destructive` field name mismatch in `PhaseConfig` and `handle_init` template

### Out of Scope

- **Per-phase CLI tool and model overrides** — deferred to a follow-up change (HAMY-001b: per-phase agent config overrides, allowing different CLI tools/models for different pipeline phases). The global agent config provides immediate value; per-phase overrides add significant complexity (config resolution rules, runner construction changes, complete-override vs. merge semantics) and serve a narrower use case.
- **Environment variable or CLI flag overrides** for agent config (e.g., `PHASE_GOLEM_CLI=opencode phase-golem run`) — future enhancement. The design should not preclude adding it later (layering: CLI flag > env var > config > default).
- Effort level or other CLI-specific parameter tuning beyond model selection (future work)
- Adding new CLI tools beyond claude and opencode (can be added incrementally via enum extension)
- Changes to prompt format or result file handling (prompt format and result file handling are CLI-agnostic)
- CLI version detection and compatibility checking (assume users have versions that support the invocation flags described in Constraints)

## Non-Functional Requirements

- **Performance:** Negligible impact — only command construction changes; no additional I/O beyond the existing `verify_cli_available` check
- **Backwards Compatibility:** Existing `phase-golem.toml` files without an `[agent]` section must continue working unchanged
- **Security:** CLI tool binary is selected from an enum allowlist; users cannot specify arbitrary command paths. The enum is the deserialization target (enforced by `serde`), not a string validated after parsing. See Risks section for trust model documentation.
- **Observability:** Log which CLI tool and model are being used at startup and during agent execution. Log the resolved binary path (e.g., via `which`) at startup for debugging.
- **Trust Model:** Phase-golem trusts all configured CLI tools to run with full permissions. The `is_destructive` flag on `PhaseConfig` controls only git commit scheduling behavior, NOT agent sandboxing. All phases — including read-only phases like `research` and `review` — run with the CLI tool's permission-skipping flag (e.g., `--dangerously-skip-permissions` for claude). This is the existing security model; this change does not alter it.

## Constraints

- **CLI tool definition structure:** Each enum variant defines:
  - Command name (binary on PATH, e.g., `"claude"`, `"opencode"`)
  - Display name for logs/errors (e.g., `"Claude CLI"`, `"OpenCode CLI"`)
  - Prompt-passing flags (e.g., `["-p"]` for claude)
  - Model flag format (e.g., `["--model"]` for claude, passed as two separate args `["--model", model_value]`; `None` if tool does not support model selection)
  - Permission-skipping flag (e.g., `["--dangerously-skip-permissions"]` for claude)
  - Whether the variant is validated/supported (see below)
- **Variant validation status:** The `opencode` variant is _defined_ in the enum but _unsupported_ at runtime. Invoking it produces an error at `verify_cli_available()` time (before any pipeline execution): "OpenCode CLI support is not yet validated. See HAMY-001 open questions." A variant becomes _supported_ when its invocation pattern is confirmed and its command-construction logic is implemented.
- Invocation patterns:
  - `claude`: `claude --dangerously-skip-permissions -p <prompt>` (model via `--model <model>` as two separate args)
  - `opencode`: TBD — must be resolved during tech research before opencode can be marked as supported.
- CLI tools MUST accept prompts via command-line arguments (NOT stdin). The orchestrator sets `stdin(Stdio::null())` because `setpgid` (set process group ID) places child processes in a background process group, and any stdin read would cause SIGTTIN (a Unix signal that suspends a background process attempting to read from the terminal). Each CLI tool MUST be verified to accept prompts via command-line arguments before it is marked as supported.
- **Prompt size:** Prompts are passed as command-line arguments. On Linux, the argv limit is typically ~2MB (`MAX_ARG_STRLEN`), which should be sufficient for most prompts. If a CLI tool has stricter limits or if prompts grow large (e.g., triage with many backlog items), an alternative mechanism (e.g., `--prompt-file`) should be considered as a future enhancement.
- All CLI tools must be spawnable as isolated process groups (using `setpgid`) for proper signal handling and timeout enforcement
- The subprocess execution infrastructure (`run_subprocess_agent`) remains shared across all CLI tools — only command construction changes
- **PhaseResult contract:** Each supported CLI tool must produce a JSON file at the result path matching the `PhaseResult` schema (deserialized via `serde_json::from_str::<PhaseResult>()`). The prompt instructs the agent to write this file. For each non-claude CLI tool, a manual or integration smoke test must verify the tool reliably produces valid `PhaseResult` JSON when given the standard prompt format. This verification is a prerequisite for marking that tool as supported.
- **Model passthrough:** Model strings are passed verbatim via the CLI tool's model flag (e.g., `--model sonnet` for claude, passed as two separate args). If a CLI tool does not support a model flag, configuring a model for that tool produces a `warn`-level log at config load time; execution proceeds with the model value ignored during command construction. Model strings must match the allowlist pattern `^[a-zA-Z0-9._/-]+$` — this rejects whitespace, control characters, and flag-like prefixes (`--`, `-`) as defense-in-depth. Although `Command::args()` prevents shell injection, the allowlist guards against argument-splitting bugs in CLI tools and prevents accidental flag injection (e.g., a model string starting with `--` being interpreted as a flag by the target CLI).
- **Global `[agent]` field defaults:** Each field defaults independently via `#[serde(default)]`. `cli` defaults to `claude`. `model` defaults to `None` (use CLI's own default). A partial section like `[agent]\nmodel = "opus"` (no cli) is valid and means cli=claude, model=opus.

## Design Decisions

- **`AgentRunner` trait unchanged:** The `AgentRunner` trait signature (`fn run_agent(&self, prompt, result_path, timeout)`) remains unchanged. `CliAgentRunner` becomes a parameterized struct carrying an `AgentConfig` struct (rather than raw `(CliTool, Option<String>)` tuple) for forward extensibility — future fields like effort level can be added to `AgentConfig` without changing the `CliAgentRunner` constructor signature. `MockAgentRunner` remains unaffected by this change. This avoids changing the trait's generic usage across `executor.rs`, `scheduler.rs`, and all test code. Note: the `AgentRunner` trait uses `impl Future` return (not object-safe), which means per-phase CLI overrides (HAMY-001b) would need to evaluate whether the monomorphic `impl AgentRunner` pattern is sufficient or whether the trait needs to become object-safe.
- **Global-only scope for v1:** Only global agent config is supported in this change. Per-phase overrides are deferred to HAMY-001b to reduce the scope of code changes and potential regressions. The config schema is designed so per-phase can be added later without breaking changes.
- **Enum-based allowlist:** CLI tools are constrained to a Rust enum (`CliTool`) deserialized directly by serde (lowercase values required). Invalid values produce a deserialization error listing valid options. This is type-safe and prevents arbitrary command execution. New tools are added by extending the enum.
- **opencode deferred validation:** The `opencode` enum variant is defined in the enum for forward compatibility but is unsupported at runtime until its invocation pattern is validated during tech research. Including the variant now avoids blocking the implementation, while the runtime error at `verify_cli_available()` acknowledges the unknown invocation pattern. The `handle_init` template should not list `opencode` as a suggested option until validated.

## Dependencies

- **Depends On:** Knowledge of each CLI tool's invocation pattern (how they accept prompts, run non-interactively, select models). Claude's pattern is known. OpenCode's pattern will be resolved during tech research — this is a prerequisite for enabling the `opencode` enum variant, not for shipping the change.
- **Blocks:** Future work on per-phase overrides (HAMY-001b: per-phase agent config overrides), effort levels, additional CLI tools, and other per-phase tuning parameters. Note: HAMY-001b will need to evaluate whether the monomorphic `impl AgentRunner` pattern is sufficient for per-phase CLI selection or whether the trait needs to become object-safe.

## Risks

- [ ] CLI invocation patterns may change across versions — mitigate by keeping CLI-specific logic isolated in enum-associated data, easy to update
- [ ] Some CLI tools may not support all architectural requirements (non-interactive mode, prompt via args, process group isolation) — mitigate by verifying during tech research phase; tools that don't meet constraints produce a runtime error until resolved
- [ ] **Trust model / permissions:** All phases run with full unrestricted permissions via `--dangerously-skip-permissions` (or equivalent). A non-destructive phase that should only produce a markdown document has the same power to delete files as the `build` phase. Different CLI tools may have different permission models. Mitigate by documenting the trust model and keeping the permission-skip flag per-enum-variant so future CLI tools can specify their own approach.
- [ ] **PATH-based binary resolution:** CLI tools are resolved via `Command::new("tool_name")` which searches `$PATH`. The enum allowlist prevents arbitrary commands in config but does not prevent a compromised binary on the user's PATH. Mitigate by logging the resolved binary path at startup (observability).
- [ ] **PhaseResult contract for non-Claude CLIs:** The orchestrator's state machine depends on agents writing specific JSON to a specific file path, enforced only via prompt instructions. Non-Claude CLIs may not reliably follow these instructions. Mitigate by requiring integration smoke tests before marking any CLI tool as supported.
- [ ] **Concurrent agents in shared git working tree:** Multiple agents running simultaneously (when `max_concurrent > 1` for non-destructive phases) share the same working directory. Different CLI tools may have different file modification patterns, amplifying concurrent-write risks. Concurrent-write risk in a shared working tree is a pre-existing concern, not introduced by this change, but multi-CLI support may amplify it due to differing file modification patterns across tools.
- [ ] **Invalid model retries:** If the CLI tool rejects the configured model string, the phase will fail and be retried per normal retry logic. Since the model string is a config-level setting, all retries will fail identically. This is acceptable for v1; future work could detect this class of error and skip retries.

## Open Questions

- [x] ~~Should the config field be a simple enum of known tools, or a more flexible structure?~~ **Decision: Enum** — type-safe, aligns with allowlist security model
- [x] ~~Where in `phase-golem.toml` should this live?~~ **Decision: `[agent]` section** — semantically distinct from execution params, room for future config
- [x] ~~Which CLI tools in v1?~~ **Decision: claude + opencode (enum variants)** — claude fully supported, opencode deferred until invocation pattern validated. Codex CLI (OpenAI) can be added later via enum extension.
- [x] ~~Per-phase CLI selection priority?~~ **Decision: Deferred to follow-up change (HAMY-001b)** — global agent config provides immediate value; per-phase overrides add significant complexity and serve a narrower use case
- [x] What is the exact invocation pattern for opencode? (command name, args for prompt, non-interactive flags, model selection) — **Resolved:** `opencode run [--model provider/model] [--quiet] <prompt>` per tech research.
- [x] How does model selection work for each CLI tool? (claude has `--model`, opencode TBD) — **Resolved:** OpenCode uses `--model provider/model` format; Claude uses `--model name` format. Both passed via `build_args()`.
- [x] ~~Should `handle_init` include the new CLI config field in generated `phase-golem.toml`?~~ **Decision: Yes, include with defaults** — makes it explicit which agent CLI and model are being used
- [x] ~~What should the config ergonomics look like for per-phase overrides?~~ **Decision: Deferred to HAMY-001b** — will be addressed when per-phase overrides are in scope
- [x] ~~Should the `AgentRunner` trait signature change?~~ **Decision: No** — `CliAgentRunner` becomes a parameterized struct; trait signature unchanged; `MockAgentRunner` unaffected

## Pre-existing Issues (noted during critique, addressed in this change)

- `handle_init` template uses `destructive = false` in generated TOML (`main.rs:201`) but the `PhaseConfig` struct field is `is_destructive` (`config.rs:57`) with no `#[serde(rename)]` or `#[serde(alias)]`. This means generated config files may not deserialize correctly for this field. **Resolution: included in Must Have scope** — fix via `#[serde(alias = "destructive")]` on the struct field and updating the template to use the canonical field name `is_destructive`.

## References

- Current hardcoded CLI invocation: `src/agent.rs:150` (`CliAgentRunner::run_agent`, `Command::new("claude")`)
- `AgentRunner` trait: `src/agent.rs:115`
- `CliAgentRunner` unit struct: `src/agent.rs:127` (`pub struct CliAgentRunner;`)
- `verify_cli_available`: `src/agent.rs:129`
- Subprocess execution: `src/agent.rs:163` (`run_subprocess_agent`)
- stdin constraint (`Stdio::null()`): `src/agent.rs:187`
- Config structure (`PhaseConfig`): `src/config.rs:52`
- Config file path: `src/config.rs:223` (`phase-golem.toml`)
- `handle_run` (creates `Arc<CliAgentRunner>`): `src/main.rs:249` (runner at line 520)
- `handle_triage` (creates `CliAgentRunner`): `src/main.rs:661` (runner at line 680)
- `handle_init` template: `src/main.rs:179`
- `spawn_triage` (receives runner): `src/scheduler.rs:1526`
- `execute_phase` (calls `runner.run_agent()`): `src/executor.rs:270`
