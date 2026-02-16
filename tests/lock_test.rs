use orchestrate::lock;

#[test]
fn lock_acquire_and_release() {
    let dir = tempfile::tempdir().unwrap();
    let orch_dir = dir.path().join(".orchestrator");

    let guard = lock::try_acquire(&orch_dir).unwrap();

    // PID file should exist with our PID
    let pid_contents = std::fs::read_to_string(orch_dir.join("orchestrator.pid")).unwrap();
    assert_eq!(
        pid_contents.trim().parse::<u32>().unwrap(),
        std::process::id()
    );

    // Drop releases the lock
    drop(guard);

    // PID file should be removed
    assert!(!orch_dir.join("orchestrator.pid").exists());
}

#[test]
fn lock_creates_directory_if_missing() {
    let dir = tempfile::tempdir().unwrap();
    let orch_dir = dir.path().join("nested").join(".orchestrator");

    assert!(!orch_dir.exists());

    let guard = lock::try_acquire(&orch_dir).unwrap();
    assert!(orch_dir.exists());

    drop(guard);
}

#[test]
fn lock_prevents_concurrent_acquisition() {
    let dir = tempfile::tempdir().unwrap();
    let orch_dir = dir.path().join(".orchestrator");

    let _guard1 = lock::try_acquire(&orch_dir).unwrap();

    // Second acquisition should fail because fslock is held
    let result = lock::try_acquire(&orch_dir);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("Another orchestrator instance"),
        "Error message should mention another instance: {}",
        err
    );
}

#[test]
fn lock_acquires_when_stale_pid_file_exists() {
    let dir = tempfile::tempdir().unwrap();
    let orch_dir = dir.path().join(".orchestrator");
    std::fs::create_dir_all(&orch_dir).unwrap();

    // Write a PID file with a definitely-dead PID
    let pid_path = orch_dir.join("orchestrator.pid");
    std::fs::write(&pid_path, "99999999").unwrap();

    // Lock file exists but is not held
    let lock_path = orch_dir.join("orchestrator.lock");
    std::fs::write(&lock_path, "").unwrap();

    // Should acquire since the fslock is not held (PID file is just leftover)
    let guard = lock::try_acquire(&orch_dir).unwrap();

    // Verify new PID was written
    let pid_contents = std::fs::read_to_string(&pid_path).unwrap();
    assert_eq!(
        pid_contents.trim().parse::<u32>().unwrap(),
        std::process::id()
    );

    drop(guard);
}

#[test]
fn lock_acquires_when_garbage_pid_file_exists() {
    let dir = tempfile::tempdir().unwrap();
    let orch_dir = dir.path().join(".orchestrator");
    std::fs::create_dir_all(&orch_dir).unwrap();

    // PID file with non-numeric content
    std::fs::write(orch_dir.join("orchestrator.pid"), "not_a_number").unwrap();
    std::fs::write(orch_dir.join("orchestrator.lock"), "").unwrap();

    // Should acquire since fslock is not held
    let guard = lock::try_acquire(&orch_dir).unwrap();
    drop(guard);
}

#[test]
fn lock_reacquire_after_release() {
    let dir = tempfile::tempdir().unwrap();
    let orch_dir = dir.path().join(".orchestrator");

    let guard = lock::try_acquire(&orch_dir).unwrap();
    drop(guard);

    // Should be able to acquire again
    let guard2 = lock::try_acquire(&orch_dir).unwrap();
    drop(guard2);
}

#[test]
fn lock_contention_via_fslock_without_pid_file() {
    let dir = tempfile::tempdir().unwrap();
    let orch_dir = dir.path().join(".orchestrator");
    std::fs::create_dir_all(&orch_dir).unwrap();

    // Hold the fslock externally without writing a PID file
    let lock_path = orch_dir.join("orchestrator.lock");
    let mut external_lock = fslock::LockFile::open(&lock_path).unwrap();
    assert!(external_lock.try_lock().unwrap());

    // try_acquire should fail because the fslock is held
    let result = lock::try_acquire(&orch_dir);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.contains("holds the lock"),
        "Error should mention held lock: {}",
        err
    );

    external_lock.unlock().unwrap();
}
