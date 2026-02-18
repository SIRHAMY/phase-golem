# Tech Research: Multi-CLI Agent Support

**ID:** HAMY-001
**Status:** Complete
**Created:** 2026-02-17
**PRD:** ./HAMY-001_multi-cli-agent-support_PRD.md
**Mode:** Medium

## Overview

Research how to support multiple AI CLI tools (claude, opencode, and potentially others) in phase-golem's pipeline execution. The PRD proposes an enum-based allowlist of CLI tools with per-tool invocation patterns. We also investigated whether the Agent Client Protocol (ACP) could provide a standard interface that replaces per-CLI special-casing entirely.

## Research Questions

- [x] What is the Agent Client Protocol (ACP) and does it define a standard CLI invocation interface?
- [x] Could ACP give phase-golem access to many AI CLIs through a single integration?
- [x] What is the exact invocation pattern for OpenCode CLI? (command, args, prompt passing, non-interactive mode, model selection)
- [x] How does each CLI tool handle: prompt via args, model selection, non-interactive/headless execution, permission skipping?
- [x] What patterns exist for multi-backend CLI tool abstraction in Rust?
- [x] What does the existing phase-golem codebase look like around agent execution and config?

---

## External Research

### Landscape Overview

The AI coding CLI landscape in early 2026 has 15+ serious tools. Major players: **Claude Code** (Anthropic), **Codex CLI** (OpenAI), **Gemini CLI** (Google), **OpenCode** (Anomaly/SST), **Aider**, **Goose** (Block), **Cline**. Nearly all support non-interactive/headless execution, making them viable as subprocess backends.

Two protocol standardization efforts exist, both confusingly called "ACP":
1. **Agent Client Protocol** (Zed Industries) -- editor-agent integration via JSON-RPC/stdio. **This is the relevant one.**
2. **Agent Communication Protocol** (IBM/Linux Foundation) -- agent-to-agent REST APIs. Not relevant here.

### Agent Client Protocol (ACP) -- Deep Dive

**What it is:** An open standard by Zed Industries that standardizes communication between code editors and AI coding agents. Described as "the LSP for AI coding agents." Apache 2.0 license.

- **Repo:** [github.com/agentclientprotocol/agent-client-protocol](https://github.com/agentclientprotocol/agent-client-protocol) (2.1k stars, 152 forks, 66 contributors)
- **Latest release:** v0.10.8 (February 2026)
- **Primary language:** Rust (97.8%)
- **Spec site:** [agentclientprotocol.com](https://agentclientprotocol.com/)

**How it works:**
- **Transport:** JSON-RPC 2.0 over stdin/stdout (local subprocess) or HTTP/WebSocket (remote, WIP)
- **Session lifecycle:** Initialization (capability negotiation) -> Task creation (send prompt) -> Execution (streaming progress, tool approval requests) -> Completion -> Shutdown
- **Key methods:** `session/initialize`, `session/new`, `session/prompt`/`task/create`, `session/update`, `session/cancel`, `task/respond`
- **Message types:** `say` (text updates, tool results) and `ask` (tool approval requests, clarifications) with streaming support

**Adoption is broad:**

| Agent | ACP Support | How |
|-------|------------|-----|
| Claude Code | Yes | `claude --acp` or `claude-code-acp` |
| OpenCode | Yes | `opencode acp` |
| Codex CLI | Yes | `codex-acp` |
| Gemini CLI | Yes | `gemini --experimental-acp` |
| Goose | Yes | Built-in |
| Cline | Yes | `cline --acp` |
| GitHub Copilot | Yes | Built-in |
| Aider | No | Not documented |

**Editors:** Zed (native), JetBrains IDEs (2025.3.2+), Neovim, Emacs, marimo, Obsidian.

**Rust SDK:** Official crate [`agent-client-protocol`](https://crates.io/crates/agent-client-protocol) -- 429K all-time downloads, 65K/month, 56 versions. Includes client and agent examples.

**Can phase-golem use ACP as a universal protocol adapter?**

Yes, architecturally viable. ACP's subprocess-based JSON-RPC means any program can be a client (not just editors). The [ACP AI SDK provider](https://ai-sdk.dev/providers/community-providers/acp) demonstrates exactly this: spawning ACP agents programmatically without any editor UI.

**However, significant caveats for phase-golem:**

1. **stdin/stdout conflict:** Phase-golem uses `Stdio::null()` because `setpgid` places child processes in a background process group, and stdin reads would cause SIGTTIN. ACP *requires* stdin/stdout for JSON-RPC communication. This is a fundamental architectural conflict.
2. **Permission auto-approval complexity:** In ACP, agents send `ask` messages for tool approval. Phase-golem would need to auto-approve all of these (currently handled by `--dangerously-skip-permissions` flag). Adds a JSON-RPC response loop.
3. **PhaseResult contract unchanged:** ACP standardizes the transport (how prompts are sent and status received), NOT the task output format. The prompt still needs to instruct the agent to write `PhaseResult` JSON to a file path.
4. **Spec still evolving:** v0.10.x with 56 versions in ~6 months suggests rapid iteration. Breaking changes possible.
5. **Complexity:** Full JSON-RPC client vs. current simple "spawn, wait, read file" model.

**References:**
- [Zed ACP page](https://zed.dev/acp) -- supported agents and editors
- [Goose blog: Intro to ACP](https://block.github.io/goose/blog/2025/10/24/intro-to-agent-client-protocol-acp/) -- technical overview
- [JetBrains ACP Registry](https://blog.jetbrains.com/ai/2026/01/acp-agent-registry/) -- agent registry
- [ACP AI SDK provider](https://ai-sdk.dev/providers/community-providers/acp) -- programmatic use
- [PromptLayer blog: ACP as LSP for AI](https://blog.promptlayer.com/agent-client-protocol-the-lsp-for-ai-coding-agents/) -- subprocess spawning details
- [Cline ACP details (DeepWiki)](https://deepwiki.com/cline/cline/12.5-agent-client-protocol-(acp)) -- JSON-RPC method flow

### Common Patterns & Approaches

#### Pattern: Direct CLI Invocation (Current PRD Design)

**How it works:** Spawn each CLI tool as a subprocess with tool-specific command-line flags. Prompt passed as arg, result read from file.

**When to use:** Small number of well-known tools, simple fire-and-wait execution model.

**Tradeoffs:**
- Pro: Simple, proven, minimal dependencies
- Pro: No protocol overhead, just spawn and wait
- Con: Each tool needs manually researched invocation patterns
- Con: Adding new tools requires code changes

#### Pattern: Protocol-Based Invocation (ACP)

**How it works:** Spawn any ACP-compatible agent as a subprocess, communicate via JSON-RPC over stdin/stdout.

**When to use:** Many tools to support, want streaming progress, rich interaction.

**Tradeoffs:**
- Pro: One protocol handles all ACP-compatible agents (10+ tools)
- Pro: Rich info: streaming progress, tool call visibility
- Pro: Official Rust SDK
- Con: More complex (JSON-RPC client, approval handling)
- Con: Conflicts with current stdin=null architecture
- Con: Spec still evolving

#### Pattern: Hybrid Approach

**How it works:** Enum-based dispatch for direct CLI invocation (v1), with ACP as a future invocation strategy.

```rust
enum InvocationMode {
    DirectCli(CliTool),   // HAMY-001: Claude, OpenCode
    Acp { command: String, args: Vec<String> },  // Future: universal adapter
}
```

**When to use:** Ship value now, add universal protocol later.

**Tradeoffs:**
- Pro: Ships immediately with known patterns
- Pro: Doesn't preclude ACP later
- Con: Two invocation paths to maintain eventually

### CLI Tool Invocation Patterns

| Tool | Command | Non-Interactive | Prompt Passing | Model Flag | Permission Skip | ACP |
|------|---------|----------------|----------------|------------|-----------------|-----|
| Claude Code | `claude` | `-p` flag | `-p <prompt>` | `--model <name>` | `--dangerously-skip-permissions` | Yes |
| OpenCode | `opencode` | `run` subcommand | `run <prompt>` (positional) | `--model <prov/model>` | Auto in `run` mode | Yes |
| Codex CLI | `codex` | `exec` subcommand | `exec <prompt>` (positional) | `--model <name>` | `--full-auto` or `--yolo` | Yes |
| Gemini CLI | `gemini` | `-p` flag | `-p <prompt>` | (config-based) | (config-based) | Yes |
| Aider | `aider` | `--message` flag | `--message <prompt>` | `--model` | `--yes` | No |
| Goose | `goose` | `run -t` | `run -t <prompt>` | (config-based) | (config-based) | Yes |

**Key observation:** Prompt passing varies significantly -- some use flags (`-p`), some use subcommands with positional args (`run <prompt>`, `exec <prompt>`). The command construction is genuinely different per tool.

### OpenCode CLI Details

**Project status:** The original `opencode-ai/opencode` repo was **archived Sept 2025**. Active development continues at [github.com/anomalyco/opencode](https://github.com/anomalyco/opencode) (maintained by Dax/Adam from SST). Current version: v1.0.68.

**Invocation pattern for phase-golem:**
```
opencode run [--model <provider/model>] [--quiet] "<prompt>"
```

- **Binary:** `opencode`
- **Display name:** "OpenCode CLI"
- **Prompt passing:** Positional argument to `run` subcommand
- **Model flag:** `--model` with `provider/model` format (e.g., `anthropic/claude-sonnet-4-5-20250929`)
- **Permission skipping:** Not needed -- `run` mode auto-approves all permissions
- **Quiet mode:** `--quiet` / `-q` suppresses spinner (useful for subprocess)
- **ACP mode:** `opencode acp` starts ACP server on stdin/stdout

**Known issues:** `--model` flag in `run` has had bugs ([#1645](https://github.com/sst/opencode/issues/1645), [#4409](https://github.com/anomalyco/opencode/issues/4409)), reportedly fixed.

**References:**
- [OpenCode CLI docs](https://opencode.ai/docs/cli/)
- [OpenCode ACP docs](https://opencode.ai/docs/acp/)
- [OpenCode GitHub (active)](https://github.com/anomalyco/opencode)

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Two different "ACP" protocols | Agent Client Protocol (Zed, JSON-RPC/stdio) vs Agent Communication Protocol (IBM, REST). Easy to confuse. | Always specify "Agent Client Protocol (Zed)" when referencing |
| OpenCode project confusion | `opencode-ai/opencode` is archived; active repo is `anomalyco/opencode` | Point docs/install to current repo |
| Model format differences | Claude uses bare names (`opus`), OpenCode uses `provider/model`, Codex uses its own | Treat model as opaque passthrough string (PRD already does this) |
| stdin/stdout conflict with ACP | Phase-golem uses `Stdio::null()` for SIGTTIN safety; ACP requires stdin/stdout | Keep direct CLI for v1; if ACP added later, pipe stdin/stdout for ACP agents only |
| OpenCode `run` model bugs | `--model` flag in `opencode run` has had bugs | Test model selection carefully; consider marking OpenCode as "experimental" |
| Permission semantics vary | Claude needs explicit flag, OpenCode auto-approves in `run`, Codex needs `--yolo` | Per-enum-variant permission config (PRD already does this) |
| Prompt as positional vs. flag | Claude uses `-p <prompt>` (flag), OpenCode uses `run <prompt>` (positional) | Command construction must be genuinely different per tool, not just different flags |
| ACP spec instability | 56 versions in ~6 months | Don't depend on ACP for v1; treat as future enhancement |

### Standards & Best Practices

- **Agent Client Protocol (ACP)** is the emerging standard for editor-agent communication. Its subprocess JSON-RPC architecture is general enough for orchestrator use. The Rust crate is actively maintained.
- **Model Context Protocol (MCP)** handles tool/context access (complementary to ACP: "MCP handles the what, ACP handles the where"). Not directly needed by phase-golem.
- **AGENTS.md** is an emerging convention for repo-level agent instructions. Not directly relevant to HAMY-001.
- **Subprocess isolation via process groups** (`setpgid`) is a best practice that phase-golem already implements.

---

## Internal Research

### Existing Codebase State

Phase-golem is a Rust CLI tool that orchestrates multi-phase AI workflows. It loads config from `phase-golem.toml`, manages a YAML backlog, and executes pipeline phases by spawning AI CLIs as subprocesses. Currently hardcoded to `claude` throughout. Clean separation: agent execution (`agent.rs`), config (`config.rs`), phase execution (`executor.rs`), scheduling (`scheduler.rs`), with a coordinator actor pattern.

### Relevant Files

| File | Key Content | Relevant Lines |
|------|-------------|---------------|
| `src/agent.rs` | `AgentRunner` trait, `CliAgentRunner` unit struct, `verify_cli_available()`, `run_subprocess_agent()`, `read_result_file()`, `MockAgentRunner` | 114-122 (trait), 125 (unit struct), 127-141 (verify), 143-153 (run_agent impl), 163-278 (subprocess infra), 316-328 (result reader), 346-375 (mock) |
| `src/config.rs` | `PhaseGolemConfig`, `ExecutionConfig`, `StalenessAction` enum, `PhaseConfig`, `load_config()`, `validate()` | 8-15 (top config), 42-49 (enum pattern), 51-60 (PhaseConfig), 162-219 (validate), 222-251 (load) |
| `src/main.rs` | `handle_run()`, `handle_triage()`, `handle_init()` -- all CliAgentRunner instantiation sites | 141-247 (init), 249-658 (run), 661-773 (triage) |
| `src/scheduler.rs` | `run_scheduler()`, `spawn_triage()` -- runner passing | 523-529 (scheduler sig), 1526-1586 (spawn_triage) |
| `src/executor.rs` | `execute_phase()`, `run_workflows_sequentially()` -- runner usage | 270-409 (execute), 412-422 (workflows), 456-459 (result path) |
| `src/types.rs` | `PhaseResult` struct with `item_id` and `phase` fields | 239-259 |

### Existing Patterns

1. **Serde enum with `rename_all = "snake_case"` + `#[default]`**: Used by `StalenessAction`, `ItemStatus`, `ResultCode`, `SizeLevel`, `PhasePool`. New `CliTool` enum should follow this exactly.
2. **Struct-level `#[serde(default)]`**: Used on `PhaseGolemConfig`, `ProjectConfig`, `ExecutionConfig`. Makes entire TOML sections optional. New `AgentConfig` should use this.
3. **Manual `Default` implementations**: Used for `ProjectConfig`, `ExecutionConfig` etc. New `AgentConfig` should follow this pattern.
4. **Config validation via `validate()`**: Returns `Result<(), Vec<String>>` for accumulated errors. New agent validation integrates here.
5. **Config loading flow**: `load_config()` -> `toml::from_str` -> `populate_default_pipelines` -> `validate()`. Model normalization fits between parse and validate.
6. **`MockAgentRunner` pattern**: Uses `Mutex<Vec<Result<PhaseResult, String>>>`, returns canned results. Completely unaffected by this change.
7. **Runner passing**: `handle_run` wraps in `Arc::new(CliAgentRunner)` -> `run_scheduler`. `handle_triage` uses directly (no Arc).
8. **Logging**: `log_info!("[prefix] message")` format with `[ITEM_ID][PHASE]` for execution.

### Reusable Components

- **`run_subprocess_agent()`** (agent.rs:163): Takes a pre-configured `Command`, handles all process lifecycle. Only command construction changes; this stays unchanged.
- **`read_result_file()`** (agent.rs:316): Reads/parses `PhaseResult` JSON. Post-read validation (item_id/phase match) can be added here or in callers.
- **`validate()` function** (config.rs:162): Existing validation framework. New agent config validation integrates here.
- **Test helpers** (tests/common/mod.rs): `setup_test_env()`, `make_item()`, `default_config()`, etc.

### Constraints

1. **`AgentRunner` trait is unchanged**: Signature `fn run_agent(&self, prompt, result_path, timeout)` stays the same. Changes are inside `CliAgentRunner`'s implementation and construction.
2. **`MockAgentRunner` is unchanged**: Doesn't depend on CLI tool config at all.
3. **stdin MUST be null**: `setpgid` means stdin reads cause SIGTTIN. Enforced in `run_subprocess_agent()`. Tool-agnostic.
4. **`PhaseResult` contract**: All CLI tools must produce same JSON result file. Enforced by prompt, not by tool.
5. **`destructive` vs `is_destructive` pre-existing bug**: `handle_init` generates TOML with `destructive = false` but struct field is `is_destructive`. Needs fix.
6. **`CliAgentRunner` is unit struct**: Changing to parameterized affects all instantiation sites but NOT trait usage.
7. **`verify_cli_available()` is static**: Needs to become instance method or take tool as parameter.

### Integration Points

1. **`config.rs`**: Add `AgentConfig` struct + `agent: AgentConfig` field to `PhaseGolemConfig`. Add `CliTool` enum.
2. **`agent.rs`**: Parameterize `CliAgentRunner` with `(CliTool, Option<String>)`. Update `verify_cli_available()` and `run_agent()`.
3. **`main.rs:handle_run()`** (line 520): Construct `CliAgentRunner` with config. Update verify call and log messages.
4. **`main.rs:handle_triage()`** (line 680): Same construction changes.
5. **`main.rs:handle_init()`** (lines 179-207): Add `[agent]` section to template. Fix `destructive` -> `is_destructive` bug.
6. **`agent.rs:read_result_file()`** or callers: Add post-read item_id/phase validation.

### Agent Execution Flow (End-to-End)

```
Config load -> CLI verify -> Runner construction -> Scheduler receives runner
-> Executor calls runner.run_agent() -> CliAgentRunner constructs Command
-> run_subprocess_agent(cmd, result_path, timeout) -> spawn child with setpgid
-> wait with timeout -> read_result_file() -> PhaseResult flows back
```

**Change point:** Step "CliAgentRunner constructs Command" -- instead of hardcoding `"claude"`, looks at `self.cli_tool` for binary name, prompt flags, permission flag, and model flag. All other steps unchanged.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Enum-based allowlist is the right approach | ACP exists as a standard protocol with broad adoption (10+ tools) and a Rust SDK. Could replace per-tool enum variants with a single protocol adapter. | Design should not preclude adding ACP as a future invocation mode. Consider whether HAMY-001 should scope down to just direct CLI + forward-compatible design, or include ACP. |
| Two initial tools (claude + opencode) | Codex CLI is equally mature with clean non-interactive support (`codex exec`). Goose and Gemini CLI also viable. | May want to include Codex CLI as a third variant, or defer all non-claude to ACP. |
| Prompt passing is "just different flags" | OpenCode and Codex use subcommands with positional args (`run <prompt>`, `exec <prompt>`), not flags. Prompt delivery is structurally different. | The enum's prompt-passing data needs to support both flag-based (`-p <prompt>`) and subcommand-based (`run <prompt>`) patterns. Not just a list of flags. |
| OpenCode invocation is "TBD" | Resolved: `opencode run [--model provider/model] "<prompt>"`. Auto-approves permissions in `run` mode. No explicit permission-skip flag needed. | OpenCode enum variant can define empty permission-skip flags. The PRD's `["--dangerously-skip-permissions"]` pattern doesn't apply uniformly. |
| Model is "free-form optional string passed through" | Correct, but model format is genuinely tool-specific (bare name vs provider/model). Users need to know the format for their tool. | Config comments or docs should note format expectations per tool. |
| `verify_cli_available()` just checks PATH | Current implementation runs `claude --version`. Other tools may have different version check patterns. | Verify should run `<binary> --version` or equivalent; may need per-tool version check command. |

---

## Critical Areas

### Prompt Delivery Mechanism Variance

**Why it's critical:** The command construction is the core change. Getting it wrong means phases silently fail or produce garbage.

**Why it's easy to miss:** It's tempting to model prompt delivery as "a list of prefix flags before the prompt" but OpenCode/Codex use subcommands where the prompt is a positional arg, not a flag value. The data model needs to capture this structural difference.

**What to watch for:** The enum's associated data for prompt passing should support at minimum: (a) flag-then-value (`-p <prompt>`), and (b) subcommand-then-positional (`run <prompt>`). A simple `Vec<String>` prefix isn't sufficient without thought.

### ACP Architectural Compatibility

**Why it's critical:** If ACP is the future direction, designing HAMY-001 in a way that makes ACP hard to add later would be costly.

**Why it's easy to miss:** The PRD focuses on enum dispatch which is orthogonal to ACP. But the subprocess infrastructure (`Stdio::null()`, fire-and-wait model) is fundamentally incompatible with ACP's bidirectional JSON-RPC.

**What to watch for:** Keep the invocation strategy separate from the agent runner interface. A future `AcpAgentRunner` (or `InvocationMode::Acp` variant) should be possible without changing the `AgentRunner` trait.

### OpenCode Stability

**Why it's critical:** OpenCode has had a turbulent history (archived repo, fork, model flag bugs). If phase-golem ships OpenCode support and the tool is unreliable, users will blame phase-golem.

**Why it's easy to miss:** The active development and version numbers (v1.0.68) look mature, but the bug history around `--model` flag and the project instability suggest caution.

**What to watch for:** Consider keeping OpenCode support as "experimental" with clear messaging. Require manual smoke testing before removing the experimental label.

---

## Deep Dives

_To be filled during Q&A_

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| Should HAMY-001 scope change to include ACP, or keep enum-only and add ACP in follow-up? | ACP could be the primary interface instead of a per-tool enum. Changes architecture significantly. | (A) Enum-only for v1, ACP later (simpler, ships faster). (B) ACP-first, enum as fallback (more future-proof, more complex). (C) Ship enum, design to not preclude ACP. |
| Should Codex CLI be a third enum variant in v1? | It's equally mature as OpenCode with clean non-interactive support. | (A) Include it (low marginal cost given the enum pattern). (B) Defer (keep v1 scope tight). |
| How should the enum model prompt delivery variance? | Flags (`-p <prompt>`) vs subcommands (`run <prompt>`) are structurally different. | (A) Method on enum that returns full args vec. (B) Enum carries structured data describing the pattern. (C) Trait method per variant. |
| Does the `destructive` vs `is_destructive` bug need fixing as part of this change? | It's a pre-existing config bug that will bite anyone using `handle_init`. | (A) Fix alongside (small scope addition). (B) Fix separately before. |

### Recommended Approaches

#### Overall Architecture: Enum-Based Direct CLI (v1) + ACP-Ready Design

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Enum-only (PRD design) | Simple, type-safe, ships fast | Adding tools needs code changes | Small tool set, well-known patterns |
| ACP-first | Universal adapter, one integration | Complex, stdin conflict, spec instability | Many tools, ACP is stable |
| **Hybrid: Enum now, ACP later** | **Ships value now, doesn't preclude ACP** | **Two paths to maintain eventually** | **Shipping is priority, ACP is maturing** |

**Initial recommendation:** Ship HAMY-001 with enum-based direct CLI invocation (claude + opencode). Design the runner construction so a future `AcpAgentRunner` or `InvocationMode::Acp` variant can coexist. ACP becomes HAMY-002 or HAMY-001c.

#### Prompt Delivery Modeling

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Method on enum returning full args | Clean API, hides complexity | Logic in method, not data | Different tools need genuinely different logic |
| Structured data (prefix args, prompt position) | Declarative, easy to add tools | May not capture all variance | Patterns are regular enough |
| **Method on enum (`fn build_command_args`)** | **Most flexible, each variant controls its own args** | **Logic spread across variants** | **Prompt patterns vary structurally** |

**Initial recommendation:** Each `CliTool` enum variant implements a method like `fn build_command_args(&self, prompt: &str, model: Option<&str>) -> Vec<String>` that returns the full args list. This is flexible enough for both `-p <prompt>` and `run <prompt>` patterns.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Agent Client Protocol spec](https://agentclientprotocol.com/) | Spec | The emerging standard for agent communication |
| [ACP GitHub repo](https://github.com/agentclientprotocol/agent-client-protocol) | Code | Protocol source, schema, Rust examples |
| [ACP Rust crate](https://crates.io/crates/agent-client-protocol) | Library | Official Rust SDK (429K downloads) |
| [Zed ACP page](https://zed.dev/acp) | Docs | Supported agents and editors list |
| [ACP AI SDK provider](https://ai-sdk.dev/providers/community-providers/acp) | Example | Shows ACP used programmatically without editor |
| [Goose blog: Intro to ACP](https://block.github.io/goose/blog/2025/10/24/intro-to-agent-client-protocol-acp/) | Article | Good technical overview |
| [JetBrains ACP Registry](https://blog.jetbrains.com/ai/2026/01/acp-agent-registry/) | Docs | Breadth of ACP adoption |
| [OpenCode CLI docs](https://opencode.ai/docs/cli/) | Docs | CLI flags, `run` command |
| [OpenCode ACP docs](https://opencode.ai/docs/acp/) | Docs | ACP server mode |
| [Codex CLI reference](https://developers.openai.com/codex/cli/reference/) | Docs | Full flag reference |
| [Codex non-interactive mode](https://developers.openai.com/codex/noninteractive/) | Docs | `codex exec` details |
| [Gemini CLI headless](https://geminicli.com/docs/cli/headless/) | Docs | Non-interactive invocation |
| [Aider scripting docs](https://aider.chat/docs/scripting.html) | Docs | `--message` and `--yes` flags |
| [enum_dispatch crate](https://docs.rs/enum_dispatch/latest/enum_dispatch/) | Library | Rust enum dispatch pattern |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-17 | Initial research launch (external + internal agents) | ACP discovered as major finding; OpenCode invocation resolved; codebase mapped |
| 2026-02-17 | PRD concern analysis | 6 concerns identified; 4 critical areas flagged |
