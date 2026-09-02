use clap::{Parser, Subcommand};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

use phase_golem::agent::{
    install_signal_handlers, is_shutdown_requested, kill_all_children, LocalProcessExecutorAdapter,
};
use task_golem::store::Store;

use phase_golem::config;
use phase_golem::coordinator;
use phase_golem::filter;
use phase_golem::lock;
use phase_golem::log::parse_log_level;
use phase_golem::pg_item::{self, PgItem};
use phase_golem::scheduler;
use phase_golem::types::DimensionLevel;
use phase_golem::{log_info, log_warn};
use task_golem::model::status::Status;

use task_golem::git as tg_git;

const MAX_BACKLOG_PREVIEW_ITEMS: usize = 3;

#[derive(Parser)]
#[command(name = "phase-golem", about = "Autonomous changes workflow engine")]
struct Cli {
    /// Project root directory (defaults to current directory)
    #[arg(long, default_value = ".")]
    root: PathBuf,

    /// Path to config file (defaults to {root}/phase-golem.toml).
    /// When specified, config-relative paths (backlog, workflows) resolve
    /// from the config file's parent directory.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Log verbosity level (error, warn, info, debug)
    #[arg(long, default_value = "info")]
    log_level: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize phase-golem directories and config
    Init,
    /// Run one finite foreground supervisor drain
    Run {
        /// Limit the foreground drain to specific TG item IDs
        #[arg(long, action = clap::ArgAction::Append)]
        target: Vec<String>,
        /// Filter items by attribute. Comma-separated values = OR within field; repeated flags = AND across fields. Examples: --only impact=high,medium --only size=small (high or medium impact AND small size). Tag: --only tag=a,b (has either) vs --only tag=a --only tag=b (has both).
        #[arg(long, conflicts_with = "target", action = clap::ArgAction::Append)]
        only: Vec<String>,
        /// Maximum number of supervised task executions
        #[arg(long, default_value = "100")]
        cap: u32,
    },
    /// Materialize the configured workflow template into Task Golem
    Materialize {
        /// Supply a public template input as NAME=VALUE; repeat for multiple inputs
        #[arg(long, action = clap::ArgAction::Append)]
        input: Vec<String>,
        /// Reuse a PG run identity for recovery and duplicate prevention
        #[arg(long)]
        run_id: Option<String>,
    },
    /// Show backlog status
    Status,
    /// Advance an item to next or specific phase
    Advance {
        /// Item ID to advance
        item_id: String,
        /// Target phase to skip to (must be a valid phase name in the item's pipeline)
        #[arg(long)]
        to: Option<String>,
    },
    /// Unblock a blocked item
    Unblock {
        /// Item ID to unblock
        item_id: String,
        /// Decision context notes
        #[arg(long)]
        notes: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match parse_log_level(&cli.log_level) {
        Ok(level) => phase_golem::log::set_log_level(level),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }

    let root = &cli.root;

    let (config_path, config_base) = match &cli.config {
        Some(p) => (
            Some(p.clone()),
            p.parent().unwrap_or(Path::new(".")).to_path_buf(),
        ),
        None => (None, root.to_path_buf()),
    };

    let result = match cli.command {
        Commands::Init => handle_init(root),
        Commands::Run { target, only, cap } => {
            handle_run(
                root,
                config_path.as_deref(),
                &config_base,
                target,
                only,
                cap,
            )
            .await
        }
        Commands::Materialize { input, run_id } => {
            handle_materialize(root, config_path.as_deref(), input, run_id)
        }
        Commands::Status => handle_status(root, config_path.as_deref(), &config_base),
        Commands::Advance { item_id, to } => {
            handle_advance(root, config_path.as_deref(), &config_base, &item_id, to)
        }
        Commands::Unblock { item_id, notes } => {
            handle_unblock(root, config_path.as_deref(), &config_base, &item_id, notes)
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn handle_init(root: &Path) -> Result<(), String> {
    // Only check that a git repo exists -- init doesn't require clean tree or branch
    phase_golem::git::is_git_repo(None)
        .map_err(|_| "Not a git repository. Run `git init` first.".to_string())?;

    // Check for .task-golem/ directory
    let task_golem_dir = root.join(".task-golem");
    if !task_golem_dir.is_dir() {
        eprintln!(
            "Warning: .task-golem/ directory not found at {}\n\
             Run `tg init` to initialize the task-golem store before using phase-golem.",
            task_golem_dir.display()
        );
    }

    // Create directories
    let dirs = ["_ideas", "_worklog", "changes", ".phase-golem"];
    for dir in &dirs {
        let dir_path = root.join(dir);
        fs::create_dir_all(&dir_path)
            .map_err(|e| format!("Failed to create {}: {}", dir_path.display(), e))?;
    }

    // Create phase-golem.toml if it doesn't exist (with default pipelines section)
    let config_path = root.join("phase-golem.toml");
    if !config_path.exists() {
        let config_contents = r#"[guardrails]
max_size = "medium"
max_complexity = "medium"
max_risk = "low"
# min_impact = "medium"

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
    {{ name = "research", workflows = [".claude/skills/changes/workflows/orchestration/research-scope.md"], is_destructive = false }},
]
phases = [
    {{ name = "prd",           workflows = [".claude/skills/changes/workflows/0-prd/create-prd.md"],                     is_destructive = false }},
    {{ name = "tech-research", workflows = [".claude/skills/changes/workflows/1-tech-research/tech-research.md"],       is_destructive = false }},
    {{ name = "design",        workflows = [".claude/skills/changes/workflows/2-design/design.md"],                       is_destructive = false }},
    {{ name = "spec",           workflows = [".claude/skills/changes/workflows/3-spec/create-spec.md"],                    is_destructive = false }},
    {{ name = "build",          workflows = [".claude/skills/changes/workflows/4-build/implement-spec-autonomous.md"],   is_destructive = true }},
    {{ name = "review",         workflows = [".claude/skills/changes/workflows/5-review/change-review.md"],               is_destructive = false }},
]
"#;
        fs::write(&config_path, config_contents)
            .map_err(|e| format!("Failed to write {}: {}", config_path.display(), e))?;
    }

    // Append .phase-golem/ to .gitignore if not already present
    let gitignore_path = root.join(".gitignore");
    let gitignore_entry = ".phase-golem/";

    let existing_gitignore = if gitignore_path.exists() {
        fs::read_to_string(&gitignore_path)
            .map_err(|e| format!("Failed to read .gitignore: {}", e))?
    } else {
        String::new()
    };

    let has_entry = existing_gitignore
        .lines()
        .any(|line| line.trim() == gitignore_entry);

    if !has_entry {
        let mut contents = existing_gitignore;
        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str(gitignore_entry);
        contents.push('\n');

        fs::write(&gitignore_path, contents)
            .map_err(|e| format!("Failed to write .gitignore: {}", e))?;
    }

    println!("Initialized phase-golem in {}", root.display());
    println!("  Created: _ideas/, _worklog/, changes/, .phase-golem/");
    println!("  Config: phase-golem.toml");
    println!("  Updated: .gitignore");

    Ok(())
}

/// Delete all `phase_result_*.json` files from the runtime directory.
///
/// Used at startup (before agents spawn) and shutdown (after all agents complete)
/// as a defense-in-depth layer against stale result files from crashed runs.
/// Swallows all errors — cleanup failure is non-critical.
// NOTE: must match executor::result_file_path() naming convention
async fn cleanup_stale_result_files(runtime_dir: &Path, context: &str) {
    let mut entries = match tokio::fs::read_dir(runtime_dir).await {
        Ok(entries) => entries,
        Err(err) => {
            log_warn!(
                "[{}] Failed to read {} for cleanup: {}",
                context,
                runtime_dir.display(),
                err
            );
            return;
        }
    };

    let mut deleted_count: u32 = 0;
    loop {
        let entry = match entries.next_entry().await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(err) => {
                log_warn!(
                    "[{}] Failed to read directory entry during cleanup: {}",
                    context,
                    err
                );
                break;
            }
        };

        let name = entry.file_name();
        let name = name.to_string_lossy();
        // NOTE: must match executor::result_file_path() naming convention
        if name.starts_with("phase_result_") && name.ends_with(".json") {
            if let Err(err) = tokio::fs::remove_file(entry.path()).await {
                log_warn!(
                    "[{}] Failed to delete stale result file {}: {}",
                    context,
                    entry.path().display(),
                    err
                );
                continue;
            }
            deleted_count += 1;
        }
    }

    if deleted_count > 0 {
        log_info!(
            "[{}] Cleaned up {} stale result file(s) from .phase-golem/",
            context,
            deleted_count
        );
    }
}

async fn handle_run(
    root: &Path,
    config_path: Option<&Path>,
    _config_base: &Path,
    target: Vec<String>,
    only: Vec<String>,
    cap: u32,
) -> Result<(), String> {
    // Install signal handlers for graceful shutdown
    install_signal_handlers()?;

    log_info!("--- Phase Golem ---");
    log_info!("");

    // Prechecks
    log_info!("[pre] Acquiring lock...");
    let runtime_dir = root.join(".phase-golem");
    let _lock = lock::try_acquire(&runtime_dir)?;
    cleanup_stale_result_files(&runtime_dir, "pre").await;
    log_info!("[pre] Checking git preconditions...");
    phase_golem::git::check_preconditions(Some(root))?;

    // Load
    let config = config::load_config_from(config_path, root)?;

    // Construct Store for task-golem access
    let tg_store_dir = root.join(".task-golem");
    let store = Store::new(tg_store_dir.clone());

    // Load items via Store for validation and display
    let snapshot = coordinator::load_work_snapshot(&store)
        .map_err(|e| format!("Failed to load task-golem store: {e}"))?;
    let items = &snapshot.items;

    // Mutual exclusivity safety net (clap conflicts_with should handle this)
    if !target.is_empty() && !only.is_empty() {
        return Err("Cannot combine --target and --only flags. Use one or the other.".to_string());
    }

    // Target validation
    let target: Vec<String> = target.iter().map(|t| t.trim().to_string()).collect();
    if !target.is_empty() {
        let mut errors = Vec::new();

        for t in &target {
            if task_golem::validate_id(t).is_err() {
                errors.push(format!("Invalid target '{t}': expected a canonical UUIDv7"));
            }
        }

        // Existence validation
        for t in &target {
            if !items.iter().any(|i| i.id() == t.as_str()) {
                errors.push(format!("Target '{}' not found in backlog", t));
            }
        }

        // Duplicate detection
        let mut seen = HashSet::new();
        let mut duplicates = HashSet::new();
        for t in &target {
            if !seen.insert(t.as_str()) {
                duplicates.insert(t.as_str());
            }
        }
        if !duplicates.is_empty() {
            let dup_list: Vec<&str> = duplicates.into_iter().collect();
            errors.push(format!("Duplicate targets: {}", dup_list.join(", ")));
        }

        if !errors.is_empty() {
            let msg = format!("Target validation failed:\n  - {}", errors.join("\n  - "));
            return Err(msg);
        }
    }

    // Filter validation
    let parsed_filters: Vec<filter::FilterCriterion> = only
        .iter()
        .map(|raw| filter::parse_filter(raw))
        .collect::<Result<Vec<_>, _>>()?;
    filter::validate_filter_criteria(&parsed_filters)?;

    // Config summary
    log_info!("");
    log_info!(
        "[config] Guardrails: max_size={}, max_complexity={}, max_risk={}",
        format!("{:?}", config.guardrails.max_size).to_lowercase(),
        format!("{:?}", config.guardrails.max_complexity).to_lowercase(),
        format!("{:?}", config.guardrails.max_risk).to_lowercase(),
    );
    log_info!(
        "[config] Execution: timeout={}min, retries={}",
        config.execution.phase_timeout_minutes,
        config.execution.max_retries,
    );
    if target.len() == 1 {
        log_info!("[config] Target: {}", target[0]);
    } else if target.len() > 1 {
        let target_display: Vec<String> = target
            .iter()
            .enumerate()
            .map(|(i, t)| {
                if i == 0 {
                    format!("{} (active, 1/{})", t, target.len())
                } else {
                    t.clone()
                }
            })
            .collect();
        log_info!("[config] Targets: {}", target_display.join(", "));
    }
    if !parsed_filters.is_empty() {
        let matching = filter::apply_filters(&parsed_filters, items);
        log_info!(
            "[config] Filter: {} — {} items match (from {} total)",
            filter::format_filter_criteria(&parsed_filters),
            matching.len(),
            items.len()
        );
    }
    log_info!("[config] Phase cap: {}", cap);

    // Pipeline summary
    log_info!("");
    for (name, pipeline) in &config.pipelines {
        let pre_names: Vec<&str> = pipeline
            .pre_phases
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        let main_names: Vec<&str> = pipeline.phases.iter().map(|p| p.name.as_str()).collect();
        log_info!(
            "[pipeline:{}] pre: [{}] -> main: [{}]",
            name,
            pre_names.join(" -> "),
            main_names.join(" -> "),
        );
    }

    // Backlog summary
    let todo_count = items.iter().filter(|i| i.status() == Status::Todo).count();
    let doing_count = items.iter().filter(|i| i.status() == Status::Doing).count();
    let blocked_count = items
        .iter()
        .filter(|i| i.status() == Status::Blocked)
        .count();
    let done_count = items.iter().filter(|i| i.status() == Status::Done).count();
    log_info!("");
    log_info!(
        "[backlog] {} items: {} todo, {} doing, {} blocked, {} done",
        items.len(),
        todo_count,
        doing_count,
        blocked_count,
        done_count,
    );

    // Queue preview — show first few actionable items
    let actionable: Vec<&PgItem> = items
        .iter()
        .filter(|i| matches!(i.status(), Status::Todo | Status::Doing))
        .take(MAX_BACKLOG_PREVIEW_ITEMS)
        .collect();
    if !actionable.is_empty() {
        log_info!("[backlog] Next up:");
        for item in &actionable {
            let phase_owned = item.phase().unwrap_or_else(|| "-".to_string());
            log_info!(
                "  {} ({:?}) phase={} -- {}",
                item.id(),
                item.status(),
                phase_owned,
                item.title()
            );
        }
        if items
            .iter()
            .filter(|i| matches!(i.status(), Status::Todo | Status::Doing))
            .count()
            > MAX_BACKLOG_PREVIEW_ITEMS
        {
            log_info!("  ...");
        }
    }

    log_info!("");
    let (coord_handle, coord_task) = coordinator::spawn_coordinator(store, root.to_path_buf());

    // Set up cancellation for graceful shutdown
    let cancel = CancellationToken::new();
    let cancel_clone = cancel.clone();

    // Spawn shutdown monitor that watches for signal and cancels
    tokio::spawn(async move {
        loop {
            if is_shutdown_requested() {
                cancel_clone.cancel();
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    });

    let filter_display = if !parsed_filters.is_empty() {
        Some(filter::format_filter_criteria(&parsed_filters))
    } else {
        None
    };

    let params = scheduler::RunParams {
        targets: target,
        filter: parsed_filters,
        cap,
    };
    let is_targeted_run = !params.targets.is_empty();

    let adapter = LocalProcessExecutorAdapter::new(root.to_path_buf());
    let verifier = phase_golem::executor::ShellVerifier::new(root.to_path_buf());
    let summary = scheduler::run_foreground_supervisor(
        coord_handle,
        &config,
        &params,
        &cancel,
        &adapter,
        &verifier,
    )
    .await?;

    // Kill any remaining child processes
    tokio::task::spawn_blocking(move || {
        kill_all_children();
    })
    .await
    .unwrap_or_else(|e| log_warn!("kill_all_children task panicked: {}", e));

    // Await coordinator shutdown (ensures pending operations complete)
    if let Err(err) = coord_task.await {
        log_warn!(
            "Coordinator task panicked, skipping shutdown commit: {:?}",
            err
        );
    } else {
        // Commit tasks.jsonl if it has uncommitted changes
        let root_for_commit = root.to_path_buf();
        let tg_store_dir_for_commit = tg_store_dir.clone();
        let halt_reason_display = format!("{:?}", summary.halt_reason);

        let commit_result = tokio::task::spawn_blocking(move || {
            let status = match phase_golem::git::get_status(Some(&root_for_commit)) {
                Ok(s) => s,
                Err(err) => {
                    return Err(format!("get_status failed: {}", err));
                }
            };

            let tasks_rel = tg_store_dir_for_commit
                .join("tasks.jsonl")
                .strip_prefix(&root_for_commit)
                .unwrap_or(Path::new(".task-golem/tasks.jsonl"))
                .to_string_lossy()
                .to_string();
            let is_tasks_dirty = status
                .iter()
                .any(|entry| entry.path.trim_matches('"') == tasks_rel.as_str());

            if !is_tasks_dirty {
                return Ok(None);
            }

            if let Err(err) = tg_git::stage_self(&root_for_commit) {
                return Err(format!("tg_git::stage_self failed: {}", err));
            }

            let message = format!(
                "[phase-golem] Save task state on halt ({})",
                halt_reason_display
            );
            match tg_git::commit(&message, &root_for_commit) {
                Ok(sha) => Ok(Some(sha)),
                Err(err) => Err(format!("tg_git::commit failed: {}", err)),
            }
        })
        .await;

        match commit_result {
            Ok(Ok(Some(sha))) => {
                log_info!("Committed task state on halt: {}", sha);
            }
            Ok(Ok(None)) => {
                // tasks.jsonl was clean, nothing to commit
            }
            Ok(Err(err)) => {
                log_warn!("Shutdown commit skipped: {}", err);
            }
            Err(err) => {
                log_warn!("spawn_blocking panicked during shutdown commit: {:?}", err);
            }
        }
    }

    cleanup_stale_result_files(&runtime_dir, "post").await;

    // Print summary
    log_info!("\n--- Run Summary ---");
    log_info!("Tasks executed: {}", summary.tasks_executed);
    if !summary.items_completed.is_empty() {
        log_info!("Items completed: {}", summary.items_completed.join(", "));
    }
    if !summary.items_blocked.is_empty() {
        log_info!("Items blocked: {}", summary.items_blocked.join(", "));
    }
    if let Some(filter_str) = filter_display {
        log_info!("Filter: {}", filter_str);
    }
    log_info!("Halt reason: {:?}", summary.halt_reason);

    if is_targeted_run && summary.halt_reason != scheduler::HaltReason::SelectedScopeComplete {
        return Err(format!(
            "Targeted run did not complete the selected scope (halted: {:?})",
            summary.halt_reason
        ));
    }

    if summary.items_completed.is_empty() && !summary.items_blocked.is_empty() {
        return Err("All targets blocked; no items completed".to_string());
    }

    Ok(())
}

#[derive(serde::Serialize)]
struct MaterializeOutput {
    run_id: String,
    node_mapping: BTreeMap<String, String>,
}

fn handle_materialize(
    root: &Path,
    config_path: Option<&Path>,
    inputs: Vec<String>,
    run_id: Option<String>,
) -> Result<(), String> {
    let public_inputs = parse_public_inputs(inputs)?;
    let config = config::load_config_from(config_path, root)?;
    let store = Store::new(root.join(".task-golem"));
    let materialized = match run_id {
        Some(run_id) => phase_golem::materialization::materialize_run_with_id(
            &store,
            &config::selected_workflow_template(&config),
            &public_inputs,
            &run_id,
        ),
        None => phase_golem::materialization::materialize_configured_run(
            &store,
            &config,
            &public_inputs,
        ),
    }
    .map_err(|error| error.to_string())?;
    let output = MaterializeOutput {
        run_id: materialized.run_id,
        node_mapping: materialized.node_mapping,
    };
    println!(
        "{}",
        serde_json::to_string(&output)
            .map_err(|error| format!("Failed to serialize materialization result: {error}"))?
    );
    Ok(())
}

fn parse_public_inputs(inputs: Vec<String>) -> Result<config::PublicTemplateInputs, String> {
    let mut values = BTreeMap::new();
    for input in inputs {
        let (name, value) = input
            .split_once('=')
            .ok_or_else(|| format!("Invalid --input '{input}': expected NAME=VALUE"))?;
        if name.trim().is_empty() {
            return Err(format!("Invalid --input '{input}': NAME must not be empty"));
        }
        if values.insert(name.to_string(), value.to_string()).is_some() {
            return Err(format!(
                "Invalid --input '{input}': input '{name}' was supplied more than once"
            ));
        }
    }
    Ok(config::PublicTemplateInputs::new(values))
}

fn handle_status(
    root: &Path,
    config_path: Option<&Path>,
    _config_base: &Path,
) -> Result<(), String> {
    let _config = config::load_config_from(config_path, root)?;

    // Load items via Store
    let tg_store_dir = root.join(".task-golem");
    let store = Store::new(tg_store_dir);
    let raw_items = store
        .load_active()
        .map_err(|e| format!("Failed to load task-golem store: {}", e))?;
    let items: Vec<PgItem> = raw_items.into_iter().map(PgItem).collect();

    if items.is_empty() {
        println!("No items in backlog.");
        return Ok(());
    }

    let mut sorted_items: Vec<&PgItem> = items.iter().collect();

    // Sort native lifecycle states, then impact descending within each state.
    sorted_items.sort_by(|a, b| {
        let priority_a = status_sort_priority(&a.status());
        let priority_b = status_sort_priority(&b.status());

        priority_a.cmp(&priority_b).then_with(|| {
            // Within same priority group, sort by impact (high first)
            let impact_a = impact_sort_value(&a.impact());
            let impact_b = impact_sort_value(&b.impact());
            impact_b.cmp(&impact_a)
        })
    });

    // Print header
    println!(
        "{:<12} {:<12} {:<12} {:<10} {:<8} {:<8} {:<8} TITLE",
        "ID", "STATUS", "PHASE", "PIPELINE", "IMPACT", "SIZE", "RISK"
    );
    println!("{}", "-".repeat(94));

    for item in &sorted_items {
        let status_str = item.status().to_string();
        let phase_str = item.phase().unwrap_or_else(|| "-".to_string());
        let pipeline_str = item.pipeline_type().unwrap_or_else(|| "-".to_string());
        let impact_str = display_optional_dimension(item.impact());
        let size_str = display_optional_size(item.size());
        let risk_str = display_optional_dimension(item.risk());

        let title = truncate_title(item.title(), 36);

        println!(
            "{:<12} {:<12} {:<12} {:<10} {:<8} {:<8} {:<8} {}",
            item.id(),
            status_str,
            phase_str,
            pipeline_str,
            impact_str,
            size_str,
            risk_str,
            title
        );
    }

    println!("\n{} item(s) total", items.len());

    Ok(())
}

fn handle_advance(
    root: &Path,
    config_path: Option<&Path>,
    _config_base: &Path,
    item_id: &str,
    to: Option<String>,
) -> Result<(), String> {
    let config = config::load_config_from(config_path, root)?;

    // Use Store directly with with_lock for single-shot CLI command
    let tg_store_dir = root.join(".task-golem");
    let store = Store::new(tg_store_dir);

    store
        .with_lock(|s| {
            let mut items = s.load_active()?;
            let idx = items
                .iter()
                .position(|i| i.id == item_id)
                .ok_or_else(|| task_golem::errors::TgError::ItemNotFound(item_id.to_string()))?;

            let pg = PgItem(items[idx].clone());
            if pg.status() != Status::Doing {
                return Err(task_golem::errors::TgError::InvalidInput(format!(
                    "Cannot advance {}: status is {}, expected doing",
                    item_id,
                    pg.status()
                )));
            }

            let pipeline_type = pg.pipeline_type().unwrap_or_else(|| "feature".to_string());
            let pipeline = config.pipelines.get(&pipeline_type).ok_or_else(|| {
                task_golem::errors::TgError::InvalidInput(format!(
                    "Pipeline type '{}' not found in config",
                    pipeline_type
                ))
            })?;

            match to {
                Some(target_phase) => {
                    // Validate target phase exists in pipeline (main phases only for advance)
                    let is_main_phase = pipeline.phases.iter().any(|p| p.name == target_phase);
                    if !is_main_phase {
                        let valid_names: Vec<&str> =
                            pipeline.phases.iter().map(|p| p.name.as_str()).collect();
                        return Err(task_golem::errors::TgError::InvalidInput(format!(
                            "Invalid phase '{}': expected one of {}",
                            target_phase,
                            valid_names.join(", ")
                        )));
                    }
                    pg_item::set_phase(&mut items[idx], Some(&target_phase));
                    pg_item::set_phase_pool(
                        &mut items[idx],
                        Some(&phase_golem::types::PhasePool::Main),
                    );
                    s.save_active(&items)?;
                    println!("Advanced {} to {}", item_id, target_phase);
                }
                None => {
                    let current_phase = pg.phase().ok_or_else(|| {
                        task_golem::errors::TgError::InvalidInput(format!(
                            "Cannot advance {}: no current phase set",
                            item_id
                        ))
                    })?;
                    let main_phases: Vec<&str> =
                        pipeline.phases.iter().map(|p| p.name.as_str()).collect();
                    let current_idx = main_phases
                        .iter()
                        .position(|&p| p == current_phase)
                        .ok_or_else(|| {
                            task_golem::errors::TgError::InvalidInput(format!(
                                "Current phase '{}' not found in pipeline",
                                current_phase
                            ))
                        })?;
                    let next = main_phases.get(current_idx + 1).ok_or_else(|| {
                        task_golem::errors::TgError::InvalidInput(format!(
                            "Cannot advance {}: '{}' is the final phase",
                            item_id, current_phase
                        ))
                    })?;
                    let prev = pg.phase();
                    pg_item::set_phase(&mut items[idx], Some(next));
                    s.save_active(&items)?;
                    println!(
                        "Advanced {} from {} to {}",
                        item_id,
                        prev.as_deref().unwrap_or("none"),
                        next
                    );
                }
            }

            Ok(())
        })
        .map_err(|e| format!("{}", e))
}

fn handle_unblock(
    root: &Path,
    config_path: Option<&Path>,
    _config_base: &Path,
    item_id: &str,
    notes: Option<String>,
) -> Result<(), String> {
    let _config = config::load_config_from(config_path, root)?;

    // Use Store directly with with_lock for single-shot CLI command
    let tg_store_dir = root.join(".task-golem");
    let store = Store::new(tg_store_dir);

    store
        .with_lock(|s| {
            let mut items = s.load_active()?;
            let idx = items
                .iter()
                .position(|i| i.id == item_id)
                .ok_or_else(|| task_golem::errors::TgError::ItemNotFound(item_id.to_string()))?;

            let pg = PgItem(items[idx].clone());
            if pg.status() != Status::Blocked {
                return Err(task_golem::errors::TgError::InvalidInput(format!(
                    "Cannot unblock {}: status is {}, expected blocked",
                    item_id,
                    pg.status()
                )));
            }

            // Set unblock_context if notes provided
            if let Some(notes_text) = notes {
                pg_item::set_unblock_context(&mut items[idx], Some(&notes_text));
            }

            // Reset last_phase_commit for staleness-blocked items
            pg_item::set_last_phase_commit(&mut items[idx], None);

            let unblock_change = items[idx].apply_unblock();
            s.commit_status_change(&items, unblock_change)?;
            if items[idx].status != Status::Todo {
                let todo_change = items[idx].apply_todo();
                s.commit_status_change(&items, todo_change)?;
            }
            println!("Unblocked {} -- restored to todo", item_id);
            Ok(())
        })
        .map_err(|e| format!("{}", e))
}

// --- Display helpers ---

fn display_optional_dimension(opt: Option<DimensionLevel>) -> String {
    opt.map(|v| format!("{:?}", v).to_lowercase())
        .unwrap_or_else(|| "-".to_string())
}

fn display_optional_size(opt: Option<phase_golem::types::SizeLevel>) -> String {
    opt.map(|v| format!("{:?}", v).to_lowercase())
        .unwrap_or_else(|| "-".to_string())
}

/// Truncate a title for display, respecting UTF-8 character boundaries.
fn truncate_title(title: &str, max_len: usize) -> String {
    if title.len() <= max_len {
        return title.to_string();
    }
    let truncated: String = title.chars().take(max_len - 3).collect();
    format!("{}...", truncated)
}

fn status_sort_priority(status: &Status) -> u8 {
    match status {
        Status::Doing => 0,
        Status::Blocked => 1,
        Status::Todo => 2,
        Status::Done => 3,
    }
}

fn impact_sort_value(impact: &Option<DimensionLevel>) -> u8 {
    match impact {
        Some(DimensionLevel::High) => 3,
        Some(DimensionLevel::Medium) => 2,
        Some(DimensionLevel::Low) => 1,
        None => 0,
    }
}

/// Find a change directory matching an item ID.
///
/// Looks for directories in `changes/` that start with the item ID followed by `_`.
pub fn find_change_dir(changes_dir: &Path, item_id: &str) -> Result<PathBuf, String> {
    let prefix = format!("{}_", item_id);

    if !changes_dir.exists() {
        return Err(format!(
            "Changes directory does not exist: {}",
            changes_dir.display()
        ));
    }

    let entries = fs::read_dir(changes_dir)
        .map_err(|e| format!("Failed to read {}: {}", changes_dir.display(), e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) && entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            return Ok(entry.path());
        }
    }

    Err(format!(
        "No change directory found for item {} in {}",
        item_id,
        changes_dir.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as std_fs;

    const RESULT_1: &str = "phase_result_019b2e7a-0001-7000-8000-000000000001_build.json";
    const RESULT_2: &str = "phase_result_019b2e7a-0002-7000-8000-000000000002_prd.json";
    const NON_JSON_RESULT: &str = "phase_result_019b2e7a-0001-7000-8000-000000000001_build.txt";
    const RESULT_DIRECTORY: &str = "phase_result_019b2e7a-0003-7000-8000-000000000003_test.json";

    #[tokio::test]
    async fn cleanup_deletes_matching_files() {
        let dir = tempfile::tempdir().unwrap();
        std_fs::write(dir.path().join(RESULT_1), "{}").unwrap();
        std_fs::write(dir.path().join(RESULT_2), "{}").unwrap();

        cleanup_stale_result_files(dir.path(), "test").await;

        assert!(!dir.path().join(RESULT_1).exists());
        assert!(!dir.path().join(RESULT_2).exists());
    }

    #[tokio::test]
    async fn cleanup_ignores_non_matching_files() {
        let dir = tempfile::tempdir().unwrap();
        std_fs::write(dir.path().join("phase-golem.lock"), "lock").unwrap();
        std_fs::write(dir.path().join("other.json"), "{}").unwrap();
        std_fs::write(dir.path().join(NON_JSON_RESULT), "{}").unwrap();

        cleanup_stale_result_files(dir.path(), "test").await;

        assert!(dir.path().join("phase-golem.lock").exists());
        assert!(dir.path().join("other.json").exists());
        assert!(dir.path().join(NON_JSON_RESULT).exists());
    }

    #[tokio::test]
    async fn cleanup_handles_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nonexistent");

        cleanup_stale_result_files(&missing, "test").await;
        // Should not panic
    }

    #[tokio::test]
    async fn cleanup_handles_empty_directory() {
        let dir = tempfile::tempdir().unwrap();

        cleanup_stale_result_files(dir.path(), "test").await;
        // Should not panic
    }

    #[tokio::test]
    async fn cleanup_continues_after_partial_failure() {
        let dir = tempfile::tempdir().unwrap();

        // Regular file that should be deleted
        std_fs::write(dir.path().join(RESULT_1), "{}").unwrap();

        // Subdirectory with matching name — remove_file will fail with EISDIR
        std_fs::create_dir(dir.path().join("phase_result_stuck.json")).unwrap();

        // Another regular file that should be deleted
        std_fs::write(dir.path().join(RESULT_2), "{}").unwrap();

        cleanup_stale_result_files(dir.path(), "test").await;

        assert!(!dir.path().join(RESULT_1).exists());
        assert!(!dir.path().join(RESULT_2).exists());
        // Subdirectory should still exist (remove_file failed on it)
        assert!(dir.path().join("phase_result_stuck.json").exists());
    }

    #[tokio::test]
    async fn cleanup_handles_directory_entry_with_matching_name() {
        let dir = tempfile::tempdir().unwrap();

        // Subdirectory with matching name
        std_fs::create_dir(dir.path().join(RESULT_DIRECTORY)).unwrap();

        // Regular matching file
        std_fs::write(dir.path().join(RESULT_1), "{}").unwrap();

        cleanup_stale_result_files(dir.path(), "test").await;

        // Regular file should be deleted
        assert!(!dir.path().join(RESULT_1).exists());
        // Directory should still exist (remove_file can't delete directories)
        assert!(dir.path().join(RESULT_DIRECTORY).exists());
    }

    #[test]
    fn direct_unblock_commits_todo_and_clears_claim() {
        // Arrange
        let directory = tempfile::tempdir().expect("tempdir");
        let task_store_dir = directory.path().join(".task-golem");
        std_fs::create_dir_all(&task_store_dir).expect("create task store");
        let store = Store::new(task_store_dir.clone());
        let mut item = pg_item::new_from_parts(
            "018f2b1c-4d5e-7abc-8123-456789abcdef".to_string(),
            "Blocked item".to_string(),
            Status::Blocked,
            vec![],
            vec![],
        )
        .0;
        item.blocked_from_status = Some(Status::Doing);
        item.blocked_reason = Some("repair required".to_string());
        store.save_active(&[item]).expect("seed blocked item");
        std_fs::write(
            task_store_dir.join("archive.jsonl"),
            "{\"schema_version\":1}\n",
        )
        .expect("initialize archive");

        // Act
        handle_unblock(
            directory.path(),
            None,
            directory.path(),
            "018f2b1c-4d5e-7abc-8123-456789abcdef",
            Some("operator repaired input".to_string()),
        )
        .expect("unblock item");

        // Assert
        let item = store.load_active().expect("load unblocked item").remove(0);
        assert_eq!(item.status, Status::Todo);
        assert!(item.claimed_by.is_none());
        assert!(item.claimed_at.is_none());
        assert_eq!(
            item.extensions["x-pg-unblock-context"],
            "operator repaired input"
        );
        let events = std_fs::read_to_string(store.events_path()).expect("read lifecycle events");
        let statuses = events
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).expect("parse lifecycle event")
                    ["status"]
                    .as_str()
                    .expect("event status")
                    .to_string()
            })
            .collect::<Vec<_>>();
        assert_eq!(statuses, vec!["doing", "todo"]);
    }
}
