mod common;

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, Barrier};

use phase_golem::config::{
    load_config_from, PhaseConfig, PipelineConfig, PublicExecutionPolicy, PublicTemplateInput,
    PublicTemplateInputs, TemplateProvenance, VerificationPlan, WorkflowNode, WorkflowTemplate,
};
use phase_golem::materialization::{
    materialize_configured_run, materialize_run, materialize_run_with_id, X_PG_EXECUTION_POLICY,
    X_PG_EXECUTOR_PROFILE, X_PG_HUMAN_DECISION, X_PG_OWNER, X_PG_RUN_ID, X_PG_TEMPLATE_ID,
    X_PG_TEMPLATE_NODE_KEY, X_PG_TEMPLATE_PROVENANCE, X_PG_VERIFICATION,
};
use task_golem::model::status::Status;
use task_golem::{GraphApplyItem, GraphApplyRequest};

fn template() -> WorkflowTemplate {
    WorkflowTemplate {
        id: "release-workflow".to_string(),
        provenance: TemplateProvenance {
            source: "tests/release-workflow.toml".to_string(),
            revision: Some("v1".to_string()),
        },
        inputs: vec![PublicTemplateInput {
            name: "change".to_string(),
            default: Some("materialization".to_string()),
        }],
        nodes: vec![
            WorkflowNode {
                key: "build".to_string(),
                title: "Build ${change}".to_string(),
                description: Some("Compile ${change}".to_string()),
                priority: 5,
                tags: vec!["release".to_string()],
                parent: None,
                dependencies: vec![],
                human_decision: false,
                executor_profile: "agent".to_string(),
                execution_policy: PublicExecutionPolicy {
                    timeout_minutes: 15,
                    max_retries: 2,
                    destructive: false,
                    workflows: vec!["build.md".to_string()],
                },
                verification: VerificationPlan {
                    required_checks: vec!["cargo test".to_string()],
                },
            },
            WorkflowNode {
                key: "approve".to_string(),
                title: "Approve ${change}".to_string(),
                description: None,
                priority: 1,
                tags: vec!["release".to_string(), "gate".to_string()],
                parent: Some("build".to_string()),
                dependencies: vec!["build".to_string()],
                human_decision: true,
                executor_profile: "human".to_string(),
                execution_policy: PublicExecutionPolicy::default(),
                verification: VerificationPlan {
                    required_checks: vec!["approval record".to_string()],
                },
            },
        ],
    }
}

#[test]
fn default_and_custom_templates_materialize_generic_graphs() {
    // Arrange
    let directory = tempfile::tempdir().expect("create tempdir");
    let store = common::setup_task_golem_store(directory.path());
    let config = phase_golem::config::PhaseGolemConfig::default();
    let custom_template = template();

    // Act
    let default_run = materialize_configured_run(&store, &config, &PublicTemplateInputs::default())
        .expect("materialize default template");
    let custom_run = materialize_run(&store, &custom_template, &PublicTemplateInputs::default())
        .expect("materialize custom template");

    // Assert
    assert_eq!(default_run.node_mapping.len(), 7);
    assert_eq!(custom_run.node_mapping.len(), 2);
    let items = store.load_active().expect("load materialized graph");
    let build = items
        .iter()
        .find(|item| item.id == custom_run.node_mapping["build"])
        .expect("find build node");
    let approval = items
        .iter()
        .find(|item| item.id == custom_run.node_mapping["approve"])
        .expect("find approval node");

    assert!(custom_run
        .node_mapping
        .values()
        .all(|item_id| task_golem::validate_id(item_id).is_ok()));
    assert_eq!(build.title, "Build materialization");
    assert_eq!(approval.parent.as_deref(), Some(build.id.as_str()));
    assert_eq!(approval.dependencies, vec![build.id.clone()]);
    assert_eq!(build.status, Status::Todo);
    assert!(build.claimed_by.is_none());
    assert!(build.claimed_at.is_none());
    assert!(build.blocked_reason.is_none());
    assert!(build.blocked_from_status.is_none());
    assert_eq!(build.extensions[X_PG_RUN_ID], custom_run.run_id);
    assert_eq!(build.extensions[X_PG_TEMPLATE_ID], "release-workflow");
    assert_eq!(build.extensions[X_PG_TEMPLATE_NODE_KEY], "build");
    assert_eq!(build.extensions[X_PG_OWNER], "phase-golem");
    assert_eq!(approval.extensions[X_PG_HUMAN_DECISION], true);
    assert_eq!(build.extensions[X_PG_EXECUTOR_PROFILE], "agent");
    assert_eq!(
        build.extensions[X_PG_TEMPLATE_PROVENANCE]["source"],
        "tests/release-workflow.toml"
    );
    assert_eq!(
        build.extensions[X_PG_EXECUTION_POLICY]["timeout_minutes"],
        15
    );
    assert_eq!(
        build.extensions[X_PG_VERIFICATION]["required_checks"][0],
        "cargo test"
    );
    let expected_metadata_keys = BTreeSet::from([
        X_PG_EXECUTION_POLICY,
        X_PG_EXECUTOR_PROFILE,
        X_PG_HUMAN_DECISION,
        X_PG_OWNER,
        X_PG_RUN_ID,
        X_PG_TEMPLATE_ID,
        X_PG_TEMPLATE_NODE_KEY,
        X_PG_TEMPLATE_PROVENANCE,
        X_PG_VERIFICATION,
    ]);
    let actual_metadata_keys: BTreeSet<_> = build.extensions.keys().map(String::as_str).collect();
    assert_eq!(actual_metadata_keys, expected_metadata_keys);
}

#[test]
fn invalid_templates_leave_the_store_unchanged() {
    // Arrange
    let mut duplicate_key = template();
    duplicate_key.nodes[1].key = "build".to_string();

    let mut malformed = template();
    malformed.nodes[0].title = "\n".to_string();

    let mut unresolved = template();
    unresolved.nodes[1].dependencies = vec!["missing".to_string()];

    let mut self_reference = template();
    self_reference.nodes[0].parent = Some("build".to_string());

    let mut cycle = template();
    cycle.nodes[0].dependencies = vec!["approve".to_string()];

    let mut missing_input = template();
    missing_input.inputs[0].default = None;

    let mut unresolved_input = template();
    unresolved_input.nodes[1].title = "Approve ${missing}".to_string();

    let cases = vec![
        (
            "empty",
            WorkflowTemplate {
                nodes: vec![],
                ..template()
            },
        ),
        ("duplicate key", duplicate_key),
        ("malformed", malformed),
        ("unresolved", unresolved),
        ("self reference", self_reference),
        ("cycle", cycle),
        ("missing input", missing_input),
        ("unresolved input", unresolved_input),
    ];

    for (name, invalid_template) in cases {
        let directory = tempfile::tempdir().expect("create tempdir");
        let store = common::setup_task_golem_store(directory.path());

        // Act
        let result = materialize_run(&store, &invalid_template, &PublicTemplateInputs::default());

        // Assert
        assert!(result.is_err(), "{name} template should fail");
        assert!(
            store.load_active().expect("load active store").is_empty(),
            "{name} template should not write items"
        );
    }
}

#[test]
fn template_edits_only_affect_later_run_snapshots() {
    // Arrange
    let directory = tempfile::tempdir().expect("create tempdir");
    let store = common::setup_task_golem_store(directory.path());
    let initial_template = template();

    // Act
    let initial_run = materialize_run(&store, &initial_template, &PublicTemplateInputs::default())
        .expect("materialize initial run");
    let mut edited_template = initial_template.clone();
    edited_template.provenance.revision = Some("v2".to_string());
    edited_template.nodes[0].execution_policy.timeout_minutes = 60;
    let edited_run = materialize_run(&store, &edited_template, &PublicTemplateInputs::default())
        .expect("materialize edited run");

    // Assert
    let items = store.load_active().expect("load materialized runs");
    let initial_build = items
        .iter()
        .find(|item| item.id == initial_run.node_mapping["build"])
        .expect("find initial build");
    let edited_build = items
        .iter()
        .find(|item| item.id == edited_run.node_mapping["build"])
        .expect("find edited build");
    assert_eq!(
        initial_build.extensions[X_PG_EXECUTION_POLICY]["timeout_minutes"],
        15
    );
    assert_eq!(
        edited_build.extensions[X_PG_EXECUTION_POLICY]["timeout_minutes"],
        60
    );
    assert_eq!(
        initial_build.extensions[X_PG_TEMPLATE_PROVENANCE]["revision"],
        "v1"
    );
    assert_eq!(
        edited_build.extensions[X_PG_TEMPLATE_PROVENANCE]["revision"],
        "v2"
    );
}

#[test]
fn repeated_template_keys_materialize_to_distinct_runs_and_uuids() {
    // Arrange
    let directory = tempfile::tempdir().expect("create tempdir");
    let store = common::setup_task_golem_store(directory.path());
    let workflow_template = template();

    // Act
    let first_run = materialize_run(&store, &workflow_template, &PublicTemplateInputs::default())
        .expect("materialize first run");
    let second_run = materialize_run(&store, &workflow_template, &PublicTemplateInputs::default())
        .expect("materialize second run");

    // Assert
    assert_ne!(first_run.run_id, second_run.run_id);
    let first_ids: HashSet<_> = first_run.node_mapping.values().collect();
    let second_ids: HashSet<_> = second_run.node_mapping.values().collect();
    assert!(first_ids.is_disjoint(&second_ids));
    let items = store.load_active().expect("load both runs");
    let build_runs: HashSet<_> = items
        .iter()
        .filter(|item| item.extensions[X_PG_TEMPLATE_NODE_KEY] == "build")
        .map(|item| item.extensions[X_PG_RUN_ID].as_str().expect("run id"))
        .collect();
    assert_eq!(build_runs.len(), 2);
}

#[test]
fn uncertain_committed_run_is_reconstructed_without_duplicate_application() {
    // Arrange
    let directory = tempfile::tempdir().expect("create tempdir");
    let store = common::setup_task_golem_store(directory.path());
    let workflow_template = template();
    let run_id = "run-with-lost-response";
    let committed = materialize_run_with_id(
        &store,
        &workflow_template,
        &PublicTemplateInputs::default(),
        run_id,
    )
    .expect("commit run before response is lost");

    // Act: retry the explicit run after discarding its successful response.
    let recovered = materialize_run_with_id(
        &store,
        &workflow_template,
        &PublicTemplateInputs::default(),
        run_id,
    )
    .expect("recover committed run");

    // Assert
    assert_eq!(recovered, committed);
    assert_eq!(
        store.load_active().expect("load recovered run").len(),
        workflow_template.nodes.len(),
        "recovery must not apply a second graph"
    );
}

#[test]
fn fully_archived_run_is_reconstructed_without_duplicate_application() {
    // Arrange
    let directory = tempfile::tempdir().expect("create tempdir");
    let store = common::setup_task_golem_store(directory.path());
    let workflow_template = template();
    let run_id = "fully-archived-run";
    let committed = materialize_run_with_id(
        &store,
        &workflow_template,
        &PublicTemplateInputs::default(),
        run_id,
    )
    .expect("materialize run before archival");
    for item_id in committed.node_mapping.values() {
        archive_item(&store, item_id);
    }

    // Act
    let recovered = materialize_run_with_id(
        &store,
        &workflow_template,
        &PublicTemplateInputs::default(),
        run_id,
    )
    .expect("recover fully archived run");

    // Assert
    assert_eq!(recovered, committed);
    assert!(store.load_active().expect("load active items").is_empty());
    assert_eq!(
        store.load_all_archive().expect("load archived run").len(),
        workflow_template.nodes.len(),
        "archived recovery must not apply a second graph"
    );
}

#[test]
fn mixed_active_and_archived_run_is_reconstructed_without_duplicate_application() {
    // Arrange
    let directory = tempfile::tempdir().expect("create tempdir");
    let store = common::setup_task_golem_store(directory.path());
    let workflow_template = template();
    let run_id = "mixed-archive-run";
    let committed = materialize_run_with_id(
        &store,
        &workflow_template,
        &PublicTemplateInputs::default(),
        run_id,
    )
    .expect("materialize run before partial archival");
    archive_item(&store, &committed.node_mapping["build"]);

    // Act
    let recovered = materialize_run_with_id(
        &store,
        &workflow_template,
        &PublicTemplateInputs::default(),
        run_id,
    )
    .expect("recover mixed active and archived run");

    // Assert
    assert_eq!(recovered, committed);
    let active_count = store.load_active().expect("load active run").len();
    let archive_count = store.load_all_archive().expect("load archived run").len();
    assert_eq!(active_count, 1);
    assert_eq!(archive_count, 1);
    assert_eq!(
        active_count + archive_count,
        workflow_template.nodes.len(),
        "mixed recovery must not apply a second graph"
    );
}

#[test]
fn absent_explicit_run_is_applied_once() {
    // Arrange
    let directory = tempfile::tempdir().expect("create tempdir");
    let store = common::setup_task_golem_store(directory.path());
    let workflow_template = template();

    // Act
    let materialized = materialize_run_with_id(
        &store,
        &workflow_template,
        &PublicTemplateInputs::default(),
        "absent-run",
    )
    .expect("materialize absent run");

    // Assert
    assert_eq!(
        materialized.node_mapping.len(),
        workflow_template.nodes.len()
    );
    assert_eq!(
        store.load_active().expect("load materialized run").len(),
        workflow_template.nodes.len()
    );
}

#[test]
fn partial_and_duplicate_discovered_runs_fail_deterministically() {
    let cases = [
        (
            "partial-run",
            vec![("build", "seed build")],
            "materialization recovery for run 'partial-run' found inconsistent state: missing node keys [approve]",
        ),
        (
            "duplicate-run",
            vec![("build", "seed build one"), ("build", "seed build two")],
            "materialization recovery for run 'duplicate-run' found inconsistent state: duplicate node key 'build'; missing node keys [approve]",
        ),
    ];

    for (run_id, discovered_nodes, expected_error) in cases {
        // Arrange
        let directory = tempfile::tempdir().expect("create tempdir");
        let store = common::setup_task_golem_store(directory.path());
        seed_discovered_run(&store, run_id, &discovered_nodes);
        let item_count_before = store.load_active().expect("load seeded run").len();

        // Act
        let result = materialize_run_with_id(
            &store,
            &template(),
            &PublicTemplateInputs::default(),
            run_id,
        );

        // Assert
        assert_eq!(
            result.expect_err("inconsistent run must fail").to_string(),
            expected_error
        );
        assert_eq!(
            store.load_active().expect("load failed recovery").len(),
            item_count_before,
            "inconsistent recovery must not apply a graph"
        );
    }
}

#[test]
fn concurrent_requests_for_one_run_share_one_materialization_decision() {
    // Arrange
    let directory = tempfile::tempdir().expect("create tempdir");
    let store = common::setup_task_golem_store(directory.path());
    let workflow_template = template();
    let barrier = Arc::new(Barrier::new(3));

    // Act
    let results = std::thread::scope(|scope| {
        let handles = (0..2)
            .map(|_| {
                let store = store.clone();
                let workflow_template = workflow_template.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    materialize_run_with_id(
                        &store,
                        &workflow_template,
                        &PublicTemplateInputs::default(),
                        "shared-run",
                    )
                    .expect("materialize shared run")
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("materialization thread"))
            .collect::<Vec<_>>()
    });

    // Assert
    assert_eq!(results[0], results[1]);
    assert_eq!(
        store.load_active().expect("load shared run").len(),
        workflow_template.nodes.len(),
        "same-run concurrency must create only one graph"
    );
}

#[test]
fn separate_explicit_runs_coexist_with_distinct_tg_ids() {
    // Arrange
    let directory = tempfile::tempdir().expect("create tempdir");
    let store = common::setup_task_golem_store(directory.path());
    let workflow_template = template();
    let barrier = Arc::new(Barrier::new(3));

    // Act
    let runs = std::thread::scope(|scope| {
        let handles = ["explicit-run-one", "explicit-run-two"].map(|run_id| {
            let store = store.clone();
            let workflow_template = workflow_template.clone();
            let barrier = barrier.clone();
            scope.spawn(move || {
                barrier.wait();
                materialize_run_with_id(
                    &store,
                    &workflow_template,
                    &PublicTemplateInputs::default(),
                    run_id,
                )
                .expect("materialize separate explicit run")
            })
        });
        barrier.wait();
        handles.map(|handle| handle.join().expect("materialization thread"))
    });

    // Assert
    let first_ids = runs[0].node_mapping.values().collect::<HashSet<_>>();
    let second_ids = runs[1].node_mapping.values().collect::<HashSet<_>>();
    assert!(first_ids.is_disjoint(&second_ids));
    assert_eq!(
        store.load_active().expect("load coexisting runs").len(),
        workflow_template.nodes.len() * 2
    );
}

fn seed_discovered_run(store: &task_golem::store::Store, run_id: &str, nodes: &[(&str, &str)]) {
    let request = GraphApplyRequest {
        items: nodes
            .iter()
            .enumerate()
            .map(|(index, (node_key, title))| GraphApplyItem {
                reference: format!("seed-{index}"),
                title: (*title).to_string(),
                description: None,
                priority: 0,
                tags: vec![],
                parent: None,
                dependencies: vec![],
                extensions: BTreeMap::from([
                    (X_PG_RUN_ID.to_string(), serde_json::json!(run_id)),
                    (
                        X_PG_TEMPLATE_ID.to_string(),
                        serde_json::json!("release-workflow"),
                    ),
                    (
                        X_PG_TEMPLATE_NODE_KEY.to_string(),
                        serde_json::json!(node_key),
                    ),
                ]),
            })
            .collect(),
    };
    store.apply_graph(request).expect("seed discovered run");
}

fn archive_item(store: &task_golem::store::Store, item_id: &str) {
    store
        .with_lock(|store| {
            let mut items = store.load_active()?;
            let index = items
                .iter()
                .position(|item| item.id == item_id)
                .ok_or_else(|| task_golem::errors::TgError::ItemNotFound(item_id.to_string()))?;
            let change = items[index].apply_done();
            let done_item = items.remove(index);
            store.commit_done(&items, &done_item, change)
        })
        .expect("archive materialized item");
}

#[test]
fn configured_default_edits_only_affect_later_run_snapshots() {
    // Arrange
    let directory = tempfile::tempdir().expect("create tempdir");
    let store = common::setup_task_golem_store(directory.path());
    let mut config = phase_golem::config::PhaseGolemConfig::default();
    config.execution.phase_timeout_minutes = 10;
    config.execution.max_retries = 1;
    config.pipelines.insert(
        "feature".to_string(),
        PipelineConfig {
            pre_phases: vec![],
            phases: vec![PhaseConfig {
                workflows: vec!["workflows/initial.md".to_string()],
                ..PhaseConfig::new("build", false)
            }],
        },
    );

    // Act
    let initial_run = materialize_configured_run(&store, &config, &PublicTemplateInputs::default())
        .expect("materialize initial configured default");
    config.execution.phase_timeout_minutes = 45;
    config.execution.max_retries = 4;
    config
        .pipelines
        .get_mut("feature")
        .expect("feature pipeline")
        .phases[0]
        .workflows = vec!["workflows/edited.md".to_string()];
    let edited_run = materialize_configured_run(&store, &config, &PublicTemplateInputs::default())
        .expect("materialize edited configured default");

    // Assert
    let items = store.load_active().expect("load configured default runs");
    let initial_build = items
        .iter()
        .find(|item| item.id == initial_run.node_mapping["build"])
        .expect("find initial configured build");
    let edited_build = items
        .iter()
        .find(|item| item.id == edited_run.node_mapping["build"])
        .expect("find edited configured build");
    assert_eq!(
        initial_build.extensions[X_PG_EXECUTION_POLICY],
        serde_json::json!({
            "timeout_minutes": 10,
            "max_retries": 1,
            "destructive": false,
            "workflows": ["workflows/initial.md"]
        })
    );
    assert_eq!(
        edited_build.extensions[X_PG_EXECUTION_POLICY],
        serde_json::json!({
            "timeout_minutes": 45,
            "max_retries": 4,
            "destructive": false,
            "workflows": ["workflows/edited.md"]
        })
    );
}

#[test]
fn explicit_config_path_selects_and_materializes_custom_template() {
    // Arrange
    let directory = tempfile::tempdir().expect("create tempdir");
    let config_path = directory.path().join("custom-phase-golem.toml");
    std::fs::write(
        &config_path,
        r#"
[workflow_template]
id = "config-path-workflow"

[workflow_template.provenance]
source = "config/custom-phase-golem.toml"
revision = "7"

[[workflow_template.inputs]]
name = "change"

[[workflow_template.nodes]]
key = "ship"
title = "Ship ${change}"
description = "Loaded through an explicit config path"
executor_profile = "release-agent"

[workflow_template.nodes.execution_policy]
timeout_minutes = 22
max_retries = 3
workflows = ["release/ship.md"]

[workflow_template.nodes.verification]
required_checks = ["cargo test"]
"#,
    )
    .expect("write custom config");
    let store = common::setup_task_golem_store(directory.path());
    let config =
        load_config_from(Some(&config_path), directory.path()).expect("load custom config");
    let public_inputs =
        PublicTemplateInputs::new(BTreeMap::from([("change".to_string(), "S5".to_string())]));

    // Act
    let run = materialize_configured_run(&store, &config, &public_inputs)
        .expect("materialize custom config template");

    // Assert
    let items = store.load_active().expect("load custom config run");
    let ship = items
        .iter()
        .find(|item| item.id == run.node_mapping["ship"])
        .expect("find custom config node");
    assert_eq!(ship.title, "Ship S5");
    assert_eq!(ship.extensions[X_PG_TEMPLATE_ID], "config-path-workflow");
    assert_eq!(ship.extensions[X_PG_EXECUTOR_PROFILE], "release-agent");
    assert_eq!(
        ship.extensions[X_PG_TEMPLATE_PROVENANCE],
        serde_json::json!({"source": "config/custom-phase-golem.toml", "revision": "7"})
    );
    assert_eq!(
        ship.extensions[X_PG_EXECUTION_POLICY]["timeout_minutes"],
        22
    );
    assert_eq!(
        ship.extensions[X_PG_VERIFICATION]["required_checks"],
        serde_json::json!(["cargo test"])
    );
}

#[test]
fn template_schema_cannot_represent_credentials_or_trusted_executor_commands() {
    let cases = [
        r#"
[workflow_template]
id = "unsafe"
credential = "token"

[workflow_template.provenance]
source = "test"

[[workflow_template.nodes]]
key = "build"
title = "Build"
executor_profile = "agent"
"#,
        r#"
[workflow_template]
id = "unsafe"

[workflow_template.provenance]
source = "test"

[[workflow_template.nodes]]
key = "build"
title = "Build"
executor_profile = "agent"
executor_command = "trusted-adapter --token value"
"#,
    ];

    for unsafe_config in cases {
        assert!(
            toml::from_str::<phase_golem::config::PhaseGolemConfig>(unsafe_config).is_err(),
            "unsafe materialization field should be rejected"
        );
    }
}
