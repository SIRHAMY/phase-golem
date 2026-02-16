# Design: Replace blocking std::fs in resolve_or_find_change_folder with tokio::fs

**ID:** WRK-006
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-006_replace-blocking-std-fs-in-resolve-or-find-change-folder-with-tokio-fs_PRD.md
**Tech Research:** ./WRK-006_feature_TECH_RESEARCH.md
**Mode:** Light

## Overview

Convert `resolve_or_find_change_folder` in `executor.rs` from a synchronous function using `std::fs` to an async function using `tokio::fs`. This is a mechanical API translation that follows three established `tokio::fs` patterns already in `agent.rs`. The function's logic, error handling semantics, and return type remain identical — only the I/O calls change from blocking to async. As a secondary improvement, the `Path::exists()` pre-check is folded into `read_dir()` error handling to eliminate a TOCTOU race.

---

## System Design

### High-Level Architecture

No architectural changes. The function's role in the system is unchanged: it resolves or creates a change folder for a given work item. The only change is the I/O mechanism (blocking → async) and the function signature (`fn` → `async fn`).

**Before:**
```
execute_phase (async) → resolve_or_find_change_folder (sync, blocks runtime)
                         ├── Path::exists()          [blocking]
                         ├── std::fs::read_dir()     [blocking]
                         ├── entry.file_type()       [blocking]
                         └── std::fs::create_dir_all [blocking]
```

**After:**
```
execute_phase (async) → resolve_or_find_change_folder (async, non-blocking)
                         ├── tokio::fs::read_dir()       [async, TOCTOU-safe]
                         ├── entries.next_entry()         [async]
                         ├── entry.file_type()            [async]
                         └── tokio::fs::create_dir_all   [async]
```

### Component Breakdown

#### resolve_or_find_change_folder (modified)

**Purpose:** Resolve an existing change folder or create one for a work item.

**Responsibilities:**
- Search `changes/` directory for a folder prefixed with `{item_id}_`
- Create `{item_id}_{slugified_title}` if none exists

**Interfaces:**
- Input: `root: &Path, item_id: &str, title: &str` (unchanged)
- Output: `Result<PathBuf, String>` (unchanged)

**Dependencies:** `tokio::fs` (replaces `std::fs`)

#### execute_phase call site (modified)

**Purpose:** Calls `resolve_or_find_change_folder` and uses the result path.

**Change:** Add `.await` to the function call at line 311.

### Data Flow

No change to data flow. The function receives the same inputs and produces the same outputs. Only the I/O execution model changes.

### Key Flows

#### Flow: Directory Search (existing folder found)

> Search the changes directory for an existing folder matching the item ID prefix.

1. **Open directory** — Call `tokio::fs::read_dir(&changes_dir).await` inside a `match` that handles `ErrorKind::NotFound` (fall through to creation) and other errors (return immediately with descriptive message)
2. **Iterate entries** — `while let Some(entry) = entries.next_entry().await.map_err(descriptive_msg)?`
3. **Check match** — Compare `entry.file_name()` against `{item_id}_` prefix
4. **Verify type** — Call `entry.file_type().await` to confirm it's a directory
5. **Return path** — Return `Ok(entry.path())`

**Edge cases:**
- `changes/` directory doesn't exist → `read_dir` returns `ErrorKind::NotFound` → fall through to create path
- `changes/` directory exists but is unreadable (e.g., permission denied) → `read_dir` returns non-NotFound error → return error immediately (does NOT fall through to creation)
- Entry is a file, not a directory → skip (same as current behavior)
- `file_type()` fails → `unwrap_or(false)` skips entry silently and continues iteration (same as current behavior)
- `next_entry()` fails mid-iteration (e.g., permission change) → error propagated with descriptive message via `.map_err()`

#### Flow: Directory Creation (no existing folder)

> Create a new change folder when none exists.

1. **No match found** — Either: (a) loop completed without finding a matching directory prefix, or (b) `read_dir` returned `ErrorKind::NotFound` (directory doesn't exist). Both paths converge here.
2. **Generate slug** — `slugify(title)` (pure function, unchanged)
3. **Build path** — `changes_dir.join(format!("{}_{}", item_id, slug))`
4. **Create directory** — `tokio::fs::create_dir_all(&folder_path).await`
5. **Return path** — Return `Ok(folder_path)`

**Edge cases:**
- `read_dir` error other than `NotFound` (e.g., `PermissionDenied`) → return error immediately, do NOT fall through to creation. This is functionally equivalent to the current behavior: if `exists()` returned true but `read_dir` failed, the error was propagated. The new pattern handles this in the same `match` arm.
- `create_dir_all` fails (e.g., permission denied, disk full) → error propagated with descriptive message via `.map_err()`

---

## Technical Decisions

### Key Decisions

#### Decision: Fold Path::exists() into read_dir error handling

**Context:** The current code calls `changes_dir.exists()` before `read_dir()`. This is a blocking call and introduces a TOCTOU race (directory could be deleted between check and read).

**Decision:** Remove the `exists()` check. Call `read_dir()` directly and match on `ErrorKind::NotFound` to fall through to directory creation.

**Rationale:** This pattern is already established in the codebase (`agent.rs:178`). It eliminates one blocking call and removes a TOCTOU race. It's a strict improvement.

**Consequences:** The control flow changes from `if exists { read_dir } else { create }` to `match read_dir { Ok => search, NotFound => create, Err => fail }`. This is slightly different structurally but equivalent in behavior.

#### Decision: Use while-let loop with next_entry().await

**Context:** `tokio::fs::ReadDir` does not implement `Iterator` or `Stream`. Iteration uses `next_entry().await` which returns `Option<DirEntry>`.

**Decision:** Replace the `for entry in entries` loop with `while let Some(entry) = entries.next_entry().await?`.

**Rationale:** This is the canonical pattern for `tokio::fs::ReadDir` iteration. It avoids needing `futures::StreamExt`.

**Consequences:** The `next_entry().await` call is wrapped with `.map_err(|e| format!("Failed to read directory entry: {}", e))?` to preserve descriptive error messages consistent with the codebase pattern (agent.rs:323). This maintains the same error context as the current per-entry `.map_err()` in the sync version.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Minor async overhead | `tokio::fs` uses `spawn_blocking` internally, adding thread-pool overhead | Non-blocking runtime, async consistency | Function is called once per phase execution — overhead is negligible |
| Control flow restructured | `if exists { search } else { create }` becomes `match read_dir { Ok => search, NotFound => create, Err => fail }` | TOCTOU elimination, one fewer I/O call | Established codebase pattern (agent.rs:178). Both paths are explicit in the `match` arms. |

**Note on cancellation:** This function does not need cancellation awareness. It performs 1-2 fast I/O operations (directory scan + optional mkdir), is called once per phase execution, and completes in milliseconds. The `execute_phase` caller handles cancellation at the phase level, not at the directory-resolution level.

---

## Alternatives Considered

### Alternative: Use spawn_blocking to wrap the existing sync function

**Summary:** Instead of converting to `tokio::fs`, wrap the entire sync function in `tokio::task::spawn_blocking()`.

**How it would work:**
- Keep the function as-is (sync)
- At the call site, wrap: `spawn_blocking(move || resolve_or_find_change_folder(root, id, title)).await`

**Pros:**
- Zero changes to function internals
- Smallest possible diff

**Cons:**
- Requires moving owned data into the closure (root, id, title become owned `PathBuf`/`String`)
- Doesn't establish the async pattern — future similar conversions would keep using this workaround
- Still blocks a thread from the blocking thread pool (same as `tokio::fs` internally, but less idiomatic)
- Goes against the codebase's established pattern of using `tokio::fs` directly

**Why not chosen:** The codebase uses `tokio::fs` directly in all three existing async fs patterns. Using `spawn_blocking` would create an inconsistent pattern. The conversion to `tokio::fs` is straightforward and the preferred approach.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Iteration order changes | None — directory iteration order is non-deterministic with both `std::fs` and `tokio::fs` | N/A | Function accepts any matching directory, not order-dependent |
| Concurrent directory creation race | Low — two concurrent executions could both attempt `create_dir_all` for the same item | Very low — orchestrator runs one phase per item at a time | Pre-existing in sync version; `create_dir_all` succeeds if directory already exists, so second caller wins harmlessly |

---

## Integration Points

### Existing Code Touchpoints

- `executor.rs:468-498` — Function body rewritten with async equivalents
- `executor.rs:311` — Call site adds `.await`
- No changes to function visibility, return type, or error type

### External Dependencies

None. `tokio::fs` is already available via `tokio` with `"full"` features enabled in `Cargo.toml`.

---

## Open Questions

None. The conversion path is fully defined by the PRD, confirmed by tech research, and follows established codebase patterns.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Initial design draft | Mechanical async conversion following established patterns, one alternative considered (spawn_blocking) and rejected |
| 2026-02-13 | Self-critique (7 agents) | Auto-fixed: preserved error messages via `.map_err()`, clarified non-NotFound error handling, added cancellation note, documented concurrent access risk. No directional or quality items requiring human input. |
