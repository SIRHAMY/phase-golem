use std::fs;
use std::process::Command;

use tempfile::TempDir;

/// Create a temporary git repository for testing.
fn setup_temp_repo() -> TempDir {
    let dir = TempDir::new().expect("Failed to create temp dir");

    Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .expect("Failed to init git repo");

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(dir.path())
        .output()
        .expect("Failed to set git email");

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(dir.path())
        .output()
        .expect("Failed to set git name");

    // Create an initial commit so HEAD exists
    let readme = dir.path().join("README.md");
    fs::write(&readme, "# Test\n").expect("Failed to write README");

    Command::new("git")
        .args(["add", "README.md"])
        .current_dir(dir.path())
        .output()
        .expect("Failed to stage README");

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(dir.path())
        .output()
        .expect("Failed to create initial commit");

    dir
}

#[test]
fn check_preconditions_clean_repo() {
    let repo = setup_temp_repo();
    let result = orchestrate::git::check_preconditions(Some(repo.path()));
    assert!(result.is_ok(), "Expected Ok, got: {:?}", result);
}

#[test]
fn check_preconditions_dirty_tree_fails() {
    let repo = setup_temp_repo();

    // Create an untracked file to dirty the tree
    fs::write(repo.path().join("dirty.txt"), "dirty").expect("Failed to write file");

    let result = orchestrate::git::check_preconditions(Some(repo.path()));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("not clean"),
        "Expected 'not clean' in: {}",
        err
    );
}

#[test]
fn check_preconditions_detached_head_fails() {
    let repo = setup_temp_repo();

    // Detach HEAD by checking out a commit hash
    let hash = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to get HEAD");
    let hash = String::from_utf8(hash.stdout).unwrap();

    Command::new("git")
        .args(["checkout", hash.trim()])
        .current_dir(repo.path())
        .output()
        .expect("Failed to detach HEAD");

    let result = orchestrate::git::check_preconditions(Some(repo.path()));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("Detached HEAD"),
        "Expected 'Detached HEAD' in: {}",
        err
    );
}

#[test]
fn check_preconditions_not_git_repo_fails() {
    let dir = TempDir::new().expect("Failed to create temp dir");

    let result = orchestrate::git::check_preconditions(Some(dir.path()));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("Not a git repository"),
        "Expected 'Not a git repository' in: {}",
        err
    );
}

#[test]
fn check_preconditions_rebase_in_progress_fails() {
    let repo = setup_temp_repo();

    // Simulate rebase by creating the sentinel directory
    let rebase_dir = repo.path().join(".git/rebase-merge");
    fs::create_dir_all(&rebase_dir).expect("Failed to create rebase-merge dir");

    let result = orchestrate::git::check_preconditions(Some(repo.path()));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("Rebase in progress"),
        "Expected 'Rebase in progress' in: {}",
        err
    );
}

#[test]
fn check_preconditions_merge_in_progress_fails() {
    let repo = setup_temp_repo();

    // Simulate merge by creating the MERGE_HEAD sentinel file
    let merge_head = repo.path().join(".git/MERGE_HEAD");
    fs::write(&merge_head, "abc123").expect("Failed to create MERGE_HEAD");

    let result = orchestrate::git::check_preconditions(Some(repo.path()));
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("Merge in progress"),
        "Expected 'Merge in progress' in: {}",
        err
    );
}

#[test]
fn is_git_repo_valid() {
    let repo = setup_temp_repo();
    let result = orchestrate::git::is_git_repo(Some(repo.path()));
    assert!(result.is_ok(), "Expected Ok for valid git repo");
}

#[test]
fn is_git_repo_not_a_repo() {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let result = orchestrate::git::is_git_repo(Some(dir.path()));
    assert!(result.is_err(), "Expected Err for non-git directory");
}

#[test]
fn stage_and_commit() {
    let repo = setup_temp_repo();

    let new_file = repo.path().join("test.txt");
    fs::write(&new_file, "hello").expect("Failed to write file");

    orchestrate::git::stage_paths(&[new_file.as_path()], Some(repo.path()))
        .expect("Failed to stage");
    let hash =
        orchestrate::git::commit("Test commit", Some(repo.path())).expect("Failed to commit");
    assert!(!hash.is_empty(), "Commit hash should not be empty");
    assert_eq!(hash.len(), 40, "Commit hash should be 40 chars");
}

#[test]
fn stage_empty_paths_is_ok() {
    let result = orchestrate::git::stage_paths(&[], None);
    assert!(result.is_ok());
}

#[test]
fn get_status_clean() {
    let repo = setup_temp_repo();
    let entries = orchestrate::git::get_status(Some(repo.path())).expect("Failed to get status");
    assert!(entries.is_empty(), "Expected empty status for clean repo");
}

#[test]
fn get_status_with_changes() {
    let repo = setup_temp_repo();

    // Create an untracked file
    fs::write(repo.path().join("new.txt"), "new").expect("Failed to write file");

    // Modify an existing file
    fs::write(repo.path().join("README.md"), "# Modified\n").expect("Failed to modify file");

    let entries = orchestrate::git::get_status(Some(repo.path())).expect("Failed to get status");
    assert!(
        entries.len() >= 2,
        "Expected at least 2 status entries, got {}",
        entries.len()
    );

    let paths: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    assert!(paths.contains(&"new.txt"), "Expected new.txt in status");
    assert!(paths.contains(&"README.md"), "Expected README.md in status");
}

#[test]
fn commit_fails_with_nothing_to_commit() {
    let repo = setup_temp_repo();
    let result = orchestrate::git::commit("Empty commit", Some(repo.path()));
    assert!(result.is_err(), "Expected error for empty commit");
}

#[test]
fn get_status_parses_status_codes() {
    let repo = setup_temp_repo();

    // Create a new file and stage it
    fs::write(repo.path().join("staged.txt"), "staged").expect("Failed to write file");
    Command::new("git")
        .args(["add", "staged.txt"])
        .current_dir(repo.path())
        .output()
        .expect("Failed to stage file");

    // Create another untracked file
    fs::write(repo.path().join("untracked.txt"), "untracked").expect("Failed to write file");

    let entries = orchestrate::git::get_status(Some(repo.path())).expect("Failed to get status");

    let staged = entries.iter().find(|e| e.path == "staged.txt");
    assert!(staged.is_some(), "Expected staged.txt in status");
    assert_eq!(
        staged.unwrap().status_code,
        "A ",
        "Expected 'A ' status for staged file"
    );

    let untracked = entries.iter().find(|e| e.path == "untracked.txt");
    assert!(untracked.is_some(), "Expected untracked.txt in status");
    assert_eq!(
        untracked.unwrap().status_code,
        "??",
        "Expected '??' status for untracked file"
    );
}

// --- get_head_sha tests ---

#[test]
fn get_head_sha_returns_40_char_sha() {
    let repo = setup_temp_repo();
    let sha = orchestrate::git::get_head_sha(repo.path()).expect("Failed to get HEAD SHA");
    assert_eq!(sha.len(), 40, "SHA should be 40 characters, got: {}", sha);
    assert!(
        sha.chars().all(|c| c.is_ascii_hexdigit()),
        "SHA should be hex characters, got: {}",
        sha
    );
}

#[test]
fn get_head_sha_changes_after_commit() {
    let repo = setup_temp_repo();
    let sha1 = orchestrate::git::get_head_sha(repo.path()).unwrap();

    fs::write(repo.path().join("new.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "new.txt"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Second commit"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let sha2 = orchestrate::git::get_head_sha(repo.path()).unwrap();
    assert_ne!(sha1, sha2, "SHA should change after commit");
}

// --- is_ancestor tests ---

#[test]
fn is_ancestor_returns_true_for_ancestor() {
    let repo = setup_temp_repo();
    let sha1 = orchestrate::git::get_head_sha(repo.path()).unwrap();

    // Create a second commit
    fs::write(repo.path().join("new.txt"), "content").unwrap();
    Command::new("git")
        .args(["add", "new.txt"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Second commit"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let result =
        orchestrate::git::is_ancestor(&sha1, repo.path()).expect("is_ancestor should succeed");
    assert!(
        result,
        "First commit should be ancestor of HEAD after second commit"
    );
}

#[test]
fn is_ancestor_returns_false_for_non_ancestor() {
    let repo = setup_temp_repo();

    // Get the current branch name (master or main)
    let branch_output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    let default_branch = String::from_utf8(branch_output.stdout)
        .unwrap()
        .trim()
        .to_string();

    // Create branch-a with a commit
    Command::new("git")
        .args(["checkout", "-b", "branch-a"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    fs::write(repo.path().join("a.txt"), "branch a").unwrap();
    Command::new("git")
        .args(["add", "a.txt"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Branch A commit"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    let sha_a = orchestrate::git::get_head_sha(repo.path()).unwrap();

    // Go back to default branch, create branch-b with a different commit
    Command::new("git")
        .args(["checkout", &default_branch])
        .current_dir(repo.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["checkout", "-b", "branch-b"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    fs::write(repo.path().join("b.txt"), "branch b").unwrap();
    Command::new("git")
        .args(["add", "b.txt"])
        .current_dir(repo.path())
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "Branch B commit"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    // sha_a (branch-a tip) is NOT an ancestor of branch-b HEAD
    let result =
        orchestrate::git::is_ancestor(&sha_a, repo.path()).expect("is_ancestor should succeed");
    assert!(
        !result,
        "Branch A commit should not be ancestor of Branch B HEAD"
    );
}

#[test]
fn is_ancestor_unknown_commit_returns_error() {
    let repo = setup_temp_repo();
    let fake_sha = "0000000000000000000000000000000000000000";

    let result = orchestrate::git::is_ancestor(fake_sha, repo.path());
    assert!(
        result.is_err(),
        "Unknown commit should return error, got: {:?}",
        result
    );
}
