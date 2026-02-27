use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::config::PhaseGolemConfig;
use crate::pg_item::PgItem;
use crate::types::{ItemStatus, PhasePool};

/// A single preflight validation error with actionable context.
#[derive(Debug, Clone, PartialEq)]
pub struct PreflightError {
    /// What condition failed.
    pub condition: String,
    /// Where in the config the error originates.
    pub config_location: String,
    /// How to fix it.
    pub suggested_fix: String,
}

impl std::fmt::Display for PreflightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Preflight error: {}\n  Config: {}\n  Fix: {}",
            self.condition, self.config_location, self.suggested_fix
        )
    }
}

/// Run all preflight validation checks.
///
/// Phases:
/// 1. Structural validation — config correctness (fast, no I/O)
/// 2. Workflow probe — verify referenced workflow files exist on disk
/// 3. Item validation — in-progress items reference valid pipelines/phases (skipped when Phase 1 finds structural errors)
/// 4. Duplicate ID validation — ensure no two items share the same ID
/// 5. Dependency graph validation — detect dangling references and circular dependencies
///
/// Returns `Ok(())` if all checks pass, or `Err(Vec<PreflightError>)` with all errors.
pub fn run_preflight(
    config: &PhaseGolemConfig,
    items: &[PgItem],
    project_root: &Path,
    config_base: &Path,
) -> Result<(), Vec<PreflightError>> {
    let mut errors = Vec::new();

    // Phase 0: .task-golem/ directory existence check
    let task_golem_dir = project_root.join(".task-golem");
    if !task_golem_dir.is_dir() {
        errors.push(PreflightError {
            condition: ".task-golem/ directory not found".to_string(),
            config_location: format!("{}", task_golem_dir.display()),
            suggested_fix: "Run `tg init` to initialize the task-golem store".to_string(),
        });
        return Err(errors);
    }

    // Phase 1: Structural validation (reuses config::validate but with richer errors)
    errors.extend(validate_structure(config));

    // Snapshot before Phase 2; gates Phase 3 on Phase 1 results only
    let structural_ok = errors.is_empty();

    // Phase 2: Workflow probe — verify workflow files exist on disk
    if errors.is_empty() {
        errors.extend(probe_workflows(config, config_base));
    }

    // Phase 3: Item validation
    if structural_ok {
        errors.extend(validate_items(config, items));
    }

    // Phase 4: Duplicate ID validation
    errors.extend(validate_duplicate_ids(items));

    // Phase 5: Dependency graph validation
    errors.extend(validate_dependency_graph(items));

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// --- Phase 1: Structural validation ---

/// Validate config structure with actionable error messages.
///
/// This is richer than `config::validate()` — each error includes the config
/// location and a suggested fix.
fn validate_structure(config: &PhaseGolemConfig) -> Vec<PreflightError> {
    let mut errors = Vec::new();

    if config.execution.max_wip < 1 {
        errors.push(PreflightError {
            condition: "max_wip must be >= 1".to_string(),
            config_location: "phase-golem.toml → execution.max_wip".to_string(),
            suggested_fix: "Set max_wip to at least 1".to_string(),
        });
    }

    if config.execution.max_concurrent < 1 {
        errors.push(PreflightError {
            condition: "max_concurrent must be >= 1".to_string(),
            config_location: "phase-golem.toml → execution.max_concurrent".to_string(),
            suggested_fix: "Set max_concurrent to at least 1".to_string(),
        });
    }

    for (pipeline_name, pipeline) in &config.pipelines {
        if pipeline.phases.is_empty() {
            errors.push(PreflightError {
                condition: format!("Pipeline \"{}\" has no main phases", pipeline_name),
                config_location: format!("phase-golem.toml → pipelines.{}.phases", pipeline_name),
                suggested_fix: "Add at least one phase to the phases array".to_string(),
            });
        }

        // Check phase name uniqueness
        let mut seen_names = HashSet::new();
        for (idx, phase) in pipeline.pre_phases.iter().enumerate() {
            if !seen_names.insert(&phase.name) {
                errors.push(PreflightError {
                    condition: format!(
                        "Duplicate phase name \"{}\" in pipeline \"{}\"",
                        phase.name, pipeline_name
                    ),
                    config_location: format!(
                        "phase-golem.toml → pipelines.{}.pre_phases[{}]",
                        pipeline_name, idx
                    ),
                    suggested_fix: "Use unique phase names within a pipeline".to_string(),
                });
            }
        }
        for (idx, phase) in pipeline.phases.iter().enumerate() {
            if !seen_names.insert(&phase.name) {
                errors.push(PreflightError {
                    condition: format!(
                        "Duplicate phase name \"{}\" in pipeline \"{}\"",
                        phase.name, pipeline_name
                    ),
                    config_location: format!(
                        "phase-golem.toml → pipelines.{}.phases[{}]",
                        pipeline_name, idx
                    ),
                    suggested_fix: "Use unique phase names within a pipeline".to_string(),
                });
            }
        }

        // destructive rejected on pre_phases
        for (idx, phase) in pipeline.pre_phases.iter().enumerate() {
            if phase.is_destructive {
                errors.push(PreflightError {
                    condition: format!(
                        "Pre-phase \"{}\" in pipeline \"{}\" cannot be destructive",
                        phase.name, pipeline_name
                    ),
                    config_location: format!(
                        "phase-golem.toml → pipelines.{}.pre_phases[{}].destructive",
                        pipeline_name, idx
                    ),
                    suggested_fix: "Remove the destructive flag from pre_phases (only main phases can be destructive)".to_string(),
                });
            }
        }

        // staleness: block incompatible with max_wip > 1
        if config.execution.max_wip > 1 {
            for phase in pipeline.pre_phases.iter().chain(pipeline.phases.iter()) {
                if phase.staleness == crate::config::StalenessAction::Block {
                    errors.push(PreflightError {
                        condition: format!(
                            "Phase \"{}\" in pipeline \"{}\" uses staleness \"block\" which is incompatible with max_wip > 1",
                            phase.name, pipeline_name
                        ),
                        config_location: format!(
                            "phase-golem.toml → pipelines.{} → phase \"{}\" staleness + execution.max_wip",
                            pipeline_name, phase.name
                        ),
                        suggested_fix: "Either set max_wip to 1 or change staleness to \"warn\" or \"ignore\"".to_string(),
                    });
                }
            }
        }
    }

    errors
}

// --- Phase 2: Workflow file probe ---

/// Collect all unique workflow file paths across all pipelines.
fn collect_unique_workflows(config: &PhaseGolemConfig) -> Vec<String> {
    let mut workflows = HashSet::new();
    for pipeline in config.pipelines.values() {
        for phase in pipeline.pre_phases.iter().chain(pipeline.phases.iter()) {
            for workflow in &phase.workflows {
                workflows.insert(workflow.clone());
            }
        }
    }
    let mut sorted: Vec<String> = workflows.into_iter().collect();
    sorted.sort();
    sorted
}

/// Verify all referenced workflow files exist on disk.
///
/// Each workflow entry is a relative file path (relative to project root).
/// Preflight checks that the file exists and is readable.
fn probe_workflows(config: &PhaseGolemConfig, project_root: &Path) -> Vec<PreflightError> {
    let workflows = collect_unique_workflows(config);
    let mut errors = Vec::new();

    for workflow_path in &workflows {
        let absolute_path = project_root.join(workflow_path);
        if !absolute_path.exists() {
            errors.push(PreflightError {
                condition: format!("Workflow file not found: {}", workflow_path),
                config_location: "phase-golem.toml → pipelines → workflows".to_string(),
                suggested_fix: format!(
                    "Create the workflow file at {} or update the path",
                    workflow_path
                ),
            });
        }
    }

    errors
}

// --- Phase 3: Item validation ---

/// Validate that in-progress and scoping items reference valid pipeline/phase combos.
fn validate_items(config: &PhaseGolemConfig, items: &[PgItem]) -> Vec<PreflightError> {
    let mut errors = Vec::new();

    for item in items {
        // Only validate items that are actively being processed
        let status = item.pg_status();
        if status != ItemStatus::InProgress && status != ItemStatus::Scoping {
            continue;
        }

        // Check pipeline_type references a valid pipeline
        let pipeline_type_owned = item.pipeline_type().unwrap_or_else(|| "feature".to_string());
        let pipeline_type = pipeline_type_owned.as_str();
        let pipeline = match config.pipelines.get(pipeline_type) {
            Some(p) => p,
            None => {
                errors.push(PreflightError {
                    condition: format!(
                        "Item {} references unknown pipeline type \"{}\"",
                        item.id(), pipeline_type
                    ),
                    config_location: format!("items → {} → pipeline_type", item.id()),
                    suggested_fix: format!(
                        "Add a [pipelines.{}] section to phase-golem.toml or update the item's pipeline_type",
                        pipeline_type
                    ),
                });
                continue;
            }
        };

        // Check phase references a valid phase name
        if let Some(phase_name) = item.phase() {
            let phase_in_pre = pipeline.pre_phases.iter().any(|p| p.name == phase_name);
            let phase_in_main = pipeline.phases.iter().any(|p| p.name == phase_name);

            if !phase_in_pre && !phase_in_main {
                errors.push(PreflightError {
                    condition: format!(
                        "Item {} references unknown phase \"{}\" in pipeline \"{}\"",
                        item.id(), phase_name, pipeline_type
                    ),
                    config_location: format!("items → {} → phase", item.id()),
                    suggested_fix: format!(
                        "Update the item's phase to a valid phase name in the \"{}\" pipeline",
                        pipeline_type
                    ),
                });
                continue;
            }

            // Check phase_pool matches phase location
            if let Some(ref pool) = item.phase_pool() {
                let expected_pool = if phase_in_pre {
                    PhasePool::Pre
                } else {
                    PhasePool::Main
                };
                if *pool != expected_pool {
                    errors.push(PreflightError {
                        condition: format!(
                            "Item {} has phase_pool {:?} but phase \"{}\" is in {:?}",
                            item.id(), pool, phase_name, expected_pool
                        ),
                        config_location: format!(
                            "items → {} → phase_pool",
                            item.id()
                        ),
                        suggested_fix: format!(
                            "Update phase_pool to {:?} to match the phase's location in the pipeline",
                            expected_pool
                        ),
                    });
                }
            }
        }
    }

    errors
}

// --- Phase 4: Duplicate ID validation ---

/// Detect duplicate item IDs in the backlog.
///
/// Uses HashMap<&str, Vec<usize>> instead of HashSet::insert() (used by the
/// dependency graph phase) because we need to report ALL indices where a
/// duplicate ID appears, not just the second occurrence.
fn validate_duplicate_ids(items: &[PgItem]) -> Vec<PreflightError> {
    let mut id_indices: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, item) in items.iter().enumerate() {
        id_indices.entry(item.id()).or_default().push(index);
    }

    let mut duplicates: Vec<_> = id_indices
        .into_iter()
        .filter(|(_, indices)| indices.len() > 1)
        .collect();
    duplicates.sort_by_key(|(_, indices)| indices[0]);

    duplicates
        .into_iter()
        .map(|(id, indices)| PreflightError {
            condition: format!(
                "Duplicate item ID \"{}\" found at indices {:?}",
                id, indices
            ),
            config_location: "BACKLOG.yaml → items".to_string(),
            suggested_fix: "Remove or rename the duplicate item so each ID is unique".to_string(),
        })
        .collect()
}

// --- Phase 5: Dependency graph validation ---

/// Validate that the dependency graph has no dangling references or cycles.
///
/// Dangling references: an item depends on an ID that doesn't exist in the backlog.
/// Cycles: a set of non-Done items form a circular dependency chain.
pub fn validate_dependency_graph(items: &[PgItem]) -> Vec<PreflightError> {
    let mut errors = Vec::new();

    // Build set of all item IDs for dangling reference detection
    let all_ids: HashSet<&str> = items.iter().map(|item| item.id()).collect();

    // Check for dangling references
    for item in items {
        for dep_id in item.dependencies() {
            if !all_ids.contains(dep_id.as_str()) {
                errors.push(PreflightError {
                    condition: format!(
                        "Item '{}' depends on '{}' which does not exist in the backlog",
                        item.id(), dep_id
                    ),
                    config_location: format!(
                        "items → {} → dependencies",
                        item.id()
                    ),
                    suggested_fix: format!(
                        "Remove '{}' from {}'s dependencies, or add the missing item to the backlog",
                        dep_id, item.id()
                    ),
                });
            }
        }
    }

    // Filter to non-Done items for cycle detection
    let non_done_items: Vec<&PgItem> = items
        .iter()
        .filter(|item| item.pg_status() != ItemStatus::Done)
        .collect();

    for cycle in detect_cycles(&non_done_items) {
        let path = cycle.join(" → ");
        let cycle_items = cycle[..cycle.len() - 1].join(", ");
        errors.push(PreflightError {
            condition: format!("Circular dependency detected: {}", path),
            config_location: "BACKLOG.yaml → items → dependencies".to_string(),
            suggested_fix: format!(
                "Remove one dependency in the cycle to break it: {}",
                cycle_items
            ),
        });
    }

    errors
}

/// DFS three-color cycle detection on non-Done items.
///
/// Returns each cycle as a path like `["A", "B", "C", "A"]`.
fn detect_cycles(items: &[&PgItem]) -> Vec<Vec<String>> {
    #[derive(Clone, Copy, PartialEq)]
    enum VisitState {
        Unvisited,
        InStack,
        Done,
    }

    let item_ids: HashSet<&str> = items.iter().map(|item| item.id()).collect();
    let mut state: HashMap<&str, VisitState> = items
        .iter()
        .map(|item| (item.id(), VisitState::Unvisited))
        .collect();
    let mut cycles = Vec::new();

    fn dfs<'a>(
        item_id: &'a str,
        items: &'a [&PgItem],
        item_ids: &HashSet<&str>,
        state: &mut HashMap<&'a str, VisitState>,
        path: &mut Vec<&'a str>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        state.insert(item_id, VisitState::InStack);
        path.push(item_id);

        let item = items
            .iter()
            .find(|i| i.id() == item_id)
            .expect("BUG: DFS called with item_id not in items slice");
        for dep_id in item.dependencies() {
            // Skip edges to IDs not in our non-Done item set (dangling refs caught separately)
            if !item_ids.contains(dep_id.as_str()) {
                continue;
            }

            match state.get(dep_id.as_str()) {
                Some(VisitState::InStack) => {
                    // Found a back-edge — extract cycle from path
                    let cycle_start = path
                        .iter()
                        .position(|&id| id == dep_id.as_str())
                        .expect("BUG: InStack node not found in path during cycle detection");
                    let mut cycle: Vec<String> =
                        path[cycle_start..].iter().map(|&s| s.to_string()).collect();
                    cycle.push(dep_id.clone());
                    cycles.push(cycle);
                }
                Some(VisitState::Unvisited) => {
                    dfs(dep_id, items, item_ids, state, path, cycles);
                }
                _ => {} // Done — already fully explored
            }
        }

        path.pop();
        state.insert(item_id, VisitState::Done);
    }

    for item in items {
        if state.get(item.id()) == Some(&VisitState::Unvisited) {
            let mut path = Vec::new();
            dfs(
                item.id(),
                items,
                &item_ids,
                &mut state,
                &mut path,
                &mut cycles,
            );
        }
    }

    cycles
}
