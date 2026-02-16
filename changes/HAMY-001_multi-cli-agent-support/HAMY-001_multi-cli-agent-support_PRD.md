# Change: Multi-CLI Agent Support for Orchestrator

**Status:** Draft
**Created:** 2026-02-15
**Author:** sirhamy

## Problem Statement

The orchestrator currently hardcodes `claude` as the only AI CLI tool for executing pipeline phases. The `CliAgentRunner` constructs `Command::new("claude")` with Claude-specific arguments (`--dangerously-skip-permissions -p`), making it impossible to use alternative AI CLI tools like OpenCode or Codex CLI.

Different CLI tools offer different strengths — cost, speed, model access, specialized capabilities — and the orchestrator should allow users to choose which tool runs their pipeline phases. The presence of `opencode.json` in the repo and the growing availability of alternative AI coding CLIs makes this a practical need now.

## User Stories / Personas

- **Orchestrator User** — Wants to configure which AI CLI tool the orchestrator uses for agent execution, so they can switch between tools (claude code, opencode) based on preference, cost, or capability without modifying orchestrator source code.

- **Multi-Tool User** — Wants to use different CLI tools and models for different pipeline phases (e.g., a cheaper model for research, a more capable one for build), enabling cost management and model tiering across the pipeline.

## Desired Outcome

When the orchestrator runs pipeline phases, it should use a configurable AI CLI tool instead of always using `claude`. The user specifies a global default CLI tool in `orchestrate.toml` under `[agent]`, and can optionally override the CLI tool and model per-phase in the pipeline config. The orchestrator constructs the appropriate command with the correct arguments for the resolved tool. If no CLI is specified anywhere, it defaults to `claude` (current behavior) for backwards compatibility.

Example config shape:
```toml
[agent]
cli = "claude"          # global default CLI
model = "sonnet"        # global default model (omit to use CLI's own default)

[[pipelines.feature.phases]]
name = "research"
agent = { cli = "opencode", model = "gpt-4o" }  # complete override, cli required

[[pipelines.feature.phases]]
name = "build"
agent = { cli = "claude", model = "opus" }       # complete override

[[pipelines.feature.phases]]
name = "review"
# no agent block — inherits everything from global [agent]
```

## Success Criteria

### Must Have

- [ ] User can configure which AI CLI tool to use in `orchestrate.toml`
- [ ] The orchestrator constructs the correct command and arguments for the configured CLI tool (each CLI has different invocation patterns), verified by unit tests per tool type
- [ ] Default behavior remains `claude` when no CLI is explicitly configured — both when the config section is entirely absent and when the field is omitted (backwards compatibility)
- [ ] `verify_cli_available()` checks all unique CLI tools referenced across global config and per-phase overrides, collecting ALL failures and reporting them in a single error message (not fail-on-first) so users can fix all missing tools at once
- [ ] All entry points that use `CliAgentRunner` are updated: `handle_run` (per-phase resolution), `handle_triage` (global default only — triage is not a pipeline phase and never uses per-phase overrides), and `spawn_triage` in `scheduler.rs` (`main.rs`, `scheduler.rs`)
- [ ] Validation at config load time that the configured CLI tool is a known/supported value — the CLI tool field MUST deserialize directly into a Rust enum via `serde` (not a string validated later), so invalid values produce a deserialization error
- [ ] Existing tests continue to pass (MockAgentRunner is unaffected)

### Should Have

- [ ] `handle_init` includes the `[agent]` section with defaults in generated `orchestrate.toml` (makes the CLI tool and model explicit in new projects)
- [ ] Error message when the configured CLI tool is not found on PATH includes the tool name and an install suggestion
- [ ] Log messages reference the configured CLI tool name instead of hardcoded "Claude" (e.g., "Verifying OpenCode CLI..." instead of "Verifying Claude CLI...")
- [ ] Per-phase CLI tool override: each phase in the pipeline can specify which CLI tool to use, falling back to the global default
- [ ] Per-phase model override: each phase can specify which model to use (if the CLI tool supports model selection)

### Nice to Have

- [ ] Ability to pass additional CLI-specific arguments beyond model (e.g., effort level, temperature)

## Scope

### In Scope

- Adding an `[agent]` config section to `orchestrate.toml` with a global default CLI tool (enum: `claude`, `opencode`)
- Per-phase CLI tool and model overrides in `PhaseConfig` (phases inherit the global default but can specify their own)
- Defining supported CLI tools and their invocation patterns (command name, argument format for passing prompts, model flag if supported)
- Updating `CliAgentRunner` to accept CLI tool + model config instead of hardcoded `claude`
- Updating `verify_cli_available()` to check for the configured tool(s)
- Updating all call sites that instantiate `CliAgentRunner` (`handle_run`, `handle_triage`)
- Updating log messages that reference "Claude" to use the configured tool name
- Config validation for the new fields
- CLI tool selection constrained to a predefined enum/allowlist (no arbitrary command paths)
- Model is a free-form optional string passed through to the CLI tool as-is (no orchestrator-side validation — the CLI tool validates it). An empty string or whitespace-only value is treated as `None` (model flag omitted). Note: model string format is tool-specific (e.g., claude uses bare names like `opus`, opencode may require provider-prefixed strings like `openai/gpt-4o`)
- Safe prompt passing via command-line arguments (no shell interpolation)
- Updating the `AgentRunner` trait interface if needed for per-phase config (trait must remain mockable for tests; all implementors including `MockAgentRunner` must be updated)

### Out of Scope

- Effort level or other CLI-specific parameter tuning beyond model selection (future work)
- Adding new CLI tools beyond claude and opencode (can be added incrementally via enum extension)
- Changes to prompt format or result file handling (prompt format and result file handling are CLI-agnostic)
- CLI version detection and compatibility checking (assume users have versions that support the invocation flags described in Constraints)

## Non-Functional Requirements

- **Performance:** Negligible impact — only command construction changes; no additional I/O beyond the existing `verify_cli_available` check (which may now check multiple tools in parallel)
- **Backwards Compatibility:** Existing `orchestrate.toml` files without CLI config must continue working unchanged
- **Security:** CLI tool binary is selected from an enum allowlist; users cannot specify arbitrary command paths. The enum is the deserialization target (enforced by `serde`), not a string validated after parsing.
- **Observability:** Log which CLI tool is being used at startup and during agent execution

## Constraints

- Each AI CLI tool has a different invocation pattern:
  - `claude`: `claude --dangerously-skip-permissions -p <prompt>` (model via `--model <model>`)
  - `opencode`: TBD — need to verify exact invocation pattern and model flag
- CLI tools MUST accept prompts via command-line arguments (NOT stdin). The orchestrator sets `stdin(Stdio::null())` because `setpgid` (set process group ID) places child processes in a background process group, and any stdin read would cause SIGTTIN (a Unix signal that silently stops a background process attempting to read from the terminal).
- All CLI tools must be spawnable as isolated process groups (using `setpgid`) for proper signal handling and timeout enforcement
- The subprocess execution infrastructure (`run_subprocess_agent`) remains shared across all CLI tools — only command construction changes
- **PhaseResult contract:** Each supported CLI tool must produce a JSON file at the result path matching the `PhaseResult` schema (deserialized via `serde_json::from_str::<PhaseResult>()`). Verification of this for each non-claude CLI tool is a prerequisite for marking that tool as supported. The prompt content itself is CLI-agnostic; only the delivery mechanism (flag name, positional arg) differs per tool.
- **Agent config resolution:** If a phase has an `agent` sub-table, it is a **complete override** of the global `[agent]` — no field-level merging. `cli` is required in the per-phase `agent` block, `model` is optional. Phases without an `agent` block inherit the full global `[agent]` config. An `agent` sub-table without a `cli` field is a config validation error.
  - Example: global `cli = "claude", model = "opus"` + phase `agent = { cli = "opencode", model = "gpt-4o" }` → phase uses opencode with gpt-4o
  - Example: global `cli = "claude", model = "opus"` + phase `agent = { cli = "claude" }` (no model) → phase uses claude with **no model flag** (CLI's own default), because complete override means no merging with global model
  - Example: global `cli = "claude", model = "opus"` + phase with no `agent` block → phase uses claude with opus (full global inheritance)
- **Global `[agent]` field defaults:** Each field defaults independently. `cli` defaults to `claude`. `model` defaults to `None` (use CLI's own default). A partial section like `[agent]\nmodel = "opus"` (no cli) is valid and means cli=claude, model=opus.

## Dependencies

- **Depends On:** Knowledge of each CLI tool's invocation pattern (how they accept prompts, run non-interactively, select models). This will be resolved during tech research.
- **Blocks:** Future work on effort levels, additional CLI tools, and other per-phase tuning parameters

## Risks

- [ ] CLI invocation patterns may change across versions — mitigate by keeping CLI-specific logic isolated and easy to update
- [ ] Some CLI tools may not support all architectural requirements (non-interactive mode, prompt via args, process group isolation) — mitigate by verifying during tech research phase; tools that don't meet constraints are excluded from v1 and the enum variant is kept but marked unsupported until resolved

## Open Questions

- [x] ~~Should the config field be a simple enum of known tools, or a more flexible structure?~~ **Decision: Enum** — type-safe, aligns with allowlist security model
- [x] ~~Where in `orchestrate.toml` should this live?~~ **Decision: `[agent]` section** — semantically distinct from execution params, room for future config
- [x] ~~Which CLI tools in v1?~~ **Decision: claude + opencode** — Codex CLI (OpenAI) can be added later via enum extension
- [x] ~~Per-phase CLI selection priority?~~ **Decision: Elevated to Should Have** — each phase can override CLI tool and model, enabling cost/capability tiering
- [ ] What is the exact invocation pattern for opencode? (command name, args for prompt, non-interactive flags, model selection)
- [ ] How does model selection work for each CLI tool? (claude has `--model`, opencode TBD)
- [x] ~~Should `handle_init` include the new CLI config field in generated `orchestrate.toml`?~~ **Decision: Yes, include with defaults** — makes it explicit which agent CLI and model are being used
- [x] ~~What should the config ergonomics look like for per-phase overrides?~~ **Decision: Nested `agent` sub-table** on `PhaseConfig` — groups agent concerns, scales for future params (effort, temperature)

## References

- Current hardcoded CLI invocation: `orchestrator/src/agent.rs:159` (`CliAgentRunner::run_agent`)
- `AgentRunner` trait: `orchestrator/src/agent.rs:119`
- Config structure: `orchestrator/src/config.rs`
- Subprocess execution: `orchestrator/src/agent.rs:172` (`run_subprocess_agent`)
- stdin constraint comment: `orchestrator/src/agent.rs:196`
- `handle_triage` CLI instantiation: `orchestrator/src/main.rs` (`handle_triage`)
- Preflight verification: `orchestrator/src/main.rs` (`handle_run`, `handle_triage`)
