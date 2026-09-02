use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use task_golem::cache::{self, SqlValue};
use task_golem::errors::TgError;
use task_golem::model::graph::GraphApplyCategory;
use task_golem::model::item::Item;
use task_golem::store::Store;
use task_golem::{GraphApplyItem, GraphApplyRequest, GraphApplyResult, GraphRef};
use uuid::Uuid;

use crate::config::{
    selected_workflow_template, PhaseGolemConfig, PublicExecutionPolicy, PublicTemplateInput,
    PublicTemplateInputs, VerificationPlan, WorkflowNode, WorkflowTemplate,
};

pub const X_PG_RUN_ID: &str = "x-pg-run-id";
pub const X_PG_TEMPLATE_ID: &str = "x-pg-template-id";
pub const X_PG_TEMPLATE_NODE_KEY: &str = "x-pg-template-node-key";
pub const X_PG_TEMPLATE_PROVENANCE: &str = "x-pg-template-provenance";
pub const X_PG_OWNER: &str = "x-pg-owner";
pub const X_PG_HUMAN_DECISION: &str = "x-pg-human-decision";
pub const X_PG_EXECUTOR_PROFILE: &str = "x-pg-executor-profile";
pub const X_PG_EXECUTION_POLICY: &str = "x-pg-execution-policy";
pub const X_PG_VERIFICATION: &str = "x-pg-verification";

const MATERIALIZATION_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializedRun {
    pub run_id: String,
    pub node_mapping: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum MaterializationError {
    #[error("invalid workflow template: {0}")]
    Template(String),
    #[error("materialization coordination for run '{run_id}' failed: {detail}")]
    Coordination { run_id: String, detail: String },
    #[error("materialization recovery for run '{run_id}' found inconsistent state: {detail}")]
    Recovery { run_id: String, detail: String },
    #[error(transparent)]
    Store(#[from] TgError),
}

pub fn materialize_configured_run(
    store: &Store,
    config: &PhaseGolemConfig,
    public_inputs: &PublicTemplateInputs,
) -> Result<MaterializedRun, MaterializationError> {
    materialize_run(store, &selected_workflow_template(config), public_inputs)
}

pub fn materialize_run(
    store: &Store,
    template: &WorkflowTemplate,
    public_inputs: &PublicTemplateInputs,
) -> Result<MaterializedRun, MaterializationError> {
    materialize_run_with_id(store, template, public_inputs, &Uuid::now_v7().to_string())
}

/// Materializes or reconstructs one PG run without repeating TG graph application.
pub fn materialize_run_with_id(
    store: &Store,
    template: &WorkflowTemplate,
    public_inputs: &PublicTemplateInputs,
    run_id: &str,
) -> Result<MaterializedRun, MaterializationError> {
    materialize_run_with_id_using_apply(store, template, public_inputs, run_id, |request| {
        store.apply_graph(request)
    })
}

fn materialize_run_with_id_using_apply(
    store: &Store,
    template: &WorkflowTemplate,
    public_inputs: &PublicTemplateInputs,
    run_id: &str,
    mut apply_graph: impl FnMut(GraphApplyRequest) -> Result<GraphApplyResult, TgError>,
) -> Result<MaterializedRun, MaterializationError> {
    if run_id.trim().is_empty() {
        return Err(template_error("run id cannot be empty"));
    }

    let resolved_inputs = resolve_inputs(&template.inputs, public_inputs)?;
    let graph_items = compile_template(template, &resolved_inputs)?;
    let run_lock = materialization_run_lock(store, run_id);
    let _decision_guard = run_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _decision_file_lock = acquire_materialization_file_lock(store, run_id)?;

    let expected_node_keys = graph_items
        .iter()
        .map(|item| item.key.clone())
        .collect::<BTreeSet<_>>();
    if let DiscoveredRun::Complete(node_mapping) =
        discover_run(store, template, run_id, &expected_node_keys)?
    {
        return Ok(MaterializedRun {
            run_id: run_id.to_string(),
            node_mapping,
        });
    }

    let request = GraphApplyRequest {
        items: graph_items
            .iter()
            .map(|item| graph_apply_item(item, template, run_id))
            .collect(),
    };
    let mut apply_attempts = 0;
    let result = loop {
        apply_attempts += 1;
        match apply_graph(request.clone()) {
            Ok(result) => break result,
            Err(apply_error) if is_uncertain_apply_error(&apply_error) => {
                match discover_run(store, template, run_id, &expected_node_keys)? {
                    DiscoveredRun::Complete(node_mapping) => {
                        return Ok(MaterializedRun {
                            run_id: run_id.to_string(),
                            node_mapping,
                        });
                    }
                    DiscoveredRun::Absent if apply_attempts == 1 => continue,
                    DiscoveredRun::Absent => return Err(MaterializationError::Store(apply_error)),
                }
            }
            Err(apply_error) => return Err(MaterializationError::Store(apply_error)),
        }
    };

    Ok(MaterializedRun {
        run_id: run_id.to_string(),
        node_mapping: result.mapping,
    })
}

fn is_uncertain_apply_error(error: &TgError) -> bool {
    matches!(
        error,
        TgError::GraphApply(error) if error.category == GraphApplyCategory::PersistenceFailure
    )
}

#[derive(Debug, PartialEq, Eq)]
enum DiscoveredRun {
    Absent,
    Complete(BTreeMap<String, String>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MaterializationRunKey {
    store_path: PathBuf,
    run_id: String,
}

fn materialization_run_lock(store: &Store, run_id: &str) -> Arc<Mutex<()>> {
    static RUN_LOCKS: OnceLock<Mutex<HashMap<MaterializationRunKey, Weak<Mutex<()>>>>> =
        OnceLock::new();

    let key = MaterializationRunKey {
        store_path: canonical_store_path(store),
        run_id: run_id.to_string(),
    };
    let mut locks = RUN_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);

    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }

    let lock = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

fn acquire_materialization_file_lock(
    store: &Store,
    run_id: &str,
) -> Result<fslock::LockFile, MaterializationError> {
    let store_path = canonical_store_path(store);
    let lock_directory = store_path
        .parent()
        .unwrap_or(store.project_dir())
        .join(".phase-golem")
        .join("materialization-locks");
    std::fs::create_dir_all(&lock_directory).map_err(|error| {
        materialization_coordination_error(
            run_id,
            format!("cannot create '{}': {error}", lock_directory.display()),
        )
    })?;

    let key = MaterializationRunKey {
        store_path,
        run_id: run_id.to_string(),
    };
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let lock_path = lock_directory.join(format!("{:016x}.lock", hasher.finish()));
    let mut lock = fslock::LockFile::open(&lock_path).map_err(|error| {
        materialization_coordination_error(
            run_id,
            format!("cannot open '{}': {error}", lock_path.display()),
        )
    })?;
    lock.lock().map_err(|error| {
        materialization_coordination_error(
            run_id,
            format!("cannot lock '{}': {error}", lock_path.display()),
        )
    })?;
    Ok(lock)
}

fn canonical_store_path(store: &Store) -> PathBuf {
    store
        .project_dir()
        .canonicalize()
        .unwrap_or_else(|_| store.project_dir().to_path_buf())
}

fn materialization_coordination_error(
    run_id: &str,
    detail: impl Into<String>,
) -> MaterializationError {
    MaterializationError::Coordination {
        run_id: run_id.to_string(),
        detail: detail.into(),
    }
}

fn discover_run(
    store: &Store,
    template: &WorkflowTemplate,
    run_id: &str,
    expected_node_keys: &BTreeSet<String>,
) -> Result<DiscoveredRun, MaterializationError> {
    let escaped_run_id = run_id.replace('\'', "''");
    let sql = format!(
        "SELECT id, extensions_json FROM tasks \
         WHERE json_extract(extensions_json, '$.\"{X_PG_RUN_ID}\"') = '{escaped_run_id}' \
         ORDER BY id"
    );
    let (query_result, archived_items) = query_active_and_load_archive(store, &sql)?;
    let has_active_matches = !query_result.rows.is_empty();
    let mut matching_items = Vec::new();
    let mut issues = BTreeSet::new();
    for row in query_result.rows {
        let [SqlValue::Text(item_id), SqlValue::Text(extensions_json)] = row.as_slice() else {
            issues.insert("metadata query returned a malformed row".to_string());
            continue;
        };
        let Ok(extensions) =
            serde_json::from_str::<BTreeMap<String, serde_json::Value>>(extensions_json)
        else {
            issues.insert(format!("item '{item_id}' has malformed metadata"));
            continue;
        };
        matching_items.push((item_id.clone(), extensions));
    }
    matching_items.extend(archived_items.into_iter().filter_map(|item| {
        (item
            .extensions
            .get(X_PG_RUN_ID)
            .and_then(|value| value.as_str())
            == Some(run_id))
        .then_some((item.id, item.extensions))
    }));
    if !has_active_matches && matching_items.is_empty() {
        return Ok(DiscoveredRun::Absent);
    }

    let mut node_mapping = BTreeMap::new();
    for (item_id, extensions) in matching_items {
        if extensions.get(X_PG_RUN_ID).and_then(|value| value.as_str()) != Some(run_id) {
            issues.insert(format!("item '{item_id}' has a mismatched run id"));
        }
        if extensions
            .get(X_PG_TEMPLATE_ID)
            .and_then(|value| value.as_str())
            != Some(template.id.as_str())
        {
            issues.insert(format!("item '{item_id}' has a mismatched template id"));
        }
        let Some(node_key) = extensions
            .get(X_PG_TEMPLATE_NODE_KEY)
            .and_then(|value| value.as_str())
        else {
            issues.insert(format!("item '{item_id}' has no template node key"));
            continue;
        };
        if !expected_node_keys.contains(node_key) {
            issues.insert(format!("unexpected node key '{node_key}'"));
        }
        if node_mapping
            .insert(node_key.to_string(), item_id.clone())
            .is_some()
        {
            issues.insert(format!("duplicate node key '{node_key}'"));
        }
    }

    let discovered_node_keys = node_mapping.keys().cloned().collect::<BTreeSet<_>>();
    let missing_node_keys = expected_node_keys
        .difference(&discovered_node_keys)
        .cloned()
        .collect::<Vec<_>>();
    if !missing_node_keys.is_empty() {
        issues.insert(format!(
            "missing node keys [{}]",
            missing_node_keys.join(", ")
        ));
    }

    if issues.is_empty() {
        Ok(DiscoveredRun::Complete(node_mapping))
    } else {
        Err(MaterializationError::Recovery {
            run_id: run_id.to_string(),
            detail: issues.into_iter().collect::<Vec<_>>().join("; "),
        })
    }
}

fn query_active_and_load_archive(
    store: &Store,
    sql: &str,
) -> Result<(task_golem::cache::QueryResult, Vec<Item>), TgError> {
    static QUERY_LOCK: Mutex<()> = Mutex::new(());

    let _query_guard = QUERY_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    loop {
        let snapshot = store.with_lock(|store| {
            if cache::is_stale(store)? {
                return Ok(None);
            }
            let active_items = cache::query::execute(store, sql, MATERIALIZATION_QUERY_TIMEOUT)?;
            let archived_items = store.load_all_archive()?;
            Ok(Some((active_items, archived_items)))
        })?;
        if let Some(snapshot) = snapshot {
            return Ok(snapshot);
        }

        cache::query::execute(store, sql, MATERIALIZATION_QUERY_TIMEOUT)?;
    }
}

#[derive(Clone)]
struct CompiledNode {
    key: String,
    title: String,
    description: Option<String>,
    priority: i64,
    tags: Vec<String>,
    parent: Option<String>,
    dependencies: Vec<String>,
    human_decision: bool,
    executor_profile: String,
    execution_policy: PublicExecutionPolicy,
    verification: VerificationPlan,
}

fn resolve_inputs(
    declarations: &[PublicTemplateInput],
    supplied: &PublicTemplateInputs,
) -> Result<BTreeMap<String, String>, MaterializationError> {
    let mut resolved = BTreeMap::new();
    for declaration in declarations {
        if declaration.name.trim().is_empty() {
            return Err(template_error("input name cannot be empty"));
        }
        if resolved.contains_key(&declaration.name) {
            return Err(template_error(format!(
                "duplicate input declaration '{}'",
                declaration.name
            )));
        }
        let value = supplied
            .get(&declaration.name)
            .cloned()
            .or_else(|| declaration.default.clone())
            .ok_or_else(|| template_error(format!("missing input '{}'", declaration.name)))?;
        resolved.insert(declaration.name.clone(), value);
    }

    if let Some(name) = supplied.keys().find(|name| !resolved.contains_key(*name)) {
        return Err(template_error(format!("undeclared input '{name}'")));
    }
    Ok(resolved)
}

fn compile_template(
    template: &WorkflowTemplate,
    inputs: &BTreeMap<String, String>,
) -> Result<Vec<CompiledNode>, MaterializationError> {
    if template.id.trim().is_empty() {
        return Err(template_error("template id cannot be empty"));
    }
    if template.provenance.source.trim().is_empty() {
        return Err(template_error("template provenance source cannot be empty"));
    }
    if template.nodes.is_empty() {
        return Err(template_error("template must declare at least one node"));
    }

    let mut node_keys = HashSet::new();
    let mut compiled = Vec::with_capacity(template.nodes.len());
    for node in &template.nodes {
        validate_node(node, &mut node_keys)?;
        compiled.push(resolve_node(node, inputs)?);
    }
    validate_references_and_cycles(&compiled, &node_keys)?;
    Ok(compiled)
}

fn validate_node(
    node: &WorkflowNode,
    node_keys: &mut HashSet<String>,
) -> Result<(), MaterializationError> {
    if node.key.trim().is_empty() {
        return Err(template_error("node key cannot be empty"));
    }
    if !node_keys.insert(node.key.clone()) {
        return Err(template_error(format!("duplicate node key '{}'", node.key)));
    }
    if node.executor_profile.trim().is_empty() {
        return Err(template_error(format!(
            "node '{}' executor profile cannot be empty",
            node.key
        )));
    }
    Ok(())
}

fn resolve_node(
    node: &WorkflowNode,
    inputs: &BTreeMap<String, String>,
) -> Result<CompiledNode, MaterializationError> {
    let title = interpolate(&node.title, inputs)?;
    if title.trim().is_empty() || title.contains(['\n', '\r']) {
        return Err(template_error(format!(
            "node '{}' title must be one non-empty line",
            node.key
        )));
    }
    let description = node
        .description
        .as_deref()
        .map(|description| interpolate(description, inputs))
        .transpose()?;

    Ok(CompiledNode {
        key: node.key.clone(),
        title,
        description,
        priority: node.priority,
        tags: node.tags.clone(),
        parent: node.parent.clone(),
        dependencies: node.dependencies.clone(),
        human_decision: node.human_decision,
        executor_profile: node.executor_profile.clone(),
        execution_policy: node.execution_policy.clone(),
        verification: node.verification.clone(),
    })
}

fn interpolate(
    value: &str,
    inputs: &BTreeMap<String, String>,
) -> Result<String, MaterializationError> {
    let mut resolved = String::new();
    let mut remaining = value;
    while let Some(start) = remaining.find("${") {
        resolved.push_str(&remaining[..start]);
        let end = remaining[start + 2..]
            .find('}')
            .ok_or_else(|| template_error(format!("unterminated input in '{value}'")))?;
        let name = &remaining[start + 2..start + end + 2];
        let replacement = inputs
            .get(name)
            .ok_or_else(|| template_error(format!("unresolved input '{name}'")))?;
        resolved.push_str(replacement);
        remaining = &remaining[start + end + 3..];
    }
    resolved.push_str(remaining);
    Ok(resolved)
}

fn validate_references_and_cycles(
    nodes: &[CompiledNode],
    node_keys: &HashSet<String>,
) -> Result<(), MaterializationError> {
    for node in nodes {
        if let Some(parent) = &node.parent {
            validate_reference(&node.key, parent, "parent", node_keys)?;
        }
        let mut dependencies = HashSet::new();
        for dependency in &node.dependencies {
            validate_reference(&node.key, dependency, "dependency", node_keys)?;
            if !dependencies.insert(dependency) {
                return Err(template_error(format!(
                    "node '{}' repeats dependency '{}'",
                    node.key, dependency
                )));
            }
        }
    }

    validate_acyclic(
        nodes,
        |node| node.parent.iter().cloned().collect(),
        "parent",
    )?;
    validate_acyclic(nodes, |node| node.dependencies.clone(), "dependency")
}

fn validate_reference(
    node_key: &str,
    target: &str,
    relation: &str,
    node_keys: &HashSet<String>,
) -> Result<(), MaterializationError> {
    if target == node_key {
        return Err(template_error(format!(
            "node '{node_key}' cannot {relation} itself"
        )));
    }
    if !node_keys.contains(target) {
        return Err(template_error(format!(
            "node '{node_key}' has unresolved {relation} '{target}'"
        )));
    }
    Ok(())
}

fn validate_acyclic(
    nodes: &[CompiledNode],
    edges: impl Fn(&CompiledNode) -> Vec<String>,
    relation: &str,
) -> Result<(), MaterializationError> {
    let by_key: HashMap<&str, &CompiledNode> =
        nodes.iter().map(|node| (node.key.as_str(), node)).collect();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();

    for node in nodes {
        if has_cycle(
            node.key.as_str(),
            &by_key,
            &edges,
            &mut visiting,
            &mut visited,
        ) {
            return Err(template_error(format!("{relation} cycle detected")));
        }
    }
    Ok(())
}

fn has_cycle(
    key: &str,
    nodes: &HashMap<&str, &CompiledNode>,
    edges: &impl Fn(&CompiledNode) -> Vec<String>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) -> bool {
    if visited.contains(key) {
        return false;
    }
    if !visiting.insert(key.to_string()) {
        return true;
    }
    let has_cycle = edges(nodes[key])
        .into_iter()
        .any(|target| has_cycle(&target, nodes, edges, visiting, visited));
    visiting.remove(key);
    visited.insert(key.to_string());
    has_cycle
}

fn graph_apply_item(
    node: &CompiledNode,
    template: &WorkflowTemplate,
    run_id: &str,
) -> GraphApplyItem {
    GraphApplyItem {
        reference: node.key.clone(),
        title: node.title.clone(),
        description: node.description.clone(),
        priority: node.priority,
        tags: node.tags.clone(),
        parent: node.parent.clone().map(GraphRef::Local),
        dependencies: node
            .dependencies
            .iter()
            .cloned()
            .map(GraphRef::Local)
            .collect(),
        extensions: snapshot_metadata(node, template, run_id),
    }
}

fn snapshot_metadata(
    node: &CompiledNode,
    template: &WorkflowTemplate,
    run_id: &str,
) -> BTreeMap<String, serde_json::Value> {
    BTreeMap::from([
        (X_PG_RUN_ID.to_string(), serde_json::json!(run_id)),
        (X_PG_TEMPLATE_ID.to_string(), serde_json::json!(template.id)),
        (
            X_PG_TEMPLATE_NODE_KEY.to_string(),
            serde_json::json!(node.key),
        ),
        (
            X_PG_TEMPLATE_PROVENANCE.to_string(),
            serde_json::to_value(&template.provenance).expect("template provenance must serialize"),
        ),
        (X_PG_OWNER.to_string(), serde_json::json!("phase-golem")),
        (
            X_PG_HUMAN_DECISION.to_string(),
            serde_json::json!(node.human_decision),
        ),
        (
            X_PG_EXECUTOR_PROFILE.to_string(),
            serde_json::json!(node.executor_profile),
        ),
        (
            X_PG_EXECUTION_POLICY.to_string(),
            serde_json::to_value(&node.execution_policy).expect("execution policy must serialize"),
        ),
        (
            X_PG_VERIFICATION.to_string(),
            serde_json::to_value(&node.verification).expect("verification plan must serialize"),
        ),
    ])
}

fn template_error(message: impl Into<String>) -> MaterializationError {
    MaterializationError::Template(message.into())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use task_golem::model::graph::{
        GraphApplyDiagnostic, GraphApplyDiagnosticCode, GraphApplyError,
    };

    use super::*;

    #[test]
    fn committed_graph_is_recovered_after_uncertain_apply_response() {
        // Arrange
        let (_directory, store) = setup_store();
        let template = selected_workflow_template(&PhaseGolemConfig::default());
        let expected_node_count = template.nodes.len();

        // Act
        let recovered = materialize_run_with_id_using_apply(
            &store,
            &template,
            &PublicTemplateInputs::default(),
            "uncertain-response-run",
            |request| {
                store.apply_graph(request).expect("commit graph");
                Err(simulated_uncertain_response())
            },
        )
        .expect("recover committed graph");

        // Assert
        assert_eq!(recovered.node_mapping.len(), expected_node_count);
        assert_eq!(
            store.load_active().expect("load recovered graph").len(),
            expected_node_count,
            "uncertain response recovery must not apply a duplicate graph"
        );
    }

    #[test]
    fn absent_state_after_uncertain_response_permits_one_safe_application() {
        // Arrange
        let (_directory, store) = setup_store();
        let template = selected_workflow_template(&PhaseGolemConfig::default());
        let expected_node_count = template.nodes.len();
        let mut apply_attempts = 0;

        // Act
        let materialized = materialize_run_with_id_using_apply(
            &store,
            &template,
            &PublicTemplateInputs::default(),
            "uncertain-absent-run",
            |request| {
                apply_attempts += 1;
                if apply_attempts == 1 {
                    Err(simulated_uncertain_response())
                } else {
                    store.apply_graph(request)
                }
            },
        )
        .expect("apply after absent recovery query");

        // Assert
        assert_eq!(apply_attempts, 2);
        assert_eq!(materialized.node_mapping.len(), expected_node_count);
        assert_eq!(
            store.load_active().expect("load materialized graph").len(),
            expected_node_count
        );
    }

    #[test]
    fn partial_state_after_uncertain_response_fails_without_retrying() {
        // Arrange
        let (_directory, store) = setup_store();
        let template = selected_workflow_template(&PhaseGolemConfig::default());
        let expected_node_count = template.nodes.len();
        let mut apply_attempts = 0;

        // Act
        let result = materialize_run_with_id_using_apply(
            &store,
            &template,
            &PublicTemplateInputs::default(),
            "uncertain-partial-run",
            |request| {
                apply_attempts += 1;
                store
                    .apply_graph(GraphApplyRequest {
                        items: request.items.into_iter().take(1).collect(),
                    })
                    .expect("commit partial simulated state");
                Err(simulated_uncertain_response())
            },
        );

        // Assert
        assert_eq!(apply_attempts, 1, "partial recovery must not retry");
        assert!(matches!(result, Err(MaterializationError::Recovery { .. })));
        assert_eq!(
            store.load_active().expect("load partial graph").len(),
            1,
            "partial recovery must not create the remaining graph"
        );
        assert!(expected_node_count > 1);
    }

    fn setup_store() -> (tempfile::TempDir, Store) {
        let directory = tempfile::tempdir().expect("create tempdir");
        let store_directory = directory.path().join(".task-golem");
        fs::create_dir_all(&store_directory).expect("create task-golem directory");
        fs::write(
            store_directory.join("archive.jsonl"),
            "{\"schema_version\":1}\n",
        )
        .expect("initialize archive");
        let store = Store::new(store_directory);
        store.save_active(&[]).expect("initialize active store");
        (directory, store)
    }

    fn simulated_uncertain_response() -> TgError {
        GraphApplyError::new(
            GraphApplyCategory::PersistenceFailure,
            vec![GraphApplyDiagnostic::new(
                GraphApplyDiagnosticCode::PersistenceFailure,
                "response",
            )],
        )
        .into()
    }
}
