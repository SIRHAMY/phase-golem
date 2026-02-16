use std::path::Path;
use std::process::Command;

/// A single entry from `git status --porcelain` output.
///
/// Note: porcelain v1 format uses ASCII for the two-character status code and space separator,
/// so byte-offset slicing at positions 0..2 and 3.. is safe. File paths with special characters
/// may be quoted by git.
#[derive(Debug, Clone, PartialEq)]
pub struct StatusEntry {
    /// Two-character status code (e.g., "M ", "??", "A ")
    pub status_code: String,
    /// The file path
    pub path: String,
}

/// Verify only that a git repository exists in the given directory.
///
/// Does not check working tree cleanliness, branch state, or rebase/merge status.
/// Use this when you only need to confirm git is available (e.g., `init`).
pub fn is_git_repo(repo_dir: Option<&Path>) -> Result<(), String> {
    run_git_command(&["rev-parse", "--git-dir"], repo_dir)
        .map_err(|_| "Not a git repository (or git is not installed)".to_string())?;
    Ok(())
}

/// Verify git preconditions for safe orchestrator operation.
///
/// Checks:
/// - Git repo exists (`git rev-parse --git-dir`)
/// - Working tree is clean (`git status --porcelain` is empty)
/// - Not in detached HEAD or rebase/merge state
pub fn check_preconditions(repo_dir: Option<&Path>) -> Result<(), String> {
    // Verify git repo exists and capture git dir path for later checks
    let git_dir_output = run_git_command(&["rev-parse", "--git-dir"], repo_dir)
        .map_err(|_| "Not a git repository (or git is not installed)".to_string())?;

    // Check for clean working tree
    let status_output = run_git_command(&["status", "--porcelain"], repo_dir)?;
    if !status_output.trim().is_empty() {
        return Err(
            "Working tree is not clean. Commit or stash changes before running the orchestrator."
                .to_string(),
        );
    }

    // Check for detached HEAD
    let head_check = run_git_command(&["symbolic-ref", "--quiet", "HEAD"], repo_dir);
    if head_check.is_err() {
        return Err(
            "Detached HEAD state detected. Check out a branch before running the orchestrator."
                .to_string(),
        );
    }

    // Check for rebase/merge in progress
    let git_dir_path = if let Some(base) = repo_dir {
        base.join(git_dir_output.trim())
    } else {
        std::path::PathBuf::from(git_dir_output.trim())
    };

    if git_dir_path.join("rebase-merge").exists() || git_dir_path.join("rebase-apply").exists() {
        return Err(
            "Rebase in progress. Complete or abort the rebase before running the orchestrator."
                .to_string(),
        );
    }

    if git_dir_path.join("MERGE_HEAD").exists() {
        return Err(
            "Merge in progress. Complete or abort the merge before running the orchestrator."
                .to_string(),
        );
    }

    Ok(())
}

/// Stage specific file paths for commit in a specific repo directory.
///
/// Uses `git add` with explicit paths only (never `-A` or `.`).
pub fn stage_paths(paths: &[&Path], repo_dir: Option<&Path>) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }

    let mut args = vec!["add".to_string(), "--".to_string()];
    for p in paths {
        args.push(
            p.to_str()
                .ok_or_else(|| format!("Path contains invalid UTF-8: {:?}", p))?
                .to_string(),
        );
    }

    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    run_git_command(&args_ref, repo_dir)?;
    Ok(())
}

/// Create a git commit with the given message.
///
/// Returns the commit hash on success. If the commit fails, returns an error
/// (caller treats as phase failure).
pub fn commit(message: &str, repo_dir: Option<&Path>) -> Result<String, String> {
    run_git_command(&["commit", "-m", message], repo_dir)?;
    let hash = run_git_command(&["rev-parse", "HEAD"], repo_dir)?;
    Ok(hash.trim().to_string())
}

/// Parse `git status --porcelain` output into structured entries.
pub fn get_status(repo_dir: Option<&Path>) -> Result<Vec<StatusEntry>, String> {
    let output = run_git_command(&["status", "--porcelain"], repo_dir)?;

    let entries = output
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            if line.len() < 3 {
                // Malformed porcelain output line -- skip
                None
            } else {
                Some(StatusEntry {
                    status_code: line[..2].to_string(),
                    path: line[3..].to_string(),
                })
            }
        })
        .collect();

    Ok(entries)
}

/// Returns the full 40-character SHA of HEAD.
pub fn get_head_sha(project_root: &Path) -> Result<String, String> {
    let output = run_git_command(&["rev-parse", "HEAD"], Some(project_root))?;
    Ok(output.trim().to_string())
}

/// Checks whether `sha` is an ancestor of the current HEAD.
///
/// Uses `git merge-base --is-ancestor`:
/// - Exit 0 → true (sha is an ancestor of HEAD)
/// - Exit 1 → false (sha is not an ancestor)
/// - Exit 128 → Err (unknown commit / other git error)
pub fn is_ancestor(sha: &str, project_root: &Path) -> Result<bool, String> {
    if sha.is_empty() || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("Invalid SHA: '{}'", sha));
    }

    let mut cmd = Command::new("git");
    cmd.args(["merge-base", "--is-ancestor", sha, "HEAD"]);
    cmd.current_dir(project_root);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run git merge-base: {}", e))?;

    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        Some(128) | None => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("git merge-base failed: {}", stderr.trim()))
        }
        Some(code) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!(
                "git merge-base exited with unexpected code {}: {}",
                code,
                stderr.trim()
            ))
        }
    }
}

/// Run a git command and return its stdout as a string.
fn run_git_command(args: &[&str], repo_dir: Option<&Path>) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.args(args);

    if let Some(dir) = repo_dir {
        cmd.current_dir(dir);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run git {}: {}", args.first().unwrap_or(&""), e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            stderr.trim()
        ));
    }

    String::from_utf8(output.stdout).map_err(|e| format!("git output is not valid UTF-8: {}", e))
}
