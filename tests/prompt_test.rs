use std::collections::HashMap;
use std::path::Path;

use phase_golem::config::{PhaseConfig, PipelineConfig};
use phase_golem::pg_item::{self, PgItem};
use phase_golem::prompt::{self, PromptParams};
use phase_golem::types::{
    DimensionLevel, ItemStatus, PhasePool, SizeLevel, StructuredDescription,
};

// --- Test helpers ---

fn default_prd_config() -> PhaseConfig {
    PhaseConfig {
        workflows: vec![".claude/skills/changes/workflows/0-prd/create-prd.md".to_string()],
        ..PhaseConfig::new("prd", false)
    }
}

fn make_item(id: &str, title: &str) -> PgItem {
    let mut pg = pg_item::new_from_parts(
        id.to_string(),
        title.to_string(),
        ItemStatus::InProgress,
        vec![],
        vec![],
    );
    pg_item::set_phase(&mut pg.0, Some("prd"));
    pg
}

fn make_item_with_assessments() -> PgItem {
    let mut pg = make_item("WRK-005", "Add dark mode");
    pg_item::set_size(&mut pg.0, Some(&SizeLevel::Medium));
    pg_item::set_complexity(&mut pg.0, Some(&DimensionLevel::Medium));
    pg_item::set_risk(&mut pg.0, Some(&DimensionLevel::Low));
    pg_item::set_impact(&mut pg.0, Some(&DimensionLevel::High));
    pg
}

fn default_pipelines() -> HashMap<String, PipelineConfig> {
    let mut map = HashMap::new();
    map.insert(
        "feature".to_string(),
        phase_golem::config::default_feature_pipeline(),
    );
    map
}

// --- build_prompt tests ---

#[test]
fn build_prompt_contains_correct_skill_command_for_each_phase() {
    let cases: Vec<(&str, PhaseConfig, &str)> = vec![
        (
            "prd",
            PhaseConfig {
                workflows: vec![".claude/skills/changes/workflows/0-prd/create-prd.md".to_string()],
                ..PhaseConfig::new("prd", false)
            },
            ".claude/skills/changes/workflows/0-prd/create-prd.md",
        ),
        (
            "tech-research",
            PhaseConfig {
                workflows: vec![
                    ".claude/skills/changes/workflows/1-tech-research/tech-research.md".to_string(),
                ],
                ..PhaseConfig::new("tech-research", false)
            },
            ".claude/skills/changes/workflows/1-tech-research/tech-research.md",
        ),
        (
            "design",
            PhaseConfig {
                workflows: vec![".claude/skills/changes/workflows/2-design/design.md".to_string()],
                ..PhaseConfig::new("design", false)
            },
            ".claude/skills/changes/workflows/2-design/design.md",
        ),
        (
            "spec",
            PhaseConfig {
                workflows: vec![
                    ".claude/skills/changes/workflows/3-spec/create-spec.md".to_string()
                ],
                ..PhaseConfig::new("spec", false)
            },
            ".claude/skills/changes/workflows/3-spec/create-spec.md",
        ),
        (
            "build",
            PhaseConfig {
                workflows: vec![
                    ".claude/skills/changes/workflows/orchestration/build-spec-phase.md"
                        .to_string(),
                ],
                ..PhaseConfig::new("build", false)
            },
            ".claude/skills/changes/workflows/orchestration/build-spec-phase.md",
        ),
        (
            "review",
            PhaseConfig {
                workflows: vec![
                    ".claude/skills/changes/workflows/5-review/change-review.md".to_string()
                ],
                ..PhaseConfig::new("review", false)
            },
            ".claude/skills/changes/workflows/5-review/change-review.md",
        ),
    ];

    for (phase_name, phase_config, expected_cmd) in &cases {
        let item = make_item("WRK-001", "Test feature");
        let result_path = Path::new(".phase-golem/result.json");
        let change_folder = Path::new("changes/WRK-001_test");

        let prompt_text = prompt::build_prompt(&PromptParams {
            phase: phase_name,
            phase_config,
            item: &item,
            result_path,
            change_folder,
            previous_summary: None,
            unblock_notes: None,
            failure_context: None,
            config_base: Path::new("."),
        });

        assert!(
            prompt_text.contains(expected_cmd),
            "Phase '{}' should contain '{}' but prompt was:\n{}",
            phase_name,
            expected_cmd,
            &prompt_text[prompt_text.len().saturating_sub(500)..],
        );
    }
}

#[test]
fn build_prompt_includes_result_file_path_in_suffix() {
    let item = make_item("WRK-001", "Test feature");
    let result_path = Path::new(".phase-golem/phase_result_WRK-001_prd.json");
    let change_folder = Path::new("changes/WRK-001_test-feature");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(prompt_text.contains(".phase-golem/phase_result_WRK-001_prd.json"));
}

#[test]
fn build_prompt_includes_previous_summary_when_provided() {
    let item = make_item("WRK-001", "Test feature");
    let result_path = Path::new(".phase-golem/phase_result_WRK-001_research.json");
    let change_folder = Path::new("changes/WRK-001_test-feature");
    let phase_config = PhaseConfig {
        workflows: vec![
            ".claude/skills/changes/workflows/1-tech-research/tech-research.md".to_string(),
        ],
        ..PhaseConfig::new("tech-research", false)
    };

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "tech-research",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: Some("PRD created with 5 success criteria and 3 user stories"),
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(prompt_text.contains("Previous Phase Summary"));
    assert!(prompt_text.contains("PRD created with 5 success criteria and 3 user stories"));
}

#[test]
fn build_prompt_excludes_previous_summary_when_none() {
    let item = make_item("WRK-001", "Test feature");
    let result_path = Path::new(".phase-golem/phase_result_WRK-001_prd.json");
    let change_folder = Path::new("changes/WRK-001_test-feature");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(!prompt_text.contains("Previous Phase Summary"));
}

#[test]
fn build_prompt_includes_unblock_notes_when_provided() {
    let item = make_item("WRK-001", "Test feature");
    let result_path = Path::new(".phase-golem/phase_result_WRK-001_design.json");
    let change_folder = Path::new("changes/WRK-001_test-feature");
    let phase_config = PhaseConfig {
        workflows: vec![".claude/skills/changes/workflows/2-design/design.md".to_string()],
        ..PhaseConfig::new("design", false)
    };

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "design",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: Some("Use PostgreSQL instead of SQLite for the database layer"),
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(prompt_text.contains("Unblock Context"));
    assert!(prompt_text.contains("Use PostgreSQL instead of SQLite for the database layer"));
}

#[test]
fn build_prompt_excludes_unblock_notes_when_none() {
    let item = make_item("WRK-001", "Test feature");
    let result_path = Path::new(".phase-golem/phase_result_WRK-001_prd.json");
    let change_folder = Path::new("changes/WRK-001_test-feature");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(!prompt_text.contains("Unblock Context"));
}

#[test]
fn build_prompt_includes_failure_context_when_retrying() {
    let item = make_item("WRK-001", "Test feature");
    let result_path = Path::new(".phase-golem/phase_result_WRK-001_prd.json");
    let change_folder = Path::new("changes/WRK-001_test-feature");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: Some("Agent timed out after 1800 seconds"),
        config_base: Path::new("."),
    });

    assert!(prompt_text.contains("Previous Failure"));
    assert!(prompt_text.contains("Agent timed out after 1800 seconds"));
    assert!(prompt_text.contains("try a different approach"));
}

#[test]
fn build_prompt_excludes_failure_context_when_none() {
    let item = make_item("WRK-001", "Test feature");
    let result_path = Path::new(".phase-golem/phase_result_WRK-001_prd.json");
    let change_folder = Path::new("changes/WRK-001_test-feature");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(!prompt_text.contains("Previous Failure"));
}

#[test]
fn build_prompt_includes_assumptions_instruction_in_preamble() {
    let item = make_item("WRK-001", "Test feature");
    let result_path = Path::new(".phase-golem/phase_result_WRK-001_prd.json");
    let change_folder = Path::new("changes/WRK-001_test-feature");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(prompt_text.contains("Assumptions"));
    assert!(prompt_text.contains("documenting decisions made without human input"));
}

#[test]
fn build_prompt_includes_assessments_when_present() {
    let item = make_item_with_assessments();
    let result_path = Path::new(".phase-golem/phase_result_WRK-005_prd.json");
    let change_folder = Path::new("changes/WRK-005_add-dark-mode");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(prompt_text.contains("Current Assessments"));
    assert!(prompt_text.contains("- **Size:** medium"));
    assert!(prompt_text.contains("- **Complexity:** medium"));
    assert!(prompt_text.contains("- **Risk:** low"));
    assert!(prompt_text.contains("- **Impact:** high"));
}

#[test]
fn build_prompt_includes_partial_assessments() {
    let mut item = make_item("WRK-007", "Partial assessments");
    pg_item::set_size(&mut item.0, Some(&SizeLevel::Small));
    pg_item::set_risk(&mut item.0, Some(&DimensionLevel::High));

    let result_path = Path::new(".phase-golem/result.json");
    let change_folder = Path::new("changes/WRK-007_partial");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(prompt_text.contains("- **Size:** small"));
    assert!(prompt_text.contains("- **Risk:** high"));
    assert!(!prompt_text.contains("**Complexity:**"));
    assert!(!prompt_text.contains("**Impact:**"));
}

#[test]
fn build_prompt_excludes_assessments_when_none() {
    let item = make_item("WRK-001", "Test feature");
    let result_path = Path::new(".phase-golem/phase_result_WRK-001_prd.json");
    let change_folder = Path::new("changes/WRK-001_test-feature");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(!prompt_text.contains("Current Assessments"));
}

#[test]
fn build_prompt_contains_json_schema_in_suffix() {
    let item = make_item("WRK-001", "Test feature");
    let result_path = Path::new(".phase-golem/phase_result_WRK-001_prd.json");
    let change_folder = Path::new("changes/WRK-001_test-feature");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(prompt_text.contains("\"item_id\""));
    assert!(prompt_text.contains("\"phase\""));
    assert!(prompt_text.contains("\"result\""));
    assert!(prompt_text.contains("\"summary\""));
    assert!(prompt_text.contains("\"follow_ups\""));
    assert!(prompt_text.contains("phase_complete"));
    assert!(prompt_text.contains("subphase_complete"));
    assert!(prompt_text.contains("failed"));
    assert!(prompt_text.contains("blocked"));
}

#[test]
fn build_prompt_item_id_embedded_in_schema() {
    let item = make_item("WRK-042", "Custom ID");
    let result_path = Path::new(".phase-golem/phase_result_WRK-042_prd.json");
    let change_folder = Path::new("changes/WRK-042_custom-id");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(prompt_text.contains("\"item_id\": \"WRK-042\""));
    assert!(prompt_text.contains("\"phase\": \"prd\""));
}

// --- build_triage_prompt tests ---

#[test]
fn triage_prompt_contains_assessment_instructions() {
    let item = make_item("WRK-010", "Fix login bug");
    let result_path = Path::new(".phase-golem/phase_result_WRK-010_triage.json");

    let prompt_text = prompt::build_triage_prompt(&item, result_path, &default_pipelines(), None);

    assert!(prompt_text.contains("Assess"));
    assert!(prompt_text.contains("**Size:**"));
    assert!(prompt_text.contains("**Complexity:**"));
    assert!(prompt_text.contains("**Risk:**"));
    assert!(prompt_text.contains("**Impact:**"));
    assert!(prompt_text.contains("small size AND low risk"));
}

#[test]
fn triage_prompt_contains_item_info() {
    let item = make_item("WRK-010", "Fix login bug");
    let result_path = Path::new(".phase-golem/phase_result_WRK-010_triage.json");

    let prompt_text = prompt::build_triage_prompt(&item, result_path, &default_pipelines(), None);

    assert!(prompt_text.contains("WRK-010"));
    assert!(prompt_text.contains("Fix login bug"));
}

#[test]
fn triage_prompt_contains_result_path() {
    let item = make_item("WRK-010", "Fix login bug");
    let result_path = Path::new(".phase-golem/phase_result_WRK-010_triage.json");

    let prompt_text = prompt::build_triage_prompt(&item, result_path, &default_pipelines(), None);

    assert!(prompt_text.contains(".phase-golem/phase_result_WRK-010_triage.json"));
}

#[test]
fn triage_prompt_contains_routing_instructions() {
    let item = make_item("WRK-010", "Fix login bug");
    let result_path = Path::new(".phase-golem/phase_result_WRK-010_triage.json");

    let prompt_text = prompt::build_triage_prompt(&item, result_path, &default_pipelines(), None);

    assert!(prompt_text.contains("promote directly"));
    assert!(prompt_text.contains("idea file"));
    assert!(prompt_text.contains("requires_human_review"));
}

#[test]
fn triage_prompt_uses_triage_phase_string() {
    let item = make_item("WRK-010", "Fix login bug");
    let result_path = Path::new(".phase-golem/phase_result_WRK-010_triage.json");

    let prompt_text = prompt::build_triage_prompt(&item, result_path, &default_pipelines(), None);

    assert!(prompt_text.contains("\"phase\": \"triage\""));
}

#[test]
fn triage_prompt_contains_description_instructions() {
    let item = make_item("WRK-010", "Fix login bug");
    let result_path = Path::new(".phase-golem/phase_result_WRK-010_triage.json");

    let prompt_text = prompt::build_triage_prompt(&item, result_path, &default_pipelines(), None);

    assert!(prompt_text.contains("structured description"));
    assert!(prompt_text.contains("`context`"));
    assert!(prompt_text.contains("`problem`"));
    assert!(prompt_text.contains("`solution`"));
    assert!(prompt_text.contains("`impact`"));
    assert!(prompt_text.contains("`sizing_rationale`"));
}

#[test]
fn triage_output_schema_contains_description_field() {
    let item = make_item("WRK-010", "Fix login bug");
    let result_path = Path::new(".phase-golem/phase_result_WRK-010_triage.json");

    let prompt_text = prompt::build_triage_prompt(&item, result_path, &default_pipelines(), None);

    assert!(prompt_text.contains("\"description\""));
    assert!(prompt_text.contains("\"context\""));
    assert!(prompt_text.contains("\"problem\""));
    assert!(prompt_text.contains("\"solution\""));
    assert!(prompt_text.contains("\"impact\""));
    assert!(prompt_text.contains("\"sizing_rationale\""));
}

#[test]
fn triage_prompt_uses_item_to_triage_heading() {
    let item = make_item("WRK-010", "Fix login bug");
    let result_path = Path::new(".phase-golem/phase_result_WRK-010_triage.json");

    let prompt_text = prompt::build_triage_prompt(&item, result_path, &default_pipelines(), None);

    assert!(prompt_text.contains("## Item to Triage"));
}

// --- All phases use correct phase string in JSON schema ---

#[test]
fn build_prompt_embeds_correct_phase_string_for_each_phase() {
    let item = make_item("WRK-001", "Test");
    let change_folder = Path::new("changes/WRK-001_test");

    let phases: Vec<(&str, &str)> = vec![
        ("prd", "prd"),
        ("tech-research", "tech-research"),
        ("design", "design"),
        ("spec", "spec"),
        ("build", "build"),
        ("review", "review"),
    ];

    for (phase_name, expected_str) in phases {
        let phase_config = PhaseConfig {
            workflows: vec!["some-skill".to_string()],
            ..PhaseConfig::new(phase_name, false)
        };
        let result_path_str = format!(".phase-golem/phase_result_WRK-001_{}.json", expected_str);
        let result_path = Path::new(&result_path_str);

        let prompt_text = prompt::build_prompt(&PromptParams {
            phase: phase_name,
            phase_config: &phase_config,
            item: &item,
            result_path,
            change_folder,
            previous_summary: None,
            unblock_notes: None,
            failure_context: None,
            config_base: Path::new("."),
        });

        let expected = format!("\"phase\": \"{}\"", expected_str);
        assert!(
            prompt_text.contains(&expected),
            "Phase '{}' should contain '{}' but prompt was:\n{}",
            phase_name,
            expected,
            &prompt_text[prompt_text.len().saturating_sub(500)..]
        );
    }
}

// --- Autonomous preamble structure ---

#[test]
fn build_prompt_contains_autonomous_preamble() {
    let item = make_item("WRK-001", "Test feature");
    let result_path = Path::new(".phase-golem/phase_result_WRK-001_prd.json");
    let change_folder = Path::new("changes/WRK-001_test-feature");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(prompt_text.contains("Autonomous Agent"));
    assert!(prompt_text.contains("running autonomously"));
    assert!(prompt_text.contains("No human is available"));
    assert!(prompt_text.contains("WRK-001"));
    assert!(prompt_text.contains("Test feature"));
}

// --- Combined optional sections ---

#[test]
fn build_prompt_with_all_optional_sections() {
    let item = make_item_with_assessments();
    let result_path = Path::new(".phase-golem/phase_result_WRK-005_design.json");
    let change_folder = Path::new("changes/WRK-005_add-dark-mode");
    let phase_config = PhaseConfig {
        workflows: vec![".claude/skills/changes/workflows/2-design/design.md".to_string()],
        ..PhaseConfig::new("design", false)
    };

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "design",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: Some("Research identified 3 viable approaches"),
        unblock_notes: Some("Go with approach B (CSS variables)"),
        failure_context: Some("Previous agent hit a dependency conflict"),
        config_base: Path::new("."),
    });

    // All sections present
    assert!(prompt_text.contains("Current Assessments"));
    assert!(prompt_text.contains("Previous Phase Summary"));
    assert!(prompt_text.contains("Research identified 3 viable approaches"));
    assert!(prompt_text.contains("Unblock Context"));
    assert!(prompt_text.contains("Go with approach B (CSS variables)"));
    assert!(prompt_text.contains("Previous Failure"));
    assert!(prompt_text.contains("Previous agent hit a dependency conflict"));
    // Workflow reference present
    assert!(prompt_text.contains(".claude/skills/changes/workflows/2-design/design.md"));
    // Structured output present
    assert!(prompt_text.contains("Structured Output"));
}

// --- Structured description in build_prompt ---

#[test]
fn build_prompt_includes_structured_description() {
    let mut item = make_item_with_assessments();
    pg_item::set_structured_description(&mut item.0, Some(&StructuredDescription {
        context: "Settings page exists".to_string(),
        problem: "No dark mode support".to_string(),
        solution: "Add toggle component".to_string(),
        impact: "Better night-time UX".to_string(),
        sizing_rationale: "Small — UI only".to_string(),
    }));
    let result_path = Path::new(".phase-golem/result.json");
    let change_folder = Path::new("changes/WRK-005_add-dark-mode");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(prompt_text.contains("## Description"));
    assert!(prompt_text.contains("**Context:** Settings page exists"));
    assert!(prompt_text.contains("**Problem:** No dark mode support"));
    assert!(prompt_text.contains("**Solution:** Add toggle component"));
    assert!(prompt_text.contains("**Impact:** Better night-time UX"));
    assert!(prompt_text.contains("**Sizing Rationale:** Small — UI only"));
}

#[test]
fn build_prompt_skips_empty_description_fields() {
    let mut item = make_item("WRK-001", "Test");
    pg_item::set_structured_description(&mut item.0, Some(&StructuredDescription {
        context: "Some context".to_string(),
        problem: String::new(),
        solution: "A solution".to_string(),
        impact: String::new(),
        sizing_rationale: String::new(),
    }));
    let result_path = Path::new(".phase-golem/result.json");
    let change_folder = Path::new("changes/WRK-001_test");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(prompt_text.contains("**Context:** Some context"));
    assert!(prompt_text.contains("**Solution:** A solution"));
    assert!(!prompt_text.contains("**Problem:**"));
    assert!(!prompt_text.contains("**Impact:**"));
    assert!(!prompt_text.contains("**Sizing Rationale:**"));
}

#[test]
fn build_prompt_omits_description_section_when_all_empty() {
    let mut item = make_item("WRK-001", "Test");
    pg_item::set_structured_description(&mut item.0, Some(&StructuredDescription {
        context: String::new(),
        problem: String::new(),
        solution: String::new(),
        impact: String::new(),
        sizing_rationale: String::new(),
    }));
    let result_path = Path::new(".phase-golem/result.json");
    let change_folder = Path::new("changes/WRK-001_test");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(!prompt_text.contains("## Description"));
}

#[test]
fn build_prompt_excludes_description_when_none() {
    let item = make_item("WRK-001", "Test");
    assert_eq!(item.structured_description(), None);
    let result_path = Path::new(".phase-golem/result.json");
    let change_folder = Path::new("changes/WRK-001_test");
    let phase_config = default_prd_config();

    let prompt_text = prompt::build_prompt(&PromptParams {
        phase: "prd",
        phase_config: &phase_config,
        item: &item,
        result_path,
        change_folder,
        previous_summary: None,
        unblock_notes: None,
        failure_context: None,
        config_base: Path::new("."),
    });

    assert!(!prompt_text.contains("## Description"));
}

// --- build_context_preamble tests ---

#[test]
fn context_preamble_contains_mode_and_item() {
    let mut item = make_item("WRK-003", "Orchestrator v2");
    pg_item::set_pipeline_type(&mut item.0, Some("feature"));
    pg_item::set_phase(&mut item.0, Some("build"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));

    let pipeline = phase_golem::config::default_feature_pipeline();
    let preamble = prompt::build_context_preamble(&item, &pipeline, None, None, None);

    assert!(preamble.contains("**Mode:** autonomous"));
    assert!(preamble.contains("WRK-003"));
    assert!(preamble.contains("Orchestrator v2"));
    assert!(preamble.contains("**Pipeline:** feature"));
}

#[test]
fn context_preamble_shows_phase_position() {
    let mut item = make_item("WRK-001", "Test");
    pg_item::set_pipeline_type(&mut item.0, Some("feature"));
    pg_item::set_phase(&mut item.0, Some("build"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));

    let pipeline = phase_golem::config::default_feature_pipeline();
    let preamble = prompt::build_context_preamble(&item, &pipeline, None, None, None);

    // build is 5th of 6 main phases
    assert!(preamble.contains("build (5/6, main)"));
}

#[test]
fn context_preamble_shows_pre_phase_position() {
    let mut item = make_item("WRK-001", "Test");
    pg_item::set_pipeline_type(&mut item.0, Some("feature"));
    pg_item::set_phase(&mut item.0, Some("research"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Pre));

    let pipeline = phase_golem::config::default_feature_pipeline();
    let preamble = prompt::build_context_preamble(&item, &pipeline, None, None, None);

    // research is 1st of 1 pre_phases
    assert!(preamble.contains("research (1/1, pre)"));
}

#[test]
fn context_preamble_includes_description() {
    let mut item = make_item("WRK-001", "Test");
    pg_item::set_pipeline_type(&mut item.0, Some("feature"));
    pg_item::set_phase(&mut item.0, Some("prd"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));
    pg_item::set_structured_description(&mut item.0, Some(&StructuredDescription {
        context: "Settings page needs theme support".to_string(),
        problem: "No dark mode available".to_string(),
        solution: "Add dark mode toggle to settings".to_string(),
        impact: "Better UX for night users".to_string(),
        sizing_rationale: String::new(),
    }));

    let pipeline = phase_golem::config::default_feature_pipeline();
    let preamble = prompt::build_context_preamble(&item, &pipeline, None, None, None);

    assert!(preamble.contains("### Description"));
    assert!(preamble.contains("**Context:** Settings page needs theme support"));
    assert!(preamble.contains("**Problem:** No dark mode available"));
    assert!(preamble.contains("**Solution:** Add dark mode toggle to settings"));
    assert!(preamble.contains("**Impact:** Better UX for night users"));
    // Empty sizing_rationale should be omitted
    assert!(!preamble.contains("**Sizing Rationale:**"));
}

#[test]
fn context_preamble_includes_previous_summary() {
    let mut item = make_item("WRK-001", "Test");
    pg_item::set_phase(&mut item.0, Some("design"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));

    let pipeline = phase_golem::config::default_feature_pipeline();
    let preamble = prompt::build_context_preamble(
        &item,
        &pipeline,
        Some("Research identified 3 approaches"),
        None,
        None,
    );

    assert!(preamble.contains("### Previous Phase Summary"));
    assert!(preamble.contains("Research identified 3 approaches"));
}

#[test]
fn context_preamble_includes_failure_context() {
    let mut item = make_item("WRK-001", "Test");
    pg_item::set_phase(&mut item.0, Some("build"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));

    let pipeline = phase_golem::config::default_feature_pipeline();
    let preamble = prompt::build_context_preamble(
        &item,
        &pipeline,
        None,
        None,
        Some("Test suite failed: 3 errors"),
    );

    assert!(preamble.contains("### Retry Context"));
    assert!(preamble.contains("Test suite failed: 3 errors"));
}

#[test]
fn context_preamble_includes_unblock_notes() {
    let mut item = make_item("WRK-001", "Test");
    pg_item::set_phase(&mut item.0, Some("build"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));

    let pipeline = phase_golem::config::default_feature_pipeline();
    let preamble = prompt::build_context_preamble(
        &item,
        &pipeline,
        None,
        Some("Use approach B instead"),
        None,
    );

    assert!(preamble.contains("### Unblock Context"));
    assert!(preamble.contains("Use approach B instead"));
}

#[test]
fn context_preamble_omits_empty_optional_sections() {
    let mut item = make_item("WRK-001", "Test");
    pg_item::set_phase(&mut item.0, Some("prd"));
    pg_item::set_phase_pool(&mut item.0, Some(&PhasePool::Main));

    let pipeline = phase_golem::config::default_feature_pipeline();
    let preamble = prompt::build_context_preamble(&item, &pipeline, None, None, None);

    assert!(!preamble.contains("### Previous Phase Summary"));
    assert!(!preamble.contains("### Retry Context"));
    assert!(!preamble.contains("### Unblock Context"));
    assert!(!preamble.contains("### Description"));
}

// --- build_triage_prompt pipeline type tests ---

#[test]
fn triage_prompt_includes_available_pipeline_types() {
    let item = make_item("WRK-010", "Fix login bug");
    let result_path = Path::new(".phase-golem/phase_result_WRK-010_triage.json");
    let pipelines = default_pipelines();

    let prompt_text = prompt::build_triage_prompt(&item, result_path, &pipelines, None);

    assert!(prompt_text.contains("Available Pipeline Types"));
    assert!(prompt_text.contains("`feature`"));
    assert!(prompt_text.contains("pipeline_type"));
}

#[test]
fn triage_prompt_with_multiple_pipelines_lists_all() {
    let item = make_item("WRK-010", "Write blog post");
    let result_path = Path::new(".phase-golem/phase_result_WRK-010_triage.json");
    let mut pipelines = default_pipelines();
    pipelines.insert(
        "blog-post".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![PhaseConfig {
                workflows: vec!["writing/draft".to_string()],
                ..PhaseConfig::new("draft", false)
            }],
        },
    );

    let prompt_text = prompt::build_triage_prompt(&item, result_path, &pipelines, None);

    assert!(prompt_text.contains("`feature`"));
    assert!(prompt_text.contains("`blog-post`"));
}

// --- build_backlog_summary tests ---

#[test]
fn backlog_summary_excludes_current_item() {
    let items = vec![
        make_item("WRK-001", "First item"),
        make_item("WRK-002", "Second item"),
        make_item("WRK-003", "Third item"),
    ];

    let summary = prompt::build_backlog_summary(&items, "WRK-002").unwrap();

    assert!(summary.contains("WRK-001"));
    assert!(!summary.contains("WRK-002"));
    assert!(summary.contains("WRK-003"));
}

#[test]
fn backlog_summary_empty_when_only_current_item() {
    let items = vec![make_item("WRK-001", "Only item")];

    let summary = prompt::build_backlog_summary(&items, "WRK-001");
    assert!(summary.is_none());
}

#[test]
fn backlog_summary_empty_for_empty_backlog() {
    let summary = prompt::build_backlog_summary(&[], "WRK-001");
    assert!(summary.is_none());
}

#[test]
fn backlog_summary_includes_status() {
    let item = pg_item::new_from_parts(
        "WRK-001".to_string(),
        "Blocked item".to_string(),
        ItemStatus::Blocked,
        vec![],
        vec![],
    );

    let summary = prompt::build_backlog_summary(&[item], "WRK-999").unwrap();
    assert!(summary.contains("[blocked]"));
}

// --- triage prompt with backlog summary tests ---

#[test]
fn triage_prompt_includes_backlog_section_when_provided() {
    let item = make_item("WRK-010", "Fix login bug");
    let result_path = Path::new(".phase-golem/phase_result_WRK-010_triage.json");
    let pipelines = default_pipelines();
    let summary = "- WRK-001: Add auth [inprogress]\n- WRK-005: Refactor DB [new]";

    let prompt_text = prompt::build_triage_prompt(&item, result_path, &pipelines, Some(summary));

    assert!(prompt_text.contains("## Current Backlog"));
    assert!(prompt_text.contains("WRK-001: Add auth"));
    assert!(prompt_text.contains("WRK-005: Refactor DB"));
    assert!(prompt_text.contains("duplicates"));
}

#[test]
fn triage_prompt_omits_backlog_section_when_none() {
    let item = make_item("WRK-010", "Fix login bug");
    let result_path = Path::new(".phase-golem/phase_result_WRK-010_triage.json");
    let pipelines = default_pipelines();

    let prompt_text = prompt::build_triage_prompt(&item, result_path, &pipelines, None);

    assert!(!prompt_text.contains("## Current Backlog"));
}
