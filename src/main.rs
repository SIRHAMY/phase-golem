use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tokio_util::sync::CancellationToken;

use phase_golem::agent::{
    install_signal_handlers, is_shutdown_requested, kill_all_children, AgentRunner, CliAgentRunner,
};
use phase_golem::backlog;
use phase_golem::config;
use phase_golem::coordinator;
use phase_golem::filter;
use phase_golem::lock;
use phase_golem::log::parse_log_level;
use phase_golem::preflight;
use phase_golem::prompt;
use phase_golem::scheduler;
use phase_golem::types::{parse_dimension_level, parse_size_level, DimensionLevel, ItemStatus};
use phase_golem::{log_error, log_info, log_warn};

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
    Init {
        /// Project prefix for item IDs (e.g., WRK)
        #[arg(long, default_value = "WRK")]
        prefix: String,
    },
    /// Run the phase-golem pipeline
    Run {
        /// Target specific backlog items by ID (can be specified multiple times for sequential processing)
        #[arg(long, action = clap::ArgAction::Append)]
        target: Vec<String>,
        /// Filter items by attribute (e.g., impact=high, status=ready)
        #[arg(long, conflicts_with = "target")]
        only: Option<String>,
        /// Maximum number of phase executions
        #[arg(long, default_value = "100")]
        cap: u32,
    },
    /// Show backlog status
    Status,
    /// Add a new item to the backlog
    Add {
        /// Item title
        title: String,
        /// Size estimate (small, medium, large)
        #[arg(short, long)]
        size: Option<String>,
        /// Risk level (low, medium, high)
        #[arg(short, long)]
        risk: Option<String>,
        /// Pipeline type hint (e.g., feature, blog-post)
        #[arg(short, long)]
        pipeline: Option<String>,
    },
    /// Triage new backlog items
    Triage,
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
        Commands::Init { prefix } => handle_init(root, &prefix),
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
        Commands::Status => handle_status(root, config_path.as_deref(), &config_base),
        Commands::Add {
            title,
            size,
            risk,
            pipeline,
        } => handle_add(
            root,
            config_path.as_deref(),
            &config_base,
            &title,
            size,
            risk,
            pipeline,
        ),
        Commands::Triage => handle_triage(root, config_path.as_deref(), &config_base).await,
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

fn resolve_backlog_path(config_base: &Path, config: &config::PhaseGolemConfig) -> PathBuf {
    config_base.join(&config.project.backlog_path)
}

fn resolve_inbox_path(config_base: &Path, config: &config::PhaseGolemConfig) -> PathBuf {
    let backlog = resolve_backlog_path(config_base, config);
    backlog
        .parent()
        .unwrap_or(config_base)
        .join("BACKLOG_INBOX.yaml")
}

fn handle_init(root: &Path, prefix: &str) -> Result<(), String> {
    // Validate prefix contains only safe characters for TOML and filenames
    if !prefix
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(
            "Prefix must contain only alphanumeric characters, hyphens, and underscores"
                .to_string(),
        );
    }

    // Only check that a git repo exists -- init doesn't require clean tree or branch
    phase_golem::git::is_git_repo(None)
        .map_err(|_| "Not a git repository. Run `git init` first.".to_string())?;

    // Create directories
    let dirs = ["_ideas", "_worklog", "changes", ".phase-golem"];
    for dir in &dirs {
        let dir_path = root.join(dir);
        fs::create_dir_all(&dir_path)
            .map_err(|e| format!("Failed to create {}: {}", dir_path.display(), e))?;
    }

    // Create BACKLOG.yaml if it doesn't exist (uses default path; config doesn't exist yet)
    let backlog = root.join("BACKLOG.yaml");
    if !backlog.exists() {
        let empty_backlog = phase_golem::types::BacklogFile {
            schema_version: 3,
            items: Vec::new(),
            next_item_id: 0,
        };
        phase_golem::backlog::save(&backlog, &empty_backlog)?;
    }

    // Create phase-golem.toml if it doesn't exist (with default pipelines section)
    let config_path = root.join("phase-golem.toml");
    if !config_path.exists() {
        let config_contents = format!(
            r#"[project]
prefix = "{prefix}"
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
"#,
            prefix = prefix
        );
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
    println!("  Created: BACKLOG.yaml, phase-golem.toml");
    println!("  Updated: .gitignore");

    Ok(())
}

async fn handle_run(
    root: &Path,
    config_path: Option<&Path>,
    config_base: &Path,
    target: Vec<String>,
    only: Option<String>,
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
    log_info!("[pre] Checking git preconditions...");
    phase_golem::git::check_preconditions(Some(root))?;

    // Load
    let config = config::load_config_from(config_path, root)?;

    // Construct runner from config and verify CLI
    let runner = CliAgentRunner::new(config.agent.cli.clone(), config.agent.model.clone());
    log_info!("[pre] Verifying Claude CLI...");
    runner.verify_cli_available()?;
    let backlog_file_path = resolve_backlog_path(config_base, &config);
    let mut backlog = backlog::load(&backlog_file_path, root)?;

    // Mutual exclusivity safety net (clap conflicts_with should handle this)
    if !target.is_empty() && only.is_some() {
        return Err("Cannot combine --target and --only flags. Use one or the other.".to_string());
    }

    // Target validation
    let target: Vec<String> = target.iter().map(|t| t.trim().to_string()).collect();
    if !target.is_empty() {
        let mut errors = Vec::new();

        // Format validation
        let prefix = &config.project.prefix;
        let pattern = format!("{}-", prefix);
        for t in &target {
            if !t.starts_with(&pattern) {
                errors.push(format!(
                    "Invalid target format '{}': expected {}-<number>",
                    t, prefix
                ));
            } else {
                let suffix = &t[pattern.len()..];
                if suffix.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
                    errors.push(format!(
                        "Invalid target format '{}': expected {}-<number>",
                        t, prefix
                    ));
                }
            }
        }

        // Existence validation
        for t in &target {
            if !backlog.items.iter().any(|i| i.id == *t) {
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
    let parsed_filter = match only {
        Some(ref raw) => Some(filter::parse_filter(raw)?),
        None => None,
    };

    // Config summary
    log_info!("");
    log_info!("[config] Prefix: {}", config.project.prefix);
    log_info!(
        "[config] Guardrails: max_size={}, max_complexity={}, max_risk={}",
        format!("{:?}", config.guardrails.max_size).to_lowercase(),
        format!("{:?}", config.guardrails.max_complexity).to_lowercase(),
        format!("{:?}", config.guardrails.max_risk).to_lowercase(),
    );
    log_info!(
        "[config] Execution: max_wip={}, max_concurrent={}, timeout={}min, retries={}",
        config.execution.max_wip,
        config.execution.max_concurrent,
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
    if let Some(ref criterion) = parsed_filter {
        let matching = filter::apply_filter(criterion, &backlog);
        log_info!(
            "[config] Filter: {} — {} items match (from {} total)",
            criterion,
            matching.items.len(),
            backlog.items.len()
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
            "[pipeline:{}] pre: [{}] → main: [{}]",
            name,
            pre_names.join(" → "),
            main_names.join(" → "),
        );
    }

    // Backlog summary
    use phase_golem::types::ItemStatus;
    let new_count = backlog
        .items
        .iter()
        .filter(|i| i.status == ItemStatus::New)
        .count();
    let scoping_count = backlog
        .items
        .iter()
        .filter(|i| i.status == ItemStatus::Scoping)
        .count();
    let ready_count = backlog
        .items
        .iter()
        .filter(|i| i.status == ItemStatus::Ready)
        .count();
    let in_progress_count = backlog
        .items
        .iter()
        .filter(|i| i.status == ItemStatus::InProgress)
        .count();
    let blocked_count = backlog
        .items
        .iter()
        .filter(|i| i.status == ItemStatus::Blocked)
        .count();
    let done_count = backlog
        .items
        .iter()
        .filter(|i| i.status == ItemStatus::Done)
        .count();
    log_info!("");
    log_info!(
        "[backlog] {} items: {} new, {} scoping, {} ready, {} in-progress, {} blocked, {} done",
        backlog.items.len(),
        new_count,
        scoping_count,
        ready_count,
        in_progress_count,
        blocked_count,
        done_count,
    );

    // Queue preview — show first few actionable items
    let actionable: Vec<&phase_golem::types::BacklogItem> = backlog
        .items
        .iter()
        .filter(|i| {
            matches!(
                i.status,
                ItemStatus::New | ItemStatus::Scoping | ItemStatus::Ready | ItemStatus::InProgress
            )
        })
        .take(MAX_BACKLOG_PREVIEW_ITEMS)
        .collect();
    if !actionable.is_empty() {
        log_info!("[backlog] Next up:");
        for item in &actionable {
            let phase_info = item.phase.as_deref().unwrap_or("-");
            log_info!(
                "  {} ({:?}) phase={} — {}",
                item.id,
                item.status,
                phase_info,
                item.title
            );
        }
        if backlog
            .items
            .iter()
            .filter(|i| {
                matches!(
                    i.status,
                    ItemStatus::New
                        | ItemStatus::Scoping
                        | ItemStatus::Ready
                        | ItemStatus::InProgress
                )
            })
            .count()
            > MAX_BACKLOG_PREVIEW_ITEMS
        {
            log_info!("  ...");
        }
    }

    // Prune stale dependencies (safety net for archived items)
    let pruned_count = backlog::prune_stale_dependencies(&mut backlog);
    if pruned_count > 0 {
        log_warn!(
            "[pre] Pruned {} stale dependency reference(s) from backlog",
            pruned_count
        );
        backlog::save(&backlog_file_path, &backlog).map_err(|e| {
            format!(
                "Failed to save backlog after pruning stale dependencies: {}",
                e
            )
        })?;
    }

    // Preflight
    log_info!("");
    log_info!("[pre] Running preflight checks...");
    if let Err(errors) = preflight::run_preflight(&config, &backlog, root, config_base) {
        log_error!("[pre] Preflight FAILED:");
        for error in &errors {
            log_error!("  {}", error);
        }
        return Err(format!(
            "{} preflight error(s) — fix all issues before running",
            errors.len()
        ));
    }
    log_info!("[pre] Preflight passed.");

    // Validate inbox file early (fail fast instead of warning mid-run)
    let inbox_path = resolve_inbox_path(config_base, &config);
    if inbox_path.exists() {
        backlog::load_inbox(&inbox_path).map_err(|e| format!("Inbox validation failed: {}", e))?;
    }

    let runner = Arc::new(runner);
    log_info!("");
    let (coord_handle, coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_file_path.clone(),
        inbox_path,
        root.to_path_buf(),
        config.project.prefix.clone(),
    );

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

    let filter_display = parsed_filter.as_ref().map(|c| c.to_string());

    let params = scheduler::RunParams {
        targets: target,
        filter: parsed_filter,
        cap,
        root: root.to_path_buf(),
        config_base: config_base.to_path_buf(),
    };

    let summary = scheduler::run_scheduler(coord_handle, runner, config, params, cancel).await?;

    // Kill any remaining child processes
    kill_all_children();

    // Await coordinator shutdown (ensures save_backlog() completes)
    if let Err(err) = coord_task.await {
        log_warn!(
            "Coordinator task panicked, skipping backlog commit: {:?}",
            err
        );
    } else {
        // Commit BACKLOG.yaml if it has uncommitted changes
        let root_for_commit = root.to_path_buf();
        let backlog_path_for_commit = backlog_file_path.clone();
        let halt_reason_display = format!("{:?}", summary.halt_reason);

        let commit_result = tokio::task::spawn_blocking(move || {
            let status = match phase_golem::git::get_status(Some(&root_for_commit)) {
                Ok(s) => s,
                Err(err) => {
                    return Err(format!("get_status failed: {}", err));
                }
            };

            let backlog_rel = backlog_path_for_commit
                .strip_prefix(&root_for_commit)
                .unwrap_or(&backlog_path_for_commit)
                .to_string_lossy();
            let is_backlog_dirty = status
                .iter()
                .any(|entry| entry.path.trim_matches('"') == backlog_rel.as_ref());

            if !is_backlog_dirty {
                return Ok(None);
            }

            if let Err(err) = phase_golem::git::stage_paths(
                &[backlog_path_for_commit.as_path()],
                Some(&root_for_commit),
            ) {
                return Err(format!("stage_paths failed: {}", err));
            }

            let message = format!(
                "[phase-golem] Save backlog state on halt ({})",
                halt_reason_display
            );
            match phase_golem::git::commit(&message, Some(&root_for_commit)) {
                Ok(sha) => Ok(Some(sha)),
                Err(err) => Err(format!("commit failed: {}", err)),
            }
        })
        .await;

        match commit_result {
            Ok(Ok(Some(sha))) => {
                log_info!("Committed backlog state on halt: {}", sha);
            }
            Ok(Ok(None)) => {
                // BACKLOG.yaml was clean, nothing to commit
            }
            Ok(Err(err)) => {
                log_warn!("Shutdown backlog commit skipped: {}", err);
            }
            Err(err) => {
                log_warn!("spawn_blocking panicked during shutdown commit: {:?}", err);
            }
        }
    }

    // Print summary
    log_info!("\n--- Run Summary ---");
    log_info!("Phases executed: {}", summary.phases_executed);
    if !summary.items_completed.is_empty() {
        log_info!("Items completed: {}", summary.items_completed.join(", "));
    }
    if !summary.items_blocked.is_empty() {
        log_info!("Items blocked: {}", summary.items_blocked.join(", "));
    }
    if summary.follow_ups_created > 0 {
        log_info!("Follow-ups created: {}", summary.follow_ups_created);
    }
    if summary.items_merged > 0 {
        log_info!("Items merged: {}", summary.items_merged);
    }
    match &summary.halt_reason {
        scheduler::HaltReason::FilterExhausted => {
            if let Some(ref filter_str) = filter_display {
                log_info!(
                    "Filter: all items matching {} are done or blocked",
                    filter_str
                );
            }
        }
        scheduler::HaltReason::NoMatchingItems => {
            if let Some(ref filter_str) = filter_display {
                log_info!("Filter: no items match {}", filter_str);
            }
        }
        _ => {}
    }
    log_info!("Halt reason: {:?}", summary.halt_reason);

    Ok(())
}

async fn handle_triage(
    root: &Path,
    config_path: Option<&Path>,
    config_base: &Path,
) -> Result<(), String> {
    // Install signal handlers for graceful shutdown
    install_signal_handlers()?;

    // Acquire lock
    let runtime_dir = root.join(".phase-golem");
    let _lock = lock::try_acquire(&runtime_dir)?;

    // Check git preconditions
    phase_golem::git::check_preconditions(Some(root))?;

    // Load config and backlog
    let config = config::load_config_from(config_path, root)?;
    let backlog_file_path = resolve_backlog_path(config_base, &config);
    let backlog = backlog::load(&backlog_file_path, root)?;

    // Construct runner from config and verify CLI
    let runner = CliAgentRunner::new(config.agent.cli.clone(), config.agent.model.clone());
    runner.verify_cli_available()?;

    // Validate inbox file early (fail fast instead of warning mid-run)
    let inbox_path = resolve_inbox_path(config_base, &config);
    if inbox_path.exists() {
        backlog::load_inbox(&inbox_path).map_err(|e| format!("Inbox validation failed: {}", e))?;
    }

    // Spawn coordinator
    let (coordinator_handle, _coord_task) = coordinator::spawn_coordinator(
        backlog,
        backlog_file_path,
        inbox_path,
        root.to_path_buf(),
        config.project.prefix.clone(),
    );

    // Find New items to triage
    let snapshot = coordinator_handle.get_snapshot().await?;
    let new_item_ids: Vec<String> = snapshot
        .items
        .iter()
        .filter(|item| item.status == ItemStatus::New)
        .map(|item| item.id.clone())
        .collect();

    let timeout =
        std::time::Duration::from_secs(config.execution.phase_timeout_minutes as u64 * 60);
    let mut triaged_count = 0u32;

    for item_id in &new_item_ids {
        if is_shutdown_requested() {
            break;
        }

        log_info!("[{}][TRIAGE] Starting triage", item_id);

        let result_path = phase_golem::executor::result_file_path(root, item_id, "triage");
        let current_snapshot = coordinator_handle.get_snapshot().await?;
        let item = current_snapshot
            .items
            .iter()
            .find(|i| i.id == *item_id)
            .ok_or_else(|| format!("Item {} not found", item_id))?;

        let backlog_summary = prompt::build_backlog_summary(&current_snapshot.items, item_id);
        let triage_prompt = prompt::build_triage_prompt(
            item,
            &result_path,
            &config.pipelines,
            backlog_summary.as_deref(),
        );

        match runner
            .run_agent(&triage_prompt, &result_path, timeout)
            .await
        {
            Ok(phase_result) => {
                // Stage and commit triage output (immediate commit via destructive flag)
                coordinator_handle
                    .complete_phase(item_id, phase_result.clone(), true)
                    .await?;

                // Apply triage routing
                scheduler::apply_triage_result(
                    &coordinator_handle,
                    item_id,
                    &phase_result,
                    &config,
                )
                .await?;

                log_info!(
                    "[{}][TRIAGE] Result: {:?} — {}",
                    item_id,
                    phase_result.result,
                    phase_result.summary
                );
                triaged_count += 1;
            }
            Err(e) => {
                log_error!("[{}][TRIAGE] Failed: {}", item_id, e);
            }
        }
    }

    // Shutdown coordinator and clean up
    drop(coordinator_handle);
    kill_all_children();

    log_info!("Triaged {} item(s)", triaged_count);

    Ok(())
}

fn handle_add(
    root: &Path,
    config_path: Option<&Path>,
    config_base: &Path,
    title: &str,
    size: Option<String>,
    risk: Option<String>,
    pipeline_type: Option<String>,
) -> Result<(), String> {
    let config = config::load_config_from(config_path, root)?;
    let backlog_file_path = resolve_backlog_path(config_base, &config);
    let mut backlog = backlog::load(&backlog_file_path, root)?;

    let parsed_size = size.map(|s| parse_size_level(&s)).transpose()?;
    let parsed_risk = risk.map(|r| parse_dimension_level(&r)).transpose()?;

    let item = backlog::add_item(
        &mut backlog,
        title,
        parsed_size,
        parsed_risk,
        &config.project.prefix,
    );

    // Set pipeline type hint if provided
    if let Some(ref pt) = pipeline_type {
        if let Some(item_mut) = backlog.items.iter_mut().find(|i| i.id == item.id) {
            item_mut.pipeline_type = Some(pt.clone());
        }
    }

    backlog::save(&backlog_file_path, &backlog)?;

    let pipeline_info = pipeline_type
        .map(|pt| format!(" (pipeline: {})", pt))
        .unwrap_or_default();
    println!("Added {} — {}{}", item.id, item.title, pipeline_info);

    Ok(())
}

fn handle_status(
    root: &Path,
    config_path: Option<&Path>,
    config_base: &Path,
) -> Result<(), String> {
    let config = config::load_config_from(config_path, root)?;
    let backlog_file_path = resolve_backlog_path(config_base, &config);
    let backlog = backlog::load(&backlog_file_path, root)?;

    if backlog.items.is_empty() {
        println!("No items in backlog.");
        return Ok(());
    }

    let mut items: Vec<&phase_golem::types::BacklogItem> = backlog.items.iter().collect();

    // Sort: in_progress first, then blocked, ready by impact desc, then scoping, new
    items.sort_by(|a, b| {
        let priority_a = status_sort_priority(&a.status);
        let priority_b = status_sort_priority(&b.status);

        priority_a.cmp(&priority_b).then_with(|| {
            // Within same priority group, sort by impact (high first)
            let impact_a = impact_sort_value(&a.impact);
            let impact_b = impact_sort_value(&b.impact);
            impact_b.cmp(&impact_a)
        })
    });

    // Print header
    println!(
        "{:<12} {:<12} {:<12} {:<10} {:<8} {:<8} {:<8} TITLE",
        "ID", "STATUS", "PHASE", "PIPELINE", "IMPACT", "SIZE", "RISK"
    );
    println!("{}", "-".repeat(94));

    for item in &items {
        let status_str = format!("{:?}", item.status).to_lowercase();
        let phase_str = item.phase.as_deref().unwrap_or("-");
        let pipeline_str = item.pipeline_type.as_deref().unwrap_or("-");
        let impact_str = display_optional(item.impact.as_ref());
        let size_str = display_optional(item.size.as_ref());
        let risk_str = display_optional(item.risk.as_ref());

        let title = truncate_title(&item.title, 36);

        println!(
            "{:<12} {:<12} {:<12} {:<10} {:<8} {:<8} {:<8} {}",
            item.id, status_str, phase_str, pipeline_str, impact_str, size_str, risk_str, title
        );
    }

    println!("\n{} item(s) total", backlog.items.len());

    Ok(())
}

fn handle_advance(
    root: &Path,
    config_path: Option<&Path>,
    config_base: &Path,
    item_id: &str,
    to: Option<String>,
) -> Result<(), String> {
    let config = config::load_config_from(config_path, root)?;
    let backlog_file_path = resolve_backlog_path(config_base, &config);
    let mut backlog = backlog::load(&backlog_file_path, root)?;

    let item = backlog
        .items
        .iter_mut()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("Item {} not found in backlog", item_id))?;

    if item.status != ItemStatus::InProgress {
        return Err(format!(
            "Cannot advance {}: status is {:?}, expected InProgress",
            item_id, item.status
        ));
    }

    let pipeline_type = item.pipeline_type.as_deref().unwrap_or("feature");
    let pipeline = config
        .pipelines
        .get(pipeline_type)
        .ok_or_else(|| format!("Pipeline type '{}' not found in config", pipeline_type))?;

    match to {
        Some(target_phase) => {
            // Validate target phase exists in pipeline (main phases only for advance)
            let is_main_phase = pipeline.phases.iter().any(|p| p.name == target_phase);
            if !is_main_phase {
                let valid_names: Vec<&str> =
                    pipeline.phases.iter().map(|p| p.name.as_str()).collect();
                return Err(format!(
                    "Invalid phase '{}': expected one of {}",
                    target_phase,
                    valid_names.join(", ")
                ));
            }
            item.phase = Some(target_phase.clone());
            item.phase_pool = Some(phase_golem::types::PhasePool::Main);
            item.updated = chrono::Utc::now().to_rfc3339();
            println!("Advanced {} to {}", item_id, target_phase);
        }
        None => {
            let current_phase = item
                .phase
                .as_deref()
                .ok_or_else(|| format!("Cannot advance {}: no current phase set", item_id))?;
            let main_phases: Vec<&str> = pipeline.phases.iter().map(|p| p.name.as_str()).collect();
            let current_idx = main_phases
                .iter()
                .position(|&p| p == current_phase)
                .ok_or_else(|| {
                    format!("Current phase '{}' not found in pipeline", current_phase)
                })?;
            let next = main_phases.get(current_idx + 1).ok_or_else(|| {
                format!(
                    "Cannot advance {}: '{}' is the final phase",
                    item_id, current_phase
                )
            })?;
            let prev = item.phase.clone();
            item.phase = Some(next.to_string());
            item.updated = chrono::Utc::now().to_rfc3339();
            println!(
                "Advanced {} from {} to {}",
                item_id,
                prev.as_deref().unwrap_or("none"),
                next
            );
        }
    }

    backlog::save(&backlog_file_path, &backlog)?;
    Ok(())
}

fn handle_unblock(
    root: &Path,
    config_path: Option<&Path>,
    config_base: &Path,
    item_id: &str,
    notes: Option<String>,
) -> Result<(), String> {
    let config = config::load_config_from(config_path, root)?;
    let backlog_file_path = resolve_backlog_path(config_base, &config);
    let mut backlog = backlog::load(&backlog_file_path, root)?;

    let item = backlog
        .items
        .iter_mut()
        .find(|i| i.id == item_id)
        .ok_or_else(|| format!("Item {} not found in backlog", item_id))?;

    if item.status != ItemStatus::Blocked {
        return Err(format!(
            "Cannot unblock {}: status is {:?}, expected Blocked",
            item_id, item.status
        ));
    }

    let restore_status = item
        .blocked_from_status
        .clone()
        .ok_or_else(|| format!("Item {} is blocked but has no blocked_from_status", item_id))?;

    backlog::transition_status(item, restore_status.clone())?;

    // Set unblock_context after transition (transition clears blocked fields)
    if let Some(notes_text) = notes {
        item.unblock_context = Some(notes_text);
    }

    // Reset last_phase_commit for staleness-blocked items
    item.last_phase_commit = None;

    backlog::save(&backlog_file_path, &backlog)?;

    println!("Unblocked {} — restored to {:?}", item_id, restore_status);

    Ok(())
}

// --- Display helpers ---

fn display_optional<T: std::fmt::Debug>(opt: Option<&T>) -> String {
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

fn status_sort_priority(status: &ItemStatus) -> u8 {
    match status {
        ItemStatus::InProgress => 0,
        ItemStatus::Blocked => 1,
        ItemStatus::Ready => 2,
        ItemStatus::Scoping => 3,
        ItemStatus::New => 4,
        ItemStatus::Done => 5,
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
