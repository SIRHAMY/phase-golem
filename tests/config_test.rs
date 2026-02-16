use phase_golem::config::*;
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
