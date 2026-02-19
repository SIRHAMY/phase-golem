use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;

use crate::types::{DimensionLevel, SizeLevel};

#[derive(Default, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct PhaseGolemConfig {
    pub project: ProjectConfig,
    pub guardrails: GuardrailsConfig,
    pub execution: ExecutionConfig,
    pub agent: AgentConfig,
    pub pipelines: HashMap<String, PipelineConfig>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ProjectConfig {
    pub prefix: String,
    pub backlog_path: String,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct GuardrailsConfig {
    pub max_size: SizeLevel,
    pub max_complexity: DimensionLevel,
    pub max_risk: DimensionLevel,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct ExecutionConfig {
    pub phase_timeout_minutes: u32,
    pub max_retries: u32,
    pub default_phase_cap: u32,
    pub max_wip: u32,
    pub max_concurrent: u32,
}

#[derive(Default, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
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

    pub fn build_args(&self, prompt: &str, model: Option<&str>) -> Vec<String> {
        match self {
            CliTool::Claude => {
                let mut args = vec!["--dangerously-skip-permissions".to_string()];
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

    pub fn install_hint(&self) -> &str {
        match self {
            CliTool::Claude => "Install: https://docs.anthropic.com/en/docs/claude-code",
            CliTool::OpenCode => "Install: https://github.com/opencode-ai/opencode",
        }
    }
}

#[derive(Default, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    pub cli: CliTool,
    pub model: Option<String>,
}

#[derive(Default, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StalenessAction {
    #[default]
    Ignore,
    Warn,
    Block,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PhaseConfig {
    pub name: String,
    /// Relative file paths to workflow files (relative to project root).
    #[serde(default)]
    pub workflows: Vec<String>,
    #[serde(alias = "destructive")]
    pub is_destructive: bool,
    #[serde(default)]
    pub staleness: StalenessAction,
}

impl PhaseConfig {
    /// Construct a PhaseConfig with sensible defaults for workflows and staleness.
    ///
    /// Defaults: `workflows` = `vec![]`, `staleness` = `StalenessAction::Ignore`.
    /// These match the `#[serde(default)]` field attributes on the struct
    /// to keep programmatic and deserialized configs consistent.
    pub fn new(name: &str, is_destructive: bool) -> Self {
        Self {
            name: name.to_string(),
            workflows: vec![],
            is_destructive,
            staleness: StalenessAction::Ignore,
        }
    }
}

#[derive(Default, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct PipelineConfig {
    pub pre_phases: Vec<PhaseConfig>,
    pub phases: Vec<PhaseConfig>,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            prefix: "WRK".to_string(),
            backlog_path: "BACKLOG.yaml".to_string(),
        }
    }
}

impl Default for GuardrailsConfig {
    fn default() -> Self {
        Self {
            max_size: SizeLevel::Medium,
            max_complexity: DimensionLevel::Medium,
            max_risk: DimensionLevel::Low,
        }
    }
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            phase_timeout_minutes: 30,
            max_retries: 2,
            default_phase_cap: 100,
            max_wip: 1,
            max_concurrent: 1,
        }
    }
}

pub fn default_feature_pipeline() -> PipelineConfig {
    PipelineConfig {
        pre_phases: vec![PhaseConfig {
            workflows: vec![
                ".claude/skills/changes/workflows/orchestration/research-scope.md".to_string(),
            ],
            ..PhaseConfig::new("research", false)
        }],
        phases: vec![
            PhaseConfig {
                workflows: vec![".claude/skills/changes/workflows/0-prd/create-prd.md".to_string()],
                ..PhaseConfig::new("prd", false)
            },
            PhaseConfig {
                workflows: vec![
                    ".claude/skills/changes/workflows/1-tech-research/tech-research.md".to_string(),
                ],
                ..PhaseConfig::new("tech-research", false)
            },
            PhaseConfig {
                workflows: vec![".claude/skills/changes/workflows/2-design/design.md".to_string()],
                ..PhaseConfig::new("design", false)
            },
            PhaseConfig {
                workflows: vec![
                    ".claude/skills/changes/workflows/3-spec/create-spec.md".to_string()
                ],
                ..PhaseConfig::new("spec", false)
            },
            PhaseConfig {
                workflows: vec![
                    ".claude/skills/changes/workflows/orchestration/build-spec-phase.md"
                        .to_string(),
                ],
                ..PhaseConfig::new("build", true)
            },
            PhaseConfig {
                workflows: vec![
                    ".claude/skills/changes/workflows/5-review/change-review.md".to_string()
                ],
                ..PhaseConfig::new("review", false)
            },
        ],
    }
}

pub fn normalize_agent_config(config: &mut PhaseGolemConfig) {
    if let Some(ref model) = config.agent.model {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            config.agent.model = None;
        } else {
            config.agent.model = Some(trimmed.to_string());
        }
    }
}

pub fn validate(config: &PhaseGolemConfig) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();

    if config.execution.max_wip < 1 {
        errors.push("execution.max_wip must be >= 1".to_string());
    }

    if config.execution.max_concurrent < 1 {
        errors.push("execution.max_concurrent must be >= 1".to_string());
    }

    if let Some(ref model) = config.agent.model {
        let is_valid = !model.is_empty()
            && model
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'));
        if !is_valid {
            errors.push(
                "agent.model contains invalid characters (allowed: alphanumeric, '.', '_', '/', '-')"
                    .to_string(),
            );
        } else if model.starts_with('-') {
            errors.push(
                "agent.model must not start with '-' (flag-like values are rejected)".to_string(),
            );
        }
    }

    for (pipeline_name, pipeline) in &config.pipelines {
        if pipeline.phases.is_empty() {
            errors.push(format!(
                "pipelines.{}: must have at least one main phase",
                pipeline_name
            ));
        }

        // Check phase name uniqueness across pre_phases + phases
        let mut seen_names = HashSet::new();
        for phase in pipeline.pre_phases.iter().chain(pipeline.phases.iter()) {
            if !seen_names.insert(&phase.name) {
                errors.push(format!(
                    "pipelines.{}: duplicate phase name '{}'",
                    pipeline_name, phase.name
                ));
            }
        }

        // destructive rejected on pre_phases
        for phase in &pipeline.pre_phases {
            if phase.is_destructive {
                errors.push(format!(
                    "pipelines.{}: pre_phase '{}' cannot be destructive",
                    pipeline_name, phase.name
                ));
            }
        }

        // staleness: block rejected when max_wip > 1
        if config.execution.max_wip > 1 {
            for phase in pipeline.pre_phases.iter().chain(pipeline.phases.iter()) {
                if phase.staleness == StalenessAction::Block {
                    errors.push(format!(
                        "pipelines.{}: phase '{}' uses staleness 'block' which is incompatible with max_wip > 1",
                        pipeline_name, phase.name
                    ));
                }
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Load config from an explicit path (if provided) or fall back to `{project_root}/phase-golem.toml`.
///
/// When `config_path` is `Some`, the file MUST exist — returns an error if missing.
/// When `config_path` is `None`, delegates to `load_config` (returns defaults if missing).
pub fn load_config_from(
    config_path: Option<&Path>,
    project_root: &Path,
) -> Result<PhaseGolemConfig, String> {
    match config_path {
        Some(path) => load_config_at(path),
        None => load_config(project_root),
    }
}

/// Load config from a specific file path. Errors if the file does not exist.
fn load_config_at(path: &Path) -> Result<PhaseGolemConfig, String> {
    if !path.exists() {
        return Err(format!("Config file not found: {}", path.display()));
    }

    let contents = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;

    let mut config: PhaseGolemConfig = toml::from_str(&contents)
        .map_err(|e| format!("Failed to parse {}: {}", path.display(), e))?;

    normalize_agent_config(&mut config);
    populate_default_pipelines(&mut config);

    validate(&config).map_err(|errors| {
        format!(
            "Config validation failed:\n{}",
            errors
                .iter()
                .map(|e| format!("  - {}", e))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })?;

    Ok(config)
}

pub fn load_config(project_root: &Path) -> Result<PhaseGolemConfig, String> {
    let config_path = project_root.join("phase-golem.toml");

    if !config_path.exists() {
        let mut config = PhaseGolemConfig::default();
        populate_default_pipelines(&mut config);
        return Ok(config);
    }

    let contents = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read {}: {}", config_path.display(), e))?;

    let mut config: PhaseGolemConfig = toml::from_str(&contents)
        .map_err(|e| format!("Failed to parse {}: {}", config_path.display(), e))?;

    normalize_agent_config(&mut config);
    populate_default_pipelines(&mut config);

    validate(&config).map_err(|errors| {
        format!(
            "Config validation failed:\n{}",
            errors
                .iter()
                .map(|e| format!("  - {}", e))
                .collect::<Vec<_>>()
                .join("\n")
        )
    })?;

    Ok(config)
}

fn populate_default_pipelines(config: &mut PhaseGolemConfig) {
    if config.pipelines.is_empty() {
        config
            .pipelines
            .insert("feature".to_string(), default_feature_pipeline());
    }
}
