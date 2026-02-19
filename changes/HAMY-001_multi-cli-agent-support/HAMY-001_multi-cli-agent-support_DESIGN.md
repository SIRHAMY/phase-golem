# Design: Multi-CLI Agent Support

**ID:** HAMY-001
**Status:** Complete
**Created:** 2026-02-17
**PRD:** ./HAMY-001_multi-cli-agent-support_PRD.md
**Tech Research:** ./HAMY-001_multi-cli-agent-support_TECH_RESEARCH.md
**Mode:** Medium

## Overview

Add a `CliTool` enum and `AgentConfig` struct to phase-golem so users can configure which AI CLI tool runs pipeline phases. The `CliTool` enum owns its invocation logic via methods (`build_args`, `binary_name`, `display_name`), keeping all CLI-specific knowledge in one place. `CliAgentRunner` becomes a parameterized struct carrying the resolved `(CliTool, Option<String>)` from config. The `AgentRunner` trait, `MockAgentRunner`, `run_subprocess_agent()`, and `read_result_file()` are all unchanged. Default behavior remains `claude` with no config changes required.

---

## System Design

### High-Level Architecture

```
phase-golem.toml          config.rs                 agent.rs                  (unchanged)
┌──────────────┐     ┌──────────────────┐     ┌──────────────────┐     ┌──────────────────┐
│ [agent]      │     │ PhaseGolemConfig  │     │ AgentRunner trait│     │run_subprocess_   │
│ cli = "claude│────▶│   agent:          │────▶│ (unchanged)      │────▶│agent()           │
│ model="sonnet│     │     AgentConfig   │     │                  │     │ (unchanged)      │
│              │     │       cli: CliTool│     │ CliAgentRunner { │     │                  │
└──────────────┘     │       model: Opt  │     │   tool: CliTool  │     │ read_result_file │
                     └──────────────────┘     │   model: Opt<Str>│     │ (unchanged)      │
                                              │ }                │     └──────────────────┘
                                              │                  │
                                              │ .run_agent():    │
                                              │   self.tool      │
                                              │     .build_args()│
                                              │   → Command      │
                                              └──────────────────┘
```

**Change boundary:** Only config parsing (new struct/enum) and command construction (inside `CliAgentRunner::run_agent`) change. Everything downstream of `Command` creation — subprocess management, result file reading, scheduling, execution — is untouched.

### Component Breakdown

#### CliTool Enum (new, in `config.rs`)

**Purpose:** Defines the set of supported AI CLI tools and encapsulates all tool-specific invocation knowledge.

**Responsibilities:**
- Deserialize from TOML string (e.g., `"claude"`, `"opencode"`) via `serde(rename_all = "snake_case")`
- Provide binary name for PATH lookup (`binary_name() -> &str`)
- Provide display name for logs/errors (`display_name() -> &str`)
- Build the full argument list for a given prompt and optional model (`build_args(prompt, model) -> Vec<String>`)
- Provide version check command args (`version_args() -> Vec<&str>`)

**Interfaces:**
- Input: prompt string, optional model string
- Output: `Vec<String>` of command-line arguments

**Dependencies:** None (pure data + logic)

**Shape:**

```rust
#[derive(Default, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CliTool {
    #[default]
    Claude,
    OpenCode,
}

impl CliTool {
    pub fn binary_name(&self) -> &str {
        match self {
            CliTool::Claude => "claude",
            CliTool::OpenCode => "opencode",
        }
    }

    pub fn display_name(&self) -> &str {
        match self {
            CliTool::Claude => "Claude CLI",
            CliTool::OpenCode => "OpenCode CLI",
        }
    }

    /// Build the full args vector for invoking this tool with a prompt.
    pub fn build_args(&self, prompt: &str, model: Option<&str>) -> Vec<String> {
        match self {
            CliTool::Claude => {
                let mut args = vec![
                    "--dangerously-skip-permissions".to_string(),
                ];
                if let Some(m) = model {
                    args.push("--model".to_string());
                    args.push(m.to_string());
                }
                args.push("-p".to_string());
                args.push(prompt.to_string());
                args
            }
            CliTool::OpenCode => {
                let mut args = vec!["run".to_string()];
                if let Some(m) = model {
                    args.push("--model".to_string());
                    args.push(m.to_string());
                }
                args.push("--quiet".to_string());
                args.push(prompt.to_string());
                args
            }
        }
    }

    pub fn version_args(&self) -> Vec<&str> {
        match self {
            CliTool::Claude => vec!["--version"],
            CliTool::OpenCode => vec!["--version"],
        }
    }
}
```

**Design note on prompt position:** Claude uses flag-then-value (`-p <prompt>`) where the prompt is the last argument. OpenCode uses a subcommand with positional arg (`run <prompt>`) where the prompt is also the last argument. Both patterns naturally place the prompt at the end of the args vector, so `build_args` returns a flat `Vec<String>` without needing to model "flag vs. positional" as a separate concept.

**Design note on version_args():** Currently returns `["--version"]` for both tools. This method exists as a forward-looking hook — future tools may use different version check patterns (e.g., `opencode version` subcommand). If this indirection is deemed unnecessary, it can be inlined as a constant in `verify_cli_available()`.

**Design note on tools not included:** Codex CLI (OpenAI) was considered as a third enum variant — tech research found it equally mature with clean non-interactive support (`codex exec`). Deferred to keep v1 scope tight; can be added via enum extension with one variant and one `build_args` match arm. Same applies to Gemini CLI, Aider, and Goose.

#### AgentConfig Struct (new, in `config.rs`)

**Purpose:** Holds the global agent configuration from `[agent]` in `phase-golem.toml`.

**Responsibilities:**
- Deserialize from TOML with field-level defaults (`cli` defaults to `Claude`, `model` defaults to `None`)
- Reject unknown fields at deserialization time (`deny_unknown_fields`)
- Normalize empty/whitespace-only model strings to `None` (via shared helper post-deserialization)
- Validate model string does not start with `-` or `--` (flag-prefix check only; `Command::args()` prevents shell injection)

**Shape:**

```rust
#[derive(Default, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    pub cli: CliTool,
    pub model: Option<String>,
}
```

**Note:** `#[derive(Default)]` is sufficient — `CliTool` derives `Default` with `#[default]` on `Claude`, and `Option<String>` defaults to `None`. No manual `Default` impl needed.

**PhaseConfig** also gets `deny_unknown_fields` and a serde alias for the pre-existing bug:

```rust
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PhaseConfig {
    pub name: String,
    #[serde(default)]
    pub workflows: Vec<String>,
    #[serde(alias = "destructive")]
    pub is_destructive: bool,
    #[serde(default)]
    pub staleness: StalenessAction,
}
```

**Integration with PhaseGolemConfig:**

```rust
#[derive(Default, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct PhaseGolemConfig {
    pub project: ProjectConfig,
    pub guardrails: GuardrailsConfig,
    pub execution: ExecutionConfig,
    pub agent: AgentConfig,          // <-- new field
    pub pipelines: HashMap<String, PipelineConfig>,
}
```

#### CliAgentRunner (modified, in `agent.rs`)

**Purpose:** Constructs and invokes a CLI subprocess for a given prompt. Now parameterized with tool and model instead of hardcoding `claude`.

**Changes from current:**
- Unit struct `CliAgentRunner;` becomes `CliAgentRunner { tool: CliTool, model: Option<String> }`
- `verify_cli_available()` becomes `verify_cli_available(&self)` (instance method using `self.tool`)
- `run_agent()` uses `self.tool.build_args()` instead of hardcoded args

**Shape:**

```rust
pub struct CliAgentRunner {
    pub tool: CliTool,
    pub model: Option<String>,
}

impl CliAgentRunner {
    pub fn new(tool: CliTool, model: Option<String>) -> Self {
        Self { tool, model }
    }

    /// Verify that the configured CLI tool is available on PATH.
    pub fn verify_cli_available(&self) -> Result<(), String> {
        let output = std::process::Command::new(self.tool.binary_name())
            .args(self.tool.version_args())
            .output()
            .map_err(|e| {
                format!(
                    "{} not found on PATH. Install it first. ({})",
                    self.tool.display_name(),
                    e
                )
            })?;

        if !output.status.success() {
            return Err(format!(
                "{} found but version check failed",
                self.tool.display_name()
            ));
        }

        Ok(())
    }
}

impl AgentRunner for CliAgentRunner {
    async fn run_agent(
        &self,
        prompt: &str,
        result_path: &Path,
        timeout: Duration,
    ) -> Result<PhaseResult, String> {
        let mut cmd = tokio::process::Command::new(self.tool.binary_name());
        cmd.args(self.tool.build_args(prompt, self.model.as_deref()));
        run_subprocess_agent(cmd, result_path, timeout).await
    }
}
```

**AgentRunner trait is unchanged.** `MockAgentRunner` is unaffected.

### Data Flow

1. **Config load:** `load_config()` / `load_config_at()` deserializes TOML including `[agent]` section into `PhaseGolemConfig.agent: AgentConfig`
2. **Model normalization:** A shared `normalize_agent_config(config: &mut PhaseGolemConfig)` helper normalizes empty/whitespace model strings to `None`. Called from both `load_config()` and `load_config_at()` after deserialization, before `validate()`. This ensures normalization always runs regardless of which config loading path is taken.
3. **Model validation:** `validate()` checks model string for invalid characters. It can assume `model` is either `None` or a non-empty, non-whitespace string (normalization already ran). Validation rejects strings that start with `-` or `--` (flag-prefix check is prefix-only, not substring — model names like `claude-opus-4` with internal hyphens are valid).
4. **Runner construction:** `handle_run()` and `handle_triage()` create `CliAgentRunner::new(config.agent.cli.clone(), config.agent.model.clone())`. **Note:** Both fields require `.clone()` because `config` is used downstream.
5. **CLI verification:** `runner.verify_cli_available()` checks `<binary_name> --version` on PATH. **Important:** This is now an instance method, so it must be called *after* config load and runner construction. Both `handle_run` (currently line 285) and `handle_triage` (currently line 689) call the old static `CliAgentRunner::verify_cli_available()` *before* `load_config()` — these must be reordered to: load config → construct runner → verify.
6. **Startup logging:** Log the resolved agent config: CLI tool, model, and resolved binary path (e.g., `"Agent: Claude CLI (model: sonnet) at /usr/local/bin/claude"`). The binary path is obtained via `which <binary_name>` or equivalent for debugging PATH issues. If `cli = "opencode"`, also log: `"Note: OpenCode CLI support is experimental."` This logging is in `handle_run()` / `handle_triage()` after runner construction (not inside `validate()`), since `validate()` is a pure error-checking function with no side effects.
7. **Phase execution:** `runner.run_agent(prompt, result_path, timeout)` calls `self.tool.build_args()` to construct the `Command`, then delegates to `run_subprocess_agent()` (unchanged)
8. **Per-phase logging:** Each phase execution log line includes the resolved CLI tool and model (e.g., `[WRK-001][BUILD] Using Claude CLI (model: opus)`). Emitted in `execute_phase()` before calling the runner.
9. **Result reading:** `read_result_file()` is unchanged — all tools must produce the same `PhaseResult` JSON

**Note:** Post-result `item_id`/`phase` validation (PRD Must Have) was implemented in SPEC Phase 2 as `validate_result_identity()` in `executor.rs`.

### Key Flows

#### Flow: Normal Phase Execution

> A pipeline phase runs using the configured CLI tool and model.

1. **Config load** — `load_config()` parses `[agent]` section; defaults to `{cli: Claude, model: None}` if absent
2. **Runner construction** — `handle_run()` creates `CliAgentRunner::new(config.agent.cli.clone(), config.agent.model.clone())`. **Ordering change:** The current code calls `CliAgentRunner::verify_cli_available()` (static) at line 285, *before* `load_config()` at line 293. This must be reordered: config load first, then runner construction, then instance-method verification.
3. **CLI verification** — `runner.verify_cli_available()` runs `<binary> --version`
4. **Log config** — Prints agent summary including CLI tool, model, and resolved binary path (e.g., `"Agent: Claude CLI (model: sonnet) at /usr/local/bin/claude"`). If OpenCode, also logs experimental warning.
5. **Scheduler receives runner** — `Arc::new(runner)` passed to `run_scheduler()`
6. **Executor logs phase** — Prints per-phase tool/model info (e.g., `[WRK-001][BUILD] Using Claude CLI (model: opus)`)
7. **Executor calls runner** — `runner.run_agent(prompt, result_path, timeout)` in `execute_phase()`
8. **Command built** — `self.tool.build_args(prompt, self.model.as_deref())` returns args
9. **Subprocess runs** — `run_subprocess_agent(cmd, ...)` handles process lifecycle (unchanged)
10. **Result read** — `read_result_file()` parses `PhaseResult` JSON (unchanged)
**Edge cases:**
- CLI tool not on PATH — `verify_cli_available()` returns error with tool name and install suggestion
- Missing `[agent]` section — defaults apply: `cli=claude`, `model=None`
- Partial `[agent]` section (e.g., only `model`) — each field defaults independently
- Empty model string in config — normalized to `None` during load
- CLI binary disappears mid-run — `run_subprocess_agent()` returns spawn error → `PhaseExecutionResult::Failed` → retry → eventual block. Standard retry/resume semantics apply.

#### Flow: Triage Execution

> A triage operation uses the same configured CLI tool.

1. **Config load** — Same as above
2. **Runner construction** — `handle_triage()` creates `CliAgentRunner::new(config.agent.cli.clone(), config.agent.model.clone())` (no Arc — used directly via `&self`). **Same ordering change as handle_run:** current code calls static `CliAgentRunner::verify_cli_available()` at line 689, before `load_config()` at line 699. Must be reordered.
3. **CLI verification** — `runner.verify_cli_available()` (instance method)
4. **Log config** — Same startup logging as handle_run
5. **Triage call** — `runner.run_agent(triage_prompt, ...)` uses configured tool
6. **Result parsing** — Same `PhaseResult` contract

#### Flow: Project Initialization

> `handle_init` generates `phase-golem.toml` with the `[agent]` section included.

1. **Template generation** — `handle_init` includes `[agent]` section with commented defaults, placed between `[execution]` and `[pipelines]` in the generated TOML
2. **Output:**
```toml
[agent]
# cli = "claude"          # AI CLI tool: "claude", "opencode"
# model = ""              # Model override (e.g., "opus" for Claude, "anthropic/claude-sonnet-4-5" for OpenCode)
```
3. **Bug fix:** The generated TOML template also fixes the pre-existing `destructive` → `is_destructive` field name mismatch in the `[pipelines]` section (e.g., `destructive = true` becomes `is_destructive = true`). See Open Questions for scoping decision.

**Edge case:** Existing projects without `[agent]` section continue working — `serde(default)` fills in defaults.

#### Flow: Config Validation

> Validates agent config at load time.

1. **Serde deserialization** — `cli` field must be a known enum variant; unknown values produce a deserialization error (e.g., `unknown variant "foo", expected one of "claude", "opencode"`). This error surfaces via `load_config()`'s `map_err` on `toml::from_str`.
2. **Model normalization** — Empty/whitespace strings → `None` (via shared `normalize_agent_config` helper, runs in both `load_config()` and `load_config_at()`)
3. **Model validation** — In `validate()`, rejects model strings that start with `-` or `--` (flag-prefix check). Note: internal hyphens are valid (e.g., `claude-opus-4`). Since model strings pass through `Command::args()` (not a shell), shell metacharacters are not an injection risk — the flag-prefix check is the only defense-in-depth validation needed.
4. **OpenCode experimental warning** — Emitted at the call site in `handle_run()` / `handle_triage()` after runner construction, NOT inside `validate()` (which is a pure error-checking function returning `Result<(), Vec<String>>` with no side effects)

---

## Technical Decisions

### Key Decisions

#### Decision: Method-based command construction on CliTool enum

**Context:** Different CLI tools have structurally different invocation patterns. Claude uses `-p <prompt>` (flag-value), OpenCode uses `run <prompt>` (subcommand-positional). Need a data model that handles this variance.

**Decision:** Each `CliTool` variant implements `build_args(&self, prompt, model) -> Vec<String>` that returns the complete argument list. The enum owns all tool-specific invocation knowledge.

**Rationale:** This is the most flexible approach — each variant controls its own args construction without needing a shared data model for "how prompts are passed." Adding a new tool means adding one match arm. The research confirmed that prompt-passing patterns are too varied (flags, subcommands, positional) for a declarative data model to capture cleanly.

**PRD constraint note:** The PRD Constraints section specifies a structured five-field data model per enum variant (command name, display name, prompt-passing flags, model flag format, permission-skipping flag). This design intentionally supersedes that constraint based on tech research findings (TECH_RESEARCH.md, "Prompt Delivery Modeling" section): the structural variance between flag-based (`-p <prompt>`) and subcommand-based (`run <prompt>`) prompt delivery makes a declarative data model insufficient. The method approach captures this variance while still exposing individual data accessors (`binary_name()`, `display_name()`) where a simple return value suffices. Permission-skipping is embedded in `build_args()` because it interacts with argument ordering (e.g., OpenCode's `run` subcommand auto-approves permissions implicitly — no flag needed. Claude requires `--dangerously-skip-permissions` before other args. New tools should document their permission model in `build_args()`).

**Consequences:** All CLI-specific logic lives in `CliTool` methods. `CliAgentRunner::run_agent()` is generic — it just calls `self.tool.build_args()` and `self.tool.binary_name()`.

#### Decision: CliAgentRunner becomes parameterized struct (trait unchanged)

**Context:** `CliAgentRunner` is currently a unit struct. Need to carry tool and model config.

**Decision:** `CliAgentRunner { tool: CliTool, model: Option<String> }`. The `AgentRunner` trait signature is unchanged. `MockAgentRunner` is unaffected.

**Rationale:** The trait defines the execution interface (`prompt, result_path, timeout`). The tool/model config is construction-time configuration, not per-call data. Keeping the trait unchanged avoids changing its usage across `executor.rs`, `scheduler.rs`, and all test code.

**Consequences:** All call sites that create `CliAgentRunner` must pass tool and model. There are exactly three: `handle_run` (Arc-wrapped), `handle_triage` (direct), and test code (which uses `MockAgentRunner` — unaffected).

#### Decision: verify_cli_available becomes instance method

**Context:** Currently a static method `CliAgentRunner::verify_cli_available()` that hardcodes `claude`. Needs to check the configured tool.

**Decision:** Change to `&self` instance method that uses `self.tool.binary_name()` and `self.tool.version_args()`.

**Rationale:** The runner is constructed before verification. Instance method naturally accesses the configured tool. Keeps verification logic colocated with the runner.

**Consequences:** Call sites change from `CliAgentRunner::verify_cli_available()` to `runner.verify_cli_available()`. The runner must be constructed before verification, which is natural — construct, verify, then use.

#### Decision: AgentConfig with field-level serde defaults

**Context:** Need backwards compatibility — existing `phase-golem.toml` files without `[agent]` section must work.

**Decision:** `AgentConfig` uses `#[serde(default)]` at both struct and field level. `PhaseGolemConfig.agent` also has `#[serde(default)]`. `cli` defaults to `Claude`, `model` defaults to `None`.

**Rationale:** Follows the exact pattern used by `ExecutionConfig`, `ProjectConfig`, etc. in the existing codebase. Partial sections like `[agent]\nmodel = "opus"` work correctly (cli defaults to Claude).

**Consequences:** Zero-config upgrade path. Existing projects get Claude as default. New projects get explicit `[agent]` section from `handle_init`.

#### Decision: Model normalization in load_config, validation in validate()

**Context:** Model strings should not contain whitespace, shell metacharacters, or flag prefixes. Empty strings should become `None`.

**Decision:** Normalization (empty → `None`) happens in `load_config()` after deserialization. Validation (reject bad characters) happens in `validate()`.

**Rationale:** Follows the existing config loading flow: `toml::from_str` → normalization → `populate_default_pipelines` → `validate()`. Normalization is a data cleanup step (like `trim()`); validation is a correctness check.

**Consequences:** The `AgentConfig.model` field is never semantically empty after `load_config()` returns.

#### Decision: Post-result item_id/phase validation — IMPLEMENTED (Phase 2)

**Context:** PRD requires asserting `result.item_id == expected_item_id` and `result.phase == expected_phase` after reading result files.

**Decision:** Originally deferred to a separate change, but un-deferred and included in Phase 2 of the SPEC. Implemented as `validate_result_identity()` in `executor.rs`, called from `execute_phase()` after `run_workflows_sequentially` returns `Ok(phase_result)`. Returns a non-retryable error on mismatch (bypasses retry loop).

#### Decision: `deny_unknown_fields` on `PhaseConfig` and `AgentConfig`

**Context:** The `destructive` vs `is_destructive` bug was caused by serde silently ignoring unknown fields. This entire class of bug is preventable.

**Decision:** Add `#[serde(deny_unknown_fields)]` to both `PhaseConfig` and `AgentConfig`. This causes config load to fail immediately on unrecognized keys (e.g., `cli_tool = "claude"` instead of `cli = "claude"`).

**Rationale:** These are the two structs most likely to have field-name typos — `PhaseConfig` is nested inside arrays (harder to spot errors), and `AgentConfig` is new (users will be typing it for the first time). The `#[serde(alias = "destructive")]` on `is_destructive` ensures backward compat with existing configs.

**Consequences:** Existing configs with `destructive = true` continue working (via alias). Typos in new `[agent]` sections produce clear errors at config load. Future field additions to these structs must consider the deny_unknown_fields constraint.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Per-tool code in enum | Adding a new CLI tool requires a code change (new match arms) | Type safety, compile-time validation, no arbitrary command execution | The PRD explicitly chose this security model. Tools are a short, curated list. |
| OpenCode as experimental | Users get a variant that logs a warning, not a hard error | Forward compatibility — tech research validated the invocation pattern (TECH_RESEARCH.md, "OpenCode CLI Details"), so the PRD's placeholder "runtime error until validated" is replaced by an experimental warning | The PRD required a runtime error because OpenCode's invocation pattern was TBD. Tech research resolved it (`opencode run [--model provider/model] [--quiet] "<prompt>"`). The remaining risk is `--model` flag stability and PhaseResult contract conformance, which the experimental label addresses. |
| No ACP in v1 | Miss the opportunity for universal protocol support | Ship value now with proven simple architecture | ACP has a fundamental stdin/stdout conflict with current architecture. Can be added as a separate `AcpAgentRunner` later without changing the trait. |
| Model as opaque passthrough | No orchestrator-side model validation | Simplicity — CLI tools do their own model validation. Users may get opaque errors from the CLI tool. | Model formats are tool-specific (bare name vs. provider/model). Validating would require per-tool model registries which is impractical and fragile. |

---

## Alternatives Considered

### Alternative: ACP-First Architecture

**Summary:** Use the Agent Client Protocol as the primary invocation mechanism instead of per-tool enum dispatch. All ACP-compatible agents (10+) would work through a single JSON-RPC client.

**How it would work:**
- Spawn CLI tool with ACP flag (e.g., `claude --acp`, `opencode acp`)
- Communicate via JSON-RPC over stdin/stdout
- Send prompts via `session/prompt` or `task/create`
- Auto-approve all `ask` messages (equivalent to permission skipping)
- Read streaming `say` messages for progress

**Pros:**
- One integration handles 10+ tools
- Rich streaming progress and tool call visibility
- Official Rust SDK available (429K downloads)
- Future-proof as more tools adopt ACP

**Cons:**
- **Fundamental stdin/stdout conflict** — phase-golem uses `Stdio::null()` because `setpgid` places children in background process groups where stdin reads cause SIGTTIN. ACP *requires* stdin/stdout for JSON-RPC.
- Significantly more complex (JSON-RPC client, approval handling loop, message parsing)
- ACP spec still evolving rapidly (56 versions in ~6 months)
- PhaseResult contract unchanged — still needs prompt-based file output

**Why not chosen:** The stdin/stdout conflict is a fundamental architectural incompatibility that would require reworking the subprocess isolation model. The complexity increase is substantial for uncertain gain at this stage. The direct CLI approach is proven, simple, and ships immediately.

**Future path:** ACP can be added as a separate `AcpAgentRunner` implementation that uses piped stdin/stdout instead of `Stdio::null()`, with its own process management logic. The `AgentRunner` trait is flexible enough to accommodate this.

### Alternative: Trait-based dispatch (one struct per tool)

**Summary:** Instead of a `CliTool` enum, define separate structs for each tool (e.g., `ClaudeRunner`, `OpenCodeRunner`) that implement `AgentRunner`.

**How it would work:**
- `AgentRunner` trait stays the same
- `ClaudeRunner { model: Option<String> }` implements `AgentRunner`
- `OpenCodeRunner { model: Option<String> }` implements `AgentRunner`
- Config deserializes to a tag that selects which struct to construct
- Use `Box<dyn AgentRunner>` or enum dispatch

**Pros:**
- Each tool is fully encapsulated in its own struct
- Can have tool-specific fields (not just model)
- Follows OOP-style polymorphism

**Cons:**
- More boilerplate for a small number of tools (2-3 initially)
- Need dynamic dispatch (`Box<dyn AgentRunner>`) or an outer enum anyway
- `verify_cli_available()` needs to be on the trait or duplicated per struct
- The tools share 95% of their logic (all use `run_subprocess_agent`) — the only difference is arg construction

**Why not chosen:** Over-engineered for the current scope. The enum with methods is simpler, keeps shared logic together, and avoids dynamic dispatch. If tool-specific behavior diverges significantly in the future, refactoring from enum to traits is straightforward.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| OpenCode `run` mode bugs (especially `--model`) | Phases fail or use wrong model | Medium | Mark OpenCode as experimental; require smoke test before removing label |
| OpenCode `--quiet` flag may affect result file output | PhaseResult JSON not written, phases always fail | Medium | `--quiet` is documented as suppressing the spinner only, but this is unverified empirically. The smoke test (see below) must verify `--quiet` does not affect file output. |
| OpenCode PhaseResult contract conformance | OpenCode agents may not produce valid PhaseResult JSON at the expected path | High | The PhaseResult contract is enforced via prompt instructions (the workflow markdown tells the agent to write JSON). Whether OpenCode reliably follows these instructions is the primary experimental risk. **Smoke test criteria for graduating OpenCode from experimental:** (1) `opencode run` produces a valid PhaseResult JSON file when given the standard prompt, (2) `--quiet` does not suppress file output, (3) `--model` works or fails gracefully. |
| CLI tool version breaks invocation pattern | Phases fail silently or noisily | Low | All tool-specific logic in `CliTool` methods — easy to update. Version not pinned. |
| Large prompts exceed argv limits | Phase execution fails | Low | Linux `MAX_ARG_STRLEN` is ~2MB. Monitor prompt sizes; add `--prompt-file` support if needed (future). |
| Model string validation too strict | Rejects valid model names with internal hyphens | Low | Validation only checks prefix (`starts_with('-')`), not substring. Model names like `claude-opus-4` are valid. |
| `destructive` vs `is_destructive` bug in `handle_init` | Generated config files silently default `is_destructive` to `false` for all phases (including `build`) because serde ignores the unrecognized `destructive` key | Medium | Fix as part of this change. Note: this affects all existing projects initialized with `handle_init`, not just new ones. Existing configs with `destructive = true` silently behave as `is_destructive = false`. |

---

## Integration Points

### Existing Code Touchpoints

- `src/config.rs` — Add `CliTool` enum, `AgentConfig` struct (with `deny_unknown_fields`), `agent` field to `PhaseGolemConfig`. Add `#[serde(deny_unknown_fields)]` and `#[serde(alias = "destructive")]` to `PhaseConfig`. Add shared `normalize_agent_config()` helper called from both `load_config()` and `load_config_at()`. Add model validation in `validate()`.
- `src/agent.rs` — Parameterize `CliAgentRunner` with fields, change `verify_cli_available()` to instance method, update `run_agent()` to use `self.tool.build_args()`. **New import:** `use crate::config::CliTool;` (creates new `agent.rs → config.rs` dependency; no circular dependency — config.rs does not import agent.rs).
- `src/main.rs:handle_run()` — **Reorder:** Move CLI verification from pre-config (currently line 285, static call) to post-config/post-construction (after line 542). Construct `CliAgentRunner::new(config.agent.cli.clone(), config.agent.model.clone())`, call `runner.verify_cli_available()`, add startup logging (tool, model, binary path, OpenCode experimental warning if applicable).
- `src/main.rs:handle_triage()` — **Same reorder:** Move CLI verification from line 689 (static) to after config load (line 699) and runner construction (line 703). Same construction and logging changes.
- `src/main.rs:handle_init()` (~line 199) — Add `[agent]` section between `[execution]` and `[pipelines]` in template. Fix `destructive` → `is_destructive` field name in generated TOML phases.
- `src/executor.rs:execute_phase()` — Add per-phase log line with CLI tool and model. Post-result `item_id`/`phase` validation implemented in SPEC Phase 2.
- `tests/` — Update any tests that construct `CliAgentRunner` directly (most use `MockAgentRunner` — unaffected)

### External Dependencies

- **No new crates required.** The `serde` derive macros already in use handle the new enum and struct.
- **Runtime dependency (OpenCode only):** `opencode` binary on PATH. This is a runtime dependency for anyone configuring `cli = "opencode"`. The specific behavioral assumptions are: (1) `opencode run <prompt>` accepts a positional prompt argument, (2) `--quiet` suppresses interactive output without affecting file output, (3) `opencode --version` exits with status 0. These are validated by tech research but not empirically smoke-tested against a live OpenCode installation.

---

## Resolved Questions

- [x] Should OpenCode's experimental status be enforced at config validation time (error) or at runtime (warning + proceed)? **Decision:** Warning at startup, not a hard error — lets users experiment while clearly communicating the status.
- [x] Should the `destructive` → `is_destructive` fix be in this change or a separate commit? **Decision:** Same change, separate commit. Fix both the template string (new output uses `is_destructive`) AND add `#[serde(alias = "destructive")]` on the `PhaseConfig.is_destructive` field for backward compat with existing configs.
- [x] Should post-result item_id/phase validation be bundled with this change or shipped separately? **Decision:** Ship separately. It's a PRD Must Have but conceptually unrelated to multi-CLI support — shipping it independently allows focused testing of the executor change.
- [x] Should the OpenCode enum variant be included in v1, or deferred until empirically validated? **Decision:** Include with experimental warning. Forward-compatible; lets early adopters experiment.
- [x] Should model validation include metacharacter checks? **Decision:** Starts-with `-` check only. `Command::args()` prevents shell injection; metacharacter validation would be security theater.
- [x] Should `PhaseConfig` and `AgentConfig` use `#[serde(deny_unknown_fields)]`? **Decision:** Yes for both. Catches typos and field-name mismatches at config load time. The `#[serde(alias = "destructive")]` on `is_destructive` must land in the same change to avoid breaking existing configs.

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
| 2026-02-17 | Initial design draft | Enum-based CliTool with method dispatch, AgentConfig struct, parameterized CliAgentRunner |
| 2026-02-17 | Self-critique (7 agents) | 54+ findings → 2 critical, 7 high, 20+ medium. Applied 16 auto-fixes. 4 directional items for human review. |
| 2026-02-17 | Directional decisions resolved | OpenCode: include w/ experimental warning. Post-result validation: defer to separate change. destructive fix: template + serde alias. Model validation: starts-with-dash only. Added deny_unknown_fields to PhaseConfig + AgentConfig. |
| 2026-02-17 | Design finalized | All questions resolved, checklist complete, status → Complete |
