use std::fs;

use tempfile::TempDir;

use orchestrate::types::{BacklogItem, ItemStatus};

fn make_test_item() -> BacklogItem {
    BacklogItem {
        id: "WRK-001".to_string(),
        title: "Test item".to_string(),
        status: ItemStatus::Done,
        phase: Some("review".to_string()),
        created: "2026-01-01T00:00:00+00:00".to_string(),
        updated: "2026-01-01T00:00:00+00:00".to_string(),
        ..Default::default()
    }
}

#[test]
fn write_entry_creates_file() {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let worklog_dir = dir.path().join("_worklog");

    let item = make_test_item();
    orchestrate::worklog::write_entry(&worklog_dir, &item, "Review", "Complete", "All tests pass")
        .expect("Failed to write entry");

    // Check that the worklog directory was created
    assert!(worklog_dir.exists(), "Worklog directory should exist");

    // Check that a YYYY-MM.md file was created
    let entries: Vec<_> = fs::read_dir(&worklog_dir)
        .expect("Failed to read worklog dir")
        .collect();
    assert_eq!(entries.len(), 1, "Expected exactly one worklog file");

    let filename = entries[0]
        .as_ref()
        .unwrap()
        .file_name()
        .to_string_lossy()
        .to_string();
    assert!(
        filename.ends_with(".md"),
        "Expected .md file, got: {}",
        filename
    );
    assert!(
        filename.len() == 10,
        "Expected YYYY-MM.md format (10 chars), got: {} ({})",
        filename,
        filename.len()
    );
}

#[test]
fn write_entry_contains_expected_fields() {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let worklog_dir = dir.path().join("_worklog");

    let item = make_test_item();
    orchestrate::worklog::write_entry(&worklog_dir, &item, "Build", "Complete", "Compiled successfully")
        .expect("Failed to write entry");

    // Read the file
    let entries: Vec<_> = fs::read_dir(&worklog_dir)
        .expect("Failed to read worklog dir")
        .collect();
    let file_path = entries[0].as_ref().unwrap().path();
    let contents = fs::read_to_string(file_path).expect("Failed to read worklog file");

    assert!(
        contents.contains("WRK-001"),
        "Expected item ID in worklog"
    );
    assert!(
        contents.contains("Test item"),
        "Expected item title in worklog"
    );
    assert!(
        contents.contains("Build"),
        "Expected phase in worklog"
    );
    assert!(
        contents.contains("Compiled successfully"),
        "Expected summary in worklog"
    );
    assert!(
        contents.contains("---"),
        "Expected separator in worklog"
    );
}

#[test]
fn write_entry_appends_chronologically() {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let worklog_dir = dir.path().join("_worklog");

    let item1 = make_test_item();
    let mut item2 = make_test_item();
    item2.id = "WRK-002".to_string();
    item2.title = "Second item".to_string();

    // Write first entry
    orchestrate::worklog::write_entry(&worklog_dir, &item1, "Build", "Complete", "First entry")
        .expect("Failed to write first entry");

    // Write second entry
    orchestrate::worklog::write_entry(&worklog_dir, &item2, "Review", "Complete", "Second entry")
        .expect("Failed to write second entry");

    // Read the file
    let entries: Vec<_> = fs::read_dir(&worklog_dir)
        .expect("Failed to read worklog dir")
        .collect();
    let file_path = entries[0].as_ref().unwrap().path();
    let contents = fs::read_to_string(file_path).expect("Failed to read worklog file");

    // First entry should appear before the second (chronological order)
    let pos_first = contents
        .find("WRK-001")
        .expect("Expected WRK-001 in worklog");
    let pos_second = contents
        .find("WRK-002")
        .expect("Expected WRK-002 in worklog");
    assert!(
        pos_first < pos_second,
        "Expected WRK-001 (older) to appear before WRK-002 (newer)"
    );
}

#[test]
fn write_entry_creates_parent_dirs() {
    let dir = TempDir::new().expect("Failed to create temp dir");
    let worklog_dir = dir.path().join("deep").join("nested").join("_worklog");

    let item = make_test_item();
    orchestrate::worklog::write_entry(&worklog_dir, &item, "Design", "Complete", "Deep nesting test")
        .expect("Failed to write entry in nested dir");

    assert!(
        worklog_dir.exists(),
        "Deeply nested worklog directory should exist"
    );
}
