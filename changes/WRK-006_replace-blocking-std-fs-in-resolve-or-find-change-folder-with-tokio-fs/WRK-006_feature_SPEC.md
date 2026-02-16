# SPEC: Replace blocking std::fs in resolve_or_find_change_folder with tokio::fs

**ID:** WRK-006
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-006_replace-blocking-std-fs-in-resolve-or-find-change-folder-with-tokio-fs_PRD.md
**Execution Mode:** autonomous
**New Agent Per Phase:** no
**Max Review Attempts:** 3

## Context

The `resolve_or_find_change_folder` function in `executor.rs` uses blocking `std::fs` calls inside an async context. While the practical impact is low (called once per phase execution), it violates async best practices and creates a copyable anti-pattern. The codebase already has three established `tokio::fs` patterns in `agent.rs` that this conversion should follow. This is a mechanical API translation with no architectural changes.

## Approach

Convert `resolve_or_find_change_folder` from a sync function to an async function by replacing all `std::fs` calls with their `tokio::fs` equivalents. The function signature gains `async`, the return type stays `Result<PathBuf, String>`, and the single call site in `execute_phase` adds `.await`. As a secondary improvement, the `Path::exists()` pre-check is folded into `read_dir()` error handling to eliminate a TOCTOU race.

The iteration pattern changes from `for entry in entries` (sync iterator) to `while let Some(entry) = entries.next_entry().await?` (async polling), which is the canonical `tokio::fs::ReadDir` iteration pattern.

**Patterns to follow:**

- `src/agent.rs:178-191` — `ErrorKind::NotFound` matching pattern for expected-missing files (apply to `read_dir` error handling)
- `src/agent.rs:323-329` — `.map_err()` chains for descriptive error messages with tokio::fs (apply to `next_entry` and `create_dir_all` errors)
- `tokio::fs::ReadDir::next_entry()` docs — canonical `while let Some(entry) = entries.next_entry().await?` iteration pattern (no codebase precedent for directory iteration, follows tokio docs directly)

**Implementation boundaries:**

- Do not modify: any file other than `executor.rs`
- Do not refactor: the function's logic, error messages, or return type beyond what's needed for async conversion
- Do not add: new unit tests for the private function (it's tested indirectly through `execute_phase` integration tests)
- Do not add: `futures` crate dependency — use native `next_entry().await`

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | Async Conversion | Low | Convert resolve_or_find_change_folder to async and update call site |

**Ordering rationale:** Single phase — the function conversion and call site update are tightly coupled and must be committed together. Splitting them would leave the codebase in a non-compiling state.

---

## Phases

---

### Phase 1: Async Conversion

> Convert resolve_or_find_change_folder to async fn using tokio::fs and update the call site

**Phase Status:** complete

**Complexity:** Low

**Goal:** Replace all blocking std::fs calls with tokio::fs equivalents, making the function async and updating its single call site.

**Files:**

- `src/executor.rs` — modify — convert function to async, update call site

**Patterns:**

- Follow `src/agent.rs:178-191` for `ErrorKind::NotFound` matching on `read_dir`
- Follow `src/agent.rs:323-329` for `.map_err()` error message pattern

**Tasks:**

- [x] Change function signature from `fn resolve_or_find_change_folder(...)` to `async fn resolve_or_find_change_folder(...)`
- [x] Replace `if changes_dir.exists() { std::fs::read_dir(...) }` with `match tokio::fs::read_dir(&changes_dir).await { Ok(mut entries) => ..., Err(e) if e.kind() == ErrorKind::NotFound => ..., Err(e) => return Err(...) }`
- [x] Replace `for entry in entries` with `while let Some(entry) = entries.next_entry().await.map_err(|e| format!("Failed to read directory entry: {}", e))?`
- [x] Replace `entry.file_type().map(|t| t.is_dir()).unwrap_or(false)` with `entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false)`
- [x] Replace `std::fs::create_dir_all(&folder_path)` with `tokio::fs::create_dir_all(&folder_path).await`
- [x] Update call site at line ~311: add `.await` to `resolve_or_find_change_folder(root, &item.id, &item.title)`

**Verification:**

- [x] `cargo build` compiles without errors
- [x] `cargo clippy` passes with no warnings (3 pre-existing warnings unrelated to this change)
- [x] `cargo test --test executor_test` — all 31 `execute_phase_*` tests pass
- [x] `cargo test` — full test suite passes with no regressions
- [x] No `std::fs` calls remain in `resolve_or_find_change_folder`

**Commit:** `[WRK-006][P1] Replace blocking std::fs with tokio::fs in resolve_or_find_change_folder`

**Notes:**

- The `entry.file_type().await` change maintains the existing `.unwrap_or(false)` behavior — if `file_type()` fails, the entry is silently skipped (same as current sync behavior).
- The `next_entry().await` error is wrapped with `.map_err()` to preserve descriptive error messages matching the existing per-entry `.map_err()` in the sync version.
- The `ErrorKind::NotFound` arm for `read_dir` replaces the `exists()` pre-check and falls through to the directory creation path — functionally identical but TOCTOU-safe.
- Non-`NotFound` errors from `read_dir` (e.g., `PermissionDenied`) return immediately — they do NOT fall through to creation. This preserves current behavior where `read_dir` failure on an existing directory is an error.

**Followups:**

- [Low] Pre-existing: `item_id` is not validated for path traversal characters before use in `resolve_or_find_change_folder`. This is out of scope for this mechanical conversion but could be addressed in a future hardening pass.

---

## Final Verification

- [x] All phases complete
- [x] All PRD success criteria met
- [x] Tests pass
- [x] No regressions introduced
- [x] Code reviewed (if applicable)

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|
| 1: Async Conversion | complete | `[WRK-006][P1] Replace blocking std::fs with tokio::fs in resolve_or_find_change_folder` | All 6 tasks completed, all verification passed |

## Followups Summary

### Critical

### High

### Medium

### Low

- Pre-existing: `item_id` path traversal validation in `resolve_or_find_change_folder` (not introduced by this change)

## Design Details

### Target Function — Before

```rust
fn resolve_or_find_change_folder(
    root: &Path,
    item_id: &str,
    title: &str,
) -> Result<PathBuf, String> {
    let changes_dir = root.join("changes");
    let prefix = format!("{}_", item_id);

    if changes_dir.exists() {
        let entries = std::fs::read_dir(&changes_dir)
            .map_err(|e| format!("Failed to read {}: {}", changes_dir.display(), e))?;

        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&prefix)
                && entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
            {
                return Ok(entry.path());
            }
        }
    }

    let slug = slugify(title);
    let folder_name = format!("{}_{}", item_id, slug);
    let folder_path = changes_dir.join(folder_name);
    std::fs::create_dir_all(&folder_path)
        .map_err(|e| format!("Failed to create {}: {}", folder_path.display(), e))?;
    Ok(folder_path)
}
```

### Target Function — After

```rust
async fn resolve_or_find_change_folder(
    root: &Path,
    item_id: &str,
    title: &str,
) -> Result<PathBuf, String> {
    let changes_dir = root.join("changes");
    let prefix = format!("{}_", item_id);

    match tokio::fs::read_dir(&changes_dir).await {
        Ok(mut entries) => {
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| format!("Failed to read directory entry: {}", e))?
            {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(&prefix)
                    && entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false)
                {
                    return Ok(entry.path());
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Directory doesn't exist yet — fall through to creation
        }
        Err(e) => {
            return Err(format!(
                "Failed to read {}: {}",
                changes_dir.display(),
                e
            ));
        }
    }

    let slug = slugify(title);
    let folder_name = format!("{}_{}", item_id, slug);
    let folder_path = changes_dir.join(folder_name);
    tokio::fs::create_dir_all(&folder_path)
        .await
        .map_err(|e| format!("Failed to create {}: {}", folder_path.display(), e))?;
    Ok(folder_path)
}
```

### Call Site — Before

```rust
let change_folder = match resolve_or_find_change_folder(root, &item.id, &item.title) {
    Ok(path) => path,
    Err(e) => return PhaseExecutionResult::Failed(e),
};
```

### Call Site — After

```rust
let change_folder = match resolve_or_find_change_folder(root, &item.id, &item.title).await {
    Ok(path) => path,
    Err(e) => return PhaseExecutionResult::Failed(e),
};
```

### Design Rationale

Direct `tokio::fs` replacement chosen over `spawn_blocking` wrapper because:
1. Matches the three existing async fs patterns in `agent.rs`
2. Establishes the correct pattern for future conversions
3. Eliminates TOCTOU race by folding `exists()` into `read_dir()` error handling
4. `spawn_blocking` would require owned data (`PathBuf`/`String`) for the closure boundary

---

## Assumptions

- PRD states no new unit tests needed for this private function — existing `execute_phase` integration tests provide coverage
- Test files need no code changes since `execute_phase` is already async and tests already use `#[tokio::test]`
- The `entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false)` pattern preserves silent skip-on-error behavior from the sync version — this is intentional per the PRD's "error handling should remain identical" requirement

## Retrospective

### What worked well?

- The SPEC's before/after code blocks made implementation straightforward — copy, verify, done
- Single-phase approach was appropriate for the tightly-coupled function + call site change
- Existing async test infrastructure (tokio::test) required zero test modifications

### What was harder than expected?

- Nothing — this was a clean mechanical translation as anticipated

### What would we do differently next time?

- Nothing to change — the SPEC level of detail was well-calibrated for this size of change
