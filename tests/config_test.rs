use phase_golem::config::*;
use phase_golem::config::{AgentConfig, CliTool};
use phase_golem::types::*;

// --- backlog_path config tests ---

#[test]
fn default_backlog_path_is_backlog_yaml() {
    let config = ProjectConfig::default();
    assert_eq!(config.backlog_path, "BACKLOG.yaml");
}

#[test]
fn custom_backlog_path_parses_from_toml() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("phase-golem.toml");
    std::fs::write(
        &config_path,
        r#"
[project]
backlog_path = ".dev/BACKLOG.yaml"
"#,
    )
    .unwrap();

    let config = load_config(dir.path()).unwrap();
    assert_eq!(config.project.backlog_path, ".dev/BACKLOG.yaml");
}

// --- PhaseConfig::new() constructor tests ---

#[test]
fn phase_config_new_sets_correct_defaults() {
    let phase = PhaseConfig::new("test", false);

    assert_eq!(phase.name, "test");
    assert!(!phase.is_destructive);
    assert!(phase.workflows.is_empty());
    assert_eq!(phase.staleness, StalenessAction::Ignore);
}

#[test]
fn phase_config_new_with_destructive_true() {
    let phase = PhaseConfig::new("build", true);

    assert_eq!(phase.name, "build");
    assert!(phase.is_destructive);
    assert!(phase.workflows.is_empty());
    assert_eq!(phase.staleness, StalenessAction::Ignore);
}

#[test]
fn phase_config_new_matches_serde_defaults() {
    let toml_str = r#"
[[pipelines.test.phases]]
name = "build"
is_destructive = true
"#;
    let config: PhaseGolemConfig = toml::from_str(toml_str).unwrap();
    let deserialized = &config.pipelines["test"].phases[0];

    let constructed = PhaseConfig::new("build", true);

    assert_eq!(*deserialized, constructed);
}

#[test]
fn load_config_defaults_when_file_missing() {
    let dir = tempfile::tempdir().unwrap();
    let config = load_config(dir.path()).unwrap();

    assert_eq!(config.project.prefix, "WRK");
    assert_eq!(config.guardrails.max_size, SizeLevel::Medium);
    assert_eq!(config.guardrails.max_complexity, DimensionLevel::Medium);
    assert_eq!(config.guardrails.max_risk, DimensionLevel::Low);
    assert_eq!(config.execution.phase_timeout_minutes, 30);
    assert_eq!(config.execution.max_retries, 2);
    assert_eq!(config.execution.default_phase_cap, 100);
}

#[test]
fn load_config_full() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("phase-golem.toml");
    std::fs::write(
        &config_path,
        r#"
[project]
prefix = "APP"

[guardrails]
max_size = "large"
max_complexity = "high"
max_risk = "medium"

[execution]
phase_timeout_minutes = 60
max_retries = 5
default_phase_cap = 50
"#,
    )
    .unwrap();

    let config = load_config(dir.path()).unwrap();

    assert_eq!(config.project.prefix, "APP");
    assert_eq!(config.guardrails.max_size, SizeLevel::Large);
    assert_eq!(config.guardrails.max_complexity, DimensionLevel::High);
    assert_eq!(config.guardrails.max_risk, DimensionLevel::Medium);
    assert_eq!(config.execution.phase_timeout_minutes, 60);
    assert_eq!(config.execution.max_retries, 5);
    assert_eq!(config.execution.default_phase_cap, 50);
}

#[test]
fn load_config_partial_uses_defaults_for_missing() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("phase-golem.toml");
    std::fs::write(
        &config_path,
        r#"
[project]
prefix = "CUSTOM"
"#,
    )
    .unwrap();

    let config = load_config(dir.path()).unwrap();

    assert_eq!(config.project.prefix, "CUSTOM");
    // Guardrails and execution should use defaults
    assert_eq!(config.guardrails.max_size, SizeLevel::Medium);
    assert_eq!(config.guardrails.max_risk, DimensionLevel::Low);
    assert_eq!(config.execution.phase_timeout_minutes, 30);
    assert_eq!(config.execution.max_retries, 2);
}

#[test]
fn load_config_empty_file_uses_all_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("phase-golem.toml");
    std::fs::write(&config_path, "").unwrap();

    let config = load_config(dir.path()).unwrap();

    assert_eq!(config.project, ProjectConfig::default());
    assert_eq!(config.guardrails, GuardrailsConfig::default());
    assert_eq!(config.execution, ExecutionConfig::default());
    // Default feature pipeline is auto-generated when no pipelines defined
    assert!(config.pipelines.contains_key("feature"));
    assert_eq!(config.pipelines.len(), 1);
}

#[test]
fn load_config_invalid_toml_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("phase-golem.toml");
    std::fs::write(&config_path, "this is not valid toml [[[").unwrap();

    let result = load_config(dir.path());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("Failed to parse"));
}

#[test]
fn load_config_invalid_enum_value_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("phase-golem.toml");
    std::fs::write(
        &config_path,
        r#"
[guardrails]
max_size = "extra_large"
"#,
    )
    .unwrap();

    let result = load_config(dir.path());
    assert!(result.is_err());
}

// --- Pipeline config tests ---

#[test]
fn load_config_with_full_pipeline() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("phase-golem.toml");
    std::fs::write(
        &config_path,
        r#"
[pipelines.blog-post]
pre_phases = [
    { name = "research", workflows = ["research/scope"], is_destructive = false },
]
phases = [
    { name = "draft", workflows = ["writing/draft"], is_destructive = false },
    { name = "edit", workflows = ["writing/edit"], is_destructive = false },
    { name = "publish", workflows = ["writing/publish"], is_destructive = true, staleness = "warn" },
]
"#,
    )
    .unwrap();

    let config = load_config(dir.path()).unwrap();

    assert!(config.pipelines.contains_key("blog-post"));
    let pipeline = &config.pipelines["blog-post"];
    assert_eq!(pipeline.pre_phases.len(), 1);
    assert_eq!(pipeline.pre_phases[0].name, "research");
    assert!(!pipeline.pre_phases[0].is_destructive);
    assert_eq!(pipeline.phases.len(), 3);
    assert_eq!(pipeline.phases[0].name, "draft");
    assert!(!pipeline.phases[0].is_destructive);
    assert_eq!(pipeline.phases[2].name, "publish");
    assert!(pipeline.phases[2].is_destructive);
    assert_eq!(pipeline.phases[2].staleness, StalenessAction::Warn);
}

#[test]
fn load_config_with_partial_pipeline_uses_phase_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("phase-golem.toml");
    std::fs::write(
        &config_path,
        r#"
[pipelines.simple]
phases = [
    { name = "build", workflows = ["build/run"], is_destructive = true },
]
"#,
    )
    .unwrap();

    let config = load_config(dir.path()).unwrap();

    let pipeline = &config.pipelines["simple"];
    assert!(pipeline.pre_phases.is_empty());
    assert_eq!(pipeline.phases.len(), 1);
    assert!(pipeline.phases[0].is_destructive);
    assert_eq!(pipeline.phases[0].staleness, StalenessAction::Ignore);
}

#[test]
fn load_config_missing_pipelines_section_generates_default() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("phase-golem.toml");
    std::fs::write(
        &config_path,
        r#"
[project]
prefix = "TEST"
"#,
    )
    .unwrap();

    let config = load_config(dir.path()).unwrap();

    assert_eq!(config.pipelines.len(), 1);
    assert!(config.pipelines.contains_key("feature"));

    let feature = &config.pipelines["feature"];
    assert_eq!(feature.pre_phases.len(), 1);
    assert_eq!(feature.pre_phases[0].name, "research");
    assert_eq!(feature.phases.len(), 6);
    assert_eq!(feature.phases[0].name, "prd");
    assert_eq!(feature.phases[4].name, "build");
    assert!(feature.phases[4].is_destructive);
    assert_eq!(feature.phases[5].name, "review");
}

#[test]
fn load_config_with_explicit_pipelines_does_not_add_default() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("phase-golem.toml");
    std::fs::write(
        &config_path,
        r#"
[pipelines.custom]
phases = [
    { name = "do-stuff", workflows = ["stuff/do"], is_destructive = false },
]
"#,
    )
    .unwrap();

    let config = load_config(dir.path()).unwrap();

    assert_eq!(config.pipelines.len(), 1);
    assert!(config.pipelines.contains_key("custom"));
    assert!(!config.pipelines.contains_key("feature"));
}

#[test]
fn load_config_with_max_wip_and_max_concurrent() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("phase-golem.toml");
    std::fs::write(
        &config_path,
        r#"
[execution]
max_wip = 3
max_concurrent = 2
"#,
    )
    .unwrap();

    let config = load_config(dir.path()).unwrap();

    assert_eq!(config.execution.max_wip, 3);
    assert_eq!(config.execution.max_concurrent, 2);
}

#[test]
fn load_config_defaults_max_wip_and_max_concurrent() {
    let dir = tempfile::tempdir().unwrap();
    let config = load_config(dir.path()).unwrap();

    assert_eq!(config.execution.max_wip, 1);
    assert_eq!(config.execution.max_concurrent, 1);
}

// --- Validation tests ---

#[test]
fn validate_valid_config_passes() {
    let dir = tempfile::tempdir().unwrap();
    let config = load_config(dir.path()).unwrap();
    assert!(validate(&config).is_ok());
}

#[test]
fn validate_max_wip_zero_fails() {
    let mut config = PhaseGolemConfig::default();
    config.execution.max_wip = 0;
    config.pipelines.insert(
        "test".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![PhaseConfig::new("build", false)],
        },
    );

    let result = validate(&config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.contains("max_wip")));
}

#[test]
fn validate_max_concurrent_zero_fails() {
    let mut config = PhaseGolemConfig::default();
    config.execution.max_concurrent = 0;
    config.pipelines.insert(
        "test".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![PhaseConfig::new("build", false)],
        },
    );

    let result = validate(&config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.contains("max_concurrent")));
}

#[test]
fn validate_pipeline_no_main_phases_fails() {
    let mut config = PhaseGolemConfig::default();
    config.pipelines.insert(
        "empty".to_string(),
        PipelineConfig {
            pre_phases: vec![PhaseConfig::new("research", false)],
            phases: vec![],
        },
    );

    let result = validate(&config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.contains("at least one main phase")));
}

#[test]
fn validate_duplicate_phase_names_fails() {
    let mut config = PhaseGolemConfig::default();
    config.pipelines.insert(
        "dup".to_string(),
        PipelineConfig {
            pre_phases: vec![PhaseConfig::new("research", false)],
            phases: vec![
                PhaseConfig::new("research", false),
                PhaseConfig::new("build", false),
            ],
        },
    );

    let result = validate(&config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.contains("duplicate phase name")));
}

#[test]
fn validate_destructive_pre_phase_fails() {
    let mut config = PhaseGolemConfig::default();
    config.pipelines.insert(
        "bad".to_string(),
        PipelineConfig {
            pre_phases: vec![PhaseConfig::new("research", true)],
            phases: vec![PhaseConfig::new("build", false)],
        },
    );

    let result = validate(&config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.contains("cannot be destructive")));
}

#[test]
fn validate_staleness_block_with_max_wip_greater_than_one_fails() {
    let mut config = PhaseGolemConfig::default();
    config.execution.max_wip = 2;
    config.pipelines.insert(
        "risky".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![PhaseConfig {
                staleness: StalenessAction::Block,
                ..PhaseConfig::new("build", true)
            }],
        },
    );

    let result = validate(&config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors
        .iter()
        .any(|e| e.contains("staleness") && e.contains("max_wip")));
}

#[test]
fn validate_staleness_block_with_max_wip_one_passes() {
    let mut config = PhaseGolemConfig::default();
    config.execution.max_wip = 1;
    config.pipelines.insert(
        "ok".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![PhaseConfig {
                staleness: StalenessAction::Block,
                ..PhaseConfig::new("build", true)
            }],
        },
    );

    let result = validate(&config);
    assert!(result.is_ok());
}

#[test]
fn validate_multiple_errors_reported() {
    let mut config = PhaseGolemConfig::default();
    config.execution.max_wip = 0;
    config.execution.max_concurrent = 0;
    config.pipelines.insert(
        "bad".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![],
        },
    );

    let result = validate(&config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(
        errors.len() >= 3,
        "Expected at least 3 errors, got {}: {:?}",
        errors.len(),
        errors
    );
}

#[test]
fn load_config_validation_failure_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("phase-golem.toml");
    std::fs::write(
        &config_path,
        r#"
[execution]
max_wip = 0

[pipelines.bad]
phases = []
"#,
    )
    .unwrap();

    let result = load_config(dir.path());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("validation failed"),
        "Expected 'validation failed' in: {}",
        err
    );
}

// --- load_config_from tests ---

#[test]
fn load_config_from_none_delegates_to_load_config() {
    let dir = tempfile::tempdir().unwrap();
    let config = load_config_from(None, dir.path()).unwrap();

    assert_eq!(config.project.prefix, "WRK");
    assert_eq!(config.guardrails.max_size, SizeLevel::Medium);
    assert_eq!(config.guardrails.max_complexity, DimensionLevel::Medium);
    assert_eq!(config.guardrails.max_risk, DimensionLevel::Low);
    assert_eq!(config.execution.phase_timeout_minutes, 30);
    assert_eq!(config.execution.max_retries, 2);
    assert_eq!(config.execution.default_phase_cap, 100);
}

#[test]
fn load_config_from_explicit_path_that_exists() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("custom-config.toml");
    std::fs::write(
        &config_path,
        r#"
[project]
prefix = "CUSTOM"

[guardrails]
max_size = "large"
max_complexity = "high"
max_risk = "medium"

[execution]
phase_timeout_minutes = 45
max_retries = 3
default_phase_cap = 75
"#,
    )
    .unwrap();

    let config = load_config_from(Some(config_path.as_path()), dir.path()).unwrap();

    assert_eq!(config.project.prefix, "CUSTOM");
    assert_eq!(config.guardrails.max_size, SizeLevel::Large);
    assert_eq!(config.guardrails.max_complexity, DimensionLevel::High);
    assert_eq!(config.guardrails.max_risk, DimensionLevel::Medium);
    assert_eq!(config.execution.phase_timeout_minutes, 45);
    assert_eq!(config.execution.max_retries, 3);
    assert_eq!(config.execution.default_phase_cap, 75);
}

#[test]
fn load_config_from_explicit_path_missing() {
    let dir = tempfile::tempdir().unwrap();
    let missing_path = dir.path().join("does-not-exist.toml");

    let result = load_config_from(Some(missing_path.as_path()), dir.path());
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("Config file not found"),
        "Expected 'Config file not found' in: {}",
        err
    );
}

// --- CliTool tests ---

#[test]
fn cli_tool_default_is_claude() {
    assert_eq!(CliTool::default(), CliTool::Claude);
}

#[test]
fn cli_tool_binary_name() {
    assert_eq!(CliTool::Claude.binary_name(), "claude");
    assert_eq!(CliTool::OpenCode.binary_name(), "opencode");
}

#[test]
fn cli_tool_display_name() {
    assert_eq!(CliTool::Claude.display_name(), "Claude CLI");
    assert_eq!(CliTool::OpenCode.display_name(), "OpenCode CLI");
}

#[test]
fn cli_tool_build_args_claude_without_model() {
    let args = CliTool::Claude.build_args("do stuff", None);
    assert_eq!(
        args,
        vec!["--dangerously-skip-permissions", "-p", "do stuff"]
    );
}

#[test]
fn cli_tool_build_args_claude_with_model() {
    let args = CliTool::Claude.build_args("do stuff", Some("opus"));
    assert_eq!(
        args,
        vec![
            "--dangerously-skip-permissions",
            "--model",
            "opus",
            "-p",
            "do stuff"
        ]
    );
}

#[test]
fn cli_tool_build_args_opencode_without_model() {
    let args = CliTool::OpenCode.build_args("do stuff", None);
    assert_eq!(args, vec!["run", "--quiet", "do stuff"]);
}

#[test]
fn cli_tool_build_args_opencode_with_model() {
    let args = CliTool::OpenCode.build_args("do stuff", Some("gpt-4"));
    assert_eq!(args, vec!["run", "--model", "gpt-4", "--quiet", "do stuff"]);
}

#[test]
fn cli_tool_build_args_with_special_chars_in_prompt() {
    let prompt = "line1\nline2\n\"quoted\"\nspecial: $HOME & stuff; rm -rf /";
    let args = CliTool::Claude.build_args(prompt, None);
    assert_eq!(args[args.len() - 1], prompt);
    let args_oc = CliTool::OpenCode.build_args(prompt, None);
    assert_eq!(args_oc[args_oc.len() - 1], prompt);
}

#[test]
fn cli_tool_version_args() {
    assert_eq!(CliTool::Claude.version_args(), vec!["--version"]);
    assert_eq!(CliTool::OpenCode.version_args(), vec!["--version"]);
}

#[test]
fn cli_tool_install_hint_non_empty() {
    assert!(!CliTool::Claude.install_hint().is_empty());
    assert!(!CliTool::OpenCode.install_hint().is_empty());
}

#[test]
fn cli_tool_serde_claude() {
    let config: PhaseGolemConfig = toml::from_str(
        r#"
[agent]
cli = "claude"
"#,
    )
    .unwrap();
    assert_eq!(config.agent.cli, CliTool::Claude);
}

#[test]
fn cli_tool_serde_opencode() {
    let config: PhaseGolemConfig = toml::from_str(
        r#"
[agent]
cli = "opencode"
"#,
    )
    .unwrap();
    assert_eq!(config.agent.cli, CliTool::OpenCode);
}

#[test]
fn cli_tool_serde_invalid_value_rejected() {
    let result = toml::from_str::<PhaseGolemConfig>(
        r#"
[agent]
cli = "gpt"
"#,
    );
    assert!(result.is_err());
}

// --- AgentConfig tests ---

#[test]
fn agent_config_full_section_parses() {
    let config: PhaseGolemConfig = toml::from_str(
        r#"
[agent]
cli = "opencode"
model = "gpt-4"
"#,
    )
    .unwrap();
    assert_eq!(config.agent.cli, CliTool::OpenCode);
    assert_eq!(config.agent.model, Some("gpt-4".to_string()));
}

#[test]
fn agent_config_partial_only_model_defaults_cli() {
    let config: PhaseGolemConfig = toml::from_str(
        r#"
[agent]
model = "sonnet"
"#,
    )
    .unwrap();
    assert_eq!(config.agent.cli, CliTool::Claude);
    assert_eq!(config.agent.model, Some("sonnet".to_string()));
}

#[test]
fn agent_config_missing_section_defaults() {
    let config: PhaseGolemConfig = toml::from_str("").unwrap();
    assert_eq!(config.agent.cli, CliTool::Claude);
    assert_eq!(config.agent.model, None);
}

#[test]
fn agent_config_deny_unknown_fields_rejects_typo() {
    let result = toml::from_str::<PhaseGolemConfig>(
        r#"
[agent]
cli_tool = "claude"
"#,
    );
    assert!(result.is_err());
}

// --- Normalization tests ---

#[test]
fn normalize_empty_string_model_to_none() {
    let mut config = PhaseGolemConfig {
        agent: AgentConfig {
            cli: CliTool::Claude,
            model: Some("".to_string()),
        },
        ..PhaseGolemConfig::default()
    };
    normalize_agent_config(&mut config);
    assert_eq!(config.agent.model, None);
}

#[test]
fn normalize_whitespace_model_to_none() {
    let mut config = PhaseGolemConfig {
        agent: AgentConfig {
            cli: CliTool::Claude,
            model: Some("   ".to_string()),
        },
        ..PhaseGolemConfig::default()
    };
    normalize_agent_config(&mut config);
    assert_eq!(config.agent.model, None);
}

#[test]
fn normalize_tab_newline_model_to_none() {
    let mut config = PhaseGolemConfig {
        agent: AgentConfig {
            cli: CliTool::Claude,
            model: Some("\t\n".to_string()),
        },
        ..PhaseGolemConfig::default()
    };
    normalize_agent_config(&mut config);
    assert_eq!(config.agent.model, None);
}

#[test]
fn normalize_valid_model_preserved() {
    let mut config = PhaseGolemConfig {
        agent: AgentConfig {
            cli: CliTool::Claude,
            model: Some("opus".to_string()),
        },
        ..PhaseGolemConfig::default()
    };
    normalize_agent_config(&mut config);
    assert_eq!(config.agent.model, Some("opus".to_string()));
}

#[test]
fn normalize_none_model_stays_none() {
    let mut config = PhaseGolemConfig::default();
    normalize_agent_config(&mut config);
    assert_eq!(config.agent.model, None);
}

#[test]
fn normalize_via_load_config_at_whitespace_model() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("custom.toml");
    std::fs::write(
        &config_path,
        r#"
[agent]
model = "  "
"#,
    )
    .unwrap();

    let config = load_config_from(Some(config_path.as_path()), dir.path()).unwrap();
    assert_eq!(config.agent.model, None);
}

// --- load_config no-file agent defaults ---

#[test]
fn load_config_no_file_agent_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let config = load_config(dir.path()).unwrap();
    assert_eq!(
        config.agent,
        AgentConfig {
            cli: CliTool::Claude,
            model: None,
        }
    );
}

// --- Validation tests (model allowlist) ---

#[test]
fn validate_model_with_internal_hyphens_accepted() {
    let mut config = PhaseGolemConfig::default();
    config.agent.model = Some("claude-opus-4".to_string());
    config
        .pipelines
        .insert("t".to_string(), default_feature_pipeline());
    assert!(validate(&config).is_ok());
}

#[test]
fn validate_model_with_dots_accepted() {
    let mut config = PhaseGolemConfig::default();
    config.agent.model = Some("gpt-4.1".to_string());
    config
        .pipelines
        .insert("t".to_string(), default_feature_pipeline());
    assert!(validate(&config).is_ok());
}

#[test]
fn validate_model_with_slashes_accepted() {
    let mut config = PhaseGolemConfig::default();
    config.agent.model = Some("openai/gpt-4o".to_string());
    config
        .pipelines
        .insert("t".to_string(), default_feature_pipeline());
    assert!(validate(&config).is_ok());
}

#[test]
fn validate_model_starting_with_hyphen_rejected() {
    let mut config = PhaseGolemConfig::default();
    config.agent.model = Some("-badmodel".to_string());
    config
        .pipelines
        .insert("t".to_string(), default_feature_pipeline());
    let result = validate(&config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.contains("agent.model")));
}

#[test]
fn validate_model_starting_with_double_hyphen_rejected() {
    let mut config = PhaseGolemConfig::default();
    config.agent.model = Some("--flag".to_string());
    config
        .pipelines
        .insert("t".to_string(), default_feature_pipeline());
    let result = validate(&config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.contains("agent.model")));
}

#[test]
fn validate_model_with_spaces_rejected() {
    let mut config = PhaseGolemConfig::default();
    config.agent.model = Some("opus 4".to_string());
    config
        .pipelines
        .insert("t".to_string(), default_feature_pipeline());
    let result = validate(&config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.contains("agent.model")));
}

#[test]
fn validate_model_with_special_chars_rejected() {
    let mut config = PhaseGolemConfig::default();
    config.agent.model = Some("model;rm".to_string());
    config
        .pipelines
        .insert("t".to_string(), default_feature_pipeline());
    let result = validate(&config);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert!(errors.iter().any(|e| e.contains("agent.model")));
}

#[test]
fn validate_none_model_passes() {
    let mut config = PhaseGolemConfig::default();
    config.agent.model = None;
    config
        .pipelines
        .insert("t".to_string(), default_feature_pipeline());
    assert!(validate(&config).is_ok());
}

// --- PhaseConfig backward compat tests ---

#[test]
fn phase_config_destructive_alias_parses() {
    let config: PhaseGolemConfig = toml::from_str(
        r#"
[[pipelines.test.phases]]
name = "build"
destructive = true
"#,
    )
    .unwrap();
    assert!(config.pipelines["test"].phases[0].is_destructive);
}

#[test]
fn phase_config_deny_unknown_fields_rejects_unknown_key() {
    let result = toml::from_str::<PhaseGolemConfig>(
        r#"
[[pipelines.test.phases]]
name = "build"
is_destructive = false
unknown_key = "bad"
"#,
    );
    assert!(result.is_err());
}

#[test]
fn cli_tool_and_agent_config_accessible_from_crate() {
    // Confirms public export through the crate boundary
    let _tool: phase_golem::config::CliTool = CliTool::Claude;
    let _config: phase_golem::config::AgentConfig = AgentConfig::default();
}

// --- Config-to-runner integration test ---

#[test]
fn config_to_runner_opencode_with_model() {
    use phase_golem::agent::CliAgentRunner;

    let config: PhaseGolemConfig = toml::from_str(
        r#"
[agent]
cli = "opencode"
model = "gpt-4"
"#,
    )
    .unwrap();

    let runner = CliAgentRunner::new(config.agent.cli, config.agent.model);
    assert_eq!(runner.tool, CliTool::OpenCode);
    assert_eq!(runner.model, Some("gpt-4".to_string()));
}

// --- Init template round-trip test ---

#[test]
fn init_template_round_trip_parses() {
    // Replicate the handle_init template TOML (with a concrete prefix)
    let template = r#"[project]
prefix = "WRK"
# backlog_path = "BACKLOG.yaml"

[guardrails]
max_size = "medium"
max_complexity = "medium"
max_risk = "low"

[execution]
phase_timeout_minutes = 30
max_retries = 2
default_phase_cap = 100
max_wip = 1
max_concurrent = 1

[agent]
# cli = "claude"          # AI CLI tool: "claude", "opencode"
# model = ""              # Model override (e.g., "opus", "sonnet")

[pipelines.feature]
pre_phases = [
    { name = "research", workflows = [".claude/skills/changes/workflows/orchestration/research-scope.md"], is_destructive = false },
]
phases = [
    { name = "prd",           workflows = [".claude/skills/changes/workflows/0-prd/create-prd.md"],                     is_destructive = false },
    { name = "tech-research", workflows = [".claude/skills/changes/workflows/1-tech-research/tech-research.md"],       is_destructive = false },
    { name = "design",        workflows = [".claude/skills/changes/workflows/2-design/design.md"],                       is_destructive = false },
    { name = "spec",           workflows = [".claude/skills/changes/workflows/3-spec/create-spec.md"],                    is_destructive = false },
    { name = "build",          workflows = [".claude/skills/changes/workflows/4-build/implement-spec-autonomous.md"],   is_destructive = true },
    { name = "review",         workflows = [".claude/skills/changes/workflows/5-review/change-review.md"],               is_destructive = false },
]
"#;

    let config: PhaseGolemConfig =
        toml::from_str(template).expect("handle_init template should parse successfully");

    // Verify agent defaults (commented-out fields should not be set)
    assert_eq!(config.agent.cli, CliTool::Claude);
    assert_eq!(config.agent.model, None);

    // Verify the pipeline parsed
    assert!(config.pipelines.contains_key("feature"));
    assert_eq!(config.pipelines["feature"].phases.len(), 6);
    assert!(config.pipelines["feature"].phases[4].is_destructive);
}
