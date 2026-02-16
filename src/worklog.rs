use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::types::BacklogItem;

/// Write a worklog entry for a phase execution.
///
/// Appends an entry to `_worklog/YYYY-MM.md`.
/// Creates the file and parent directories if missing.
///
/// Format:
/// ```text
/// ## {datetime} — {item_id} ({title})
///
/// - **Phase:** {phase}
/// - **Outcome:** {outcome}
/// - **Summary:** {summary}
///
/// ---
/// ```
pub fn write_entry(
    worklog_dir: &Path,
    item: &BacklogItem,
    phase: &str,
    outcome: &str,
    result_summary: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now();
    let filename = now.format("%Y-%m").to_string();
    let worklog_path = worklog_dir.join(format!("{}.md", filename));

    fs::create_dir_all(worklog_dir)
        .map_err(|e| format!("Failed to create worklog directory {}: {}", worklog_dir.display(), e))?;

    let datetime = now.to_rfc3339();
    let entry = format!(
        "## {} — {} ({})\n\n- **Phase:** {}\n- **Outcome:** {}\n- **Summary:** {}\n\n---\n\n",
        datetime, item.id, item.title, phase, outcome, result_summary,
    );

    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .open(&worklog_path)
        .map_err(|e| format!("Failed to open worklog at {}: {}", worklog_path.display(), e))?;

    file.write_all(entry.as_bytes())
        .map_err(|e| format!("Failed to write worklog at {}: {}", worklog_path.display(), e))?;

    Ok(())
}
