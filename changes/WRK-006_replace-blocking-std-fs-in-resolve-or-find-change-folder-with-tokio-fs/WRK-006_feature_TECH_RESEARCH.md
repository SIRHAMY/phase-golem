# Tech Research: Replace blocking std::fs with tokio::fs

**ID:** WRK-006
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-006_replace-blocking-std-fs-in-resolve-or-find-change-folder-with-tokio-fs_PRD.md
**Mode:** Light

## Overview

Research how to convert `resolve_or_find_change_folder` in `executor.rs` from blocking `std::fs` calls to async `tokio::fs` equivalents. The function uses `read_dir`, `create_dir_all`, and `DirEntry::file_type()` inside an async context. We need to understand the correct async iteration pattern for `tokio::fs::ReadDir` and confirm there are no gotchas with this straightforward conversion.

## Research Questions

- [x] What is the correct async directory iteration pattern with `tokio::fs::read_dir`?
- [x] Are there any gotchas converting `std::fs` to `tokio::fs` for this use case?
- [x] Does the codebase already have async fs patterns to follow?

---

## External Research

### Landscape Overview

The Rust async filesystem landscape is straightforward: `tokio::fs` provides async wrappers around `std::fs` operations using `spawn_blocking` under the hood. There are no native async filesystem APIs on most operating systems, so `tokio::fs` simply offloads blocking calls to a dedicated thread pool. For this use case (single directory scan, called once per phase execution), the approach is a direct API translation with no architectural decisions needed.

### Common Patterns & Approaches

#### Pattern: Async Directory Iteration with next_entry()

**How it works:** Replace `std::fs::read_dir()` iterator with `tokio::fs::read_dir().await` followed by `while let Some(entry) = entries.next_entry().await?` loop.

**When to use:** Any directory traversal in async contexts on the Tokio runtime.

**Tradeoffs:**
- Pro: Prevents blocking the async runtime
- Pro: Cancellation-safe iteration
- Con: More verbose than iterator-based `for` loop
- Con: Still blocking underneath (`spawn_blocking`)

**Example:**
```rust
// Before (std::fs)
for entry in std::fs::read_dir("dir")? {
    let entry = entry?;
    // ...
}

// After (tokio::fs)
let mut entries = tokio::fs::read_dir("dir").await?;
while let Some(entry) = entries.next_entry().await? {
    // ...
}
```

**References:**
- [tokio::fs::ReadDir docs](https://docs.rs/tokio/latest/tokio/fs/struct.ReadDir.html) — API reference, cancellation safety
- [tokio::fs::DirEntry docs](https://docs.rs/tokio/latest/tokio/fs/struct.DirEntry.html) — Async `file_type()` method

#### Pattern: TOCTOU-safe Existence Checking via Error Handling

**How it works:** Instead of `path.exists()` (synchronous) followed by `read_dir()`, call `read_dir()` directly and match on `ErrorKind::NotFound`.

**When to use:** Checking directory existence before reading in async code.

**Tradeoffs:**
- Pro: TOCTOU-safe (no race between check and use)
- Pro: One async operation instead of two
- Con: Slightly less explicit than `if exists()` pattern

**Example:**
```rust
match tokio::fs::read_dir(&changes_dir).await {
    Ok(mut entries) => { /* iterate */ }
    Err(e) if e.kind() == ErrorKind::NotFound => { /* create */ }
    Err(e) => return Err(format!("Failed to read {}: {}", changes_dir.display(), e)),
}
```

### Technologies & Tools

| Technology | Purpose | Pros | Cons | Fit |
|------------|---------|------|------|-----|
| [tokio::fs](https://docs.rs/tokio/latest/tokio/fs/index.html) | Async filesystem ops | Already available, standard pattern | Uses `spawn_blocking` internally | Perfect fit |

No additional technologies needed. `tokio::fs` is included in `tokio` with the `"full"` feature already enabled.

### Standards & Best Practices

- Use `tokio::fs` in async contexts, never `std::fs` directly on the runtime
- Prefer TOCTOU-safe patterns (attempt operation, handle `NotFound`) over separate existence checks
- `tokio::fs::ReadDir::next_entry()` is cancellation-safe — prefer it over `StreamExt` adapters

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Not batching many small `tokio::fs` calls | Each `tokio::fs` call uses `spawn_blocking`, overhead adds up | Not applicable — this function makes 1-2 calls total |
| Using `futures::StreamExt` to iterate `ReadDir` | Unnecessary dependency, more complex | Use native `next_entry().await` pattern |
| Forgetting `.await` on `entry.file_type()` | `tokio::fs::DirEntry::file_type()` is async unlike `std::fs::DirEntry::file_type()` | Compiler will catch this — it returns a future, not `io::Result<FileType>` |

### Key Learnings

- The conversion is a mechanical API translation — no design decisions needed
- `tokio::fs::ReadDir` does not implement `Stream`; use `next_entry().await` in a `while let` loop
- `DirEntry::file_type()` becomes async (`.await` required) but is typically zero-cost on most platforms
- Folding `path.exists()` into `read_dir()` error handling is a strict improvement (TOCTOU elimination)

---

## Internal Research

### Existing Codebase State

The orchestrator is built on tokio 1.x with `"full"` features (Cargo.toml line 20). The codebase is async-first: `execute_phase` is async, the coordinator spawns tasks, and agent runners are async. The target function `resolve_or_find_change_folder` (executor.rs:468-498) is the only sync function in the async call chain.

**Relevant files/modules:**
- `executor.rs:468-498` — Target function with 3 blocking calls
- `executor.rs:311` — Call site in `execute_phase` (already async)
- `agent.rs:178-191` — Existing pattern: `tokio::fs::remove_file()` with `ErrorKind::NotFound` handling
- `agent.rs:323-335` — Existing pattern: `tokio::fs::read_to_string()` with `.map_err()`
- `agent.rs:339-346` — Existing pattern: async cleanup with `tokio::fs::remove_file()`

**Existing patterns in use:**
- `ErrorKind::NotFound` matching for expected-missing files (agent.rs:178)
- `.map_err()` chains for detailed error messages (agent.rs:323)
- All async fs operations use `tokio::fs` module directly

### Blocking Calls to Replace

| Line | Current Call | Replacement |
|------|-------------|-------------|
| 476 | `changes_dir.exists()` | Fold into `read_dir()` error handling |
| 477 | `std::fs::read_dir(&changes_dir)` | `tokio::fs::read_dir(&changes_dir).await` |
| 484 | `entry.file_type()` | `entry.file_type().await` |
| 495 | `std::fs::create_dir_all(&folder_path)` | `tokio::fs::create_dir_all(&folder_path).await` |

### Reusable Components

- Error handling pattern from `agent.rs:178` — `ErrorKind::NotFound` matching is already established
- `.map_err()` pattern from `agent.rs:323` — consistent error message formatting
- `slugify()` function (executor.rs:501-517) — pure function, no changes needed

### Constraints from Existing Code

- Function is private (`fn`, not `pub fn`) — API change is internal only
- Tests call `execute_phase` (already async with `#[tokio::test]`) — no test signature changes needed
- Error type is `Result<PathBuf, String>` — same for both sync and async versions
- Both `std::fs` and `tokio::fs` use `std::io::Error` — error handling patterns transfer directly

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| PRD says `DirEntry::file_type()` needs async conversion | Confirmed: `tokio::fs::DirEntry::file_type()` is an async method returning `impl Future<Output = io::Result<FileType>>` | Straightforward `.await` addition |
| PRD says no `futures` crate needed | Confirmed: `next_entry()` is a native async method on `ReadDir`, not a `Stream` trait | No new dependencies |
| PRD says `Path::exists()` can be folded into `read_dir` error handling | Confirmed: this is an established pattern in the codebase (agent.rs:178) and a TOCTOU improvement | Strictly better approach |

No concerns found — PRD assumptions are all confirmed by research.

---

## Critical Areas

### Iteration Pattern Change (for → while let)

**Why it's critical:** The `for entry in entries` loop becomes `while let Some(entry) = entries.next_entry().await?` — the error handling moves from per-entry `entry?` unwrap inside the loop to the `next_entry()` call itself.

**Why it's easy to miss:** The `?` on `next_entry().await?` handles both "failed to read entry" and "no more entries" (returns `None`). The old pattern had a separate `entry.map_err(...)` inside the loop body. In the new pattern, that error handling is absorbed by the `?` on `next_entry()`.

**What to watch for:** Ensure the error message for a failed directory entry read is still descriptive. `next_entry().await?` will propagate the raw `io::Error` — the `.map_err()` for entry-level errors needs to be applied differently or accepted as-is since `tokio::fs::ReadDir::next_entry()` already provides a clear error.

---

## Deep Dives

No deep dives needed — light mode research answered all questions.

---

## Synthesis

### Open Questions

None. The conversion path is clear and all assumptions are confirmed.

### Recommended Approaches

#### Async Conversion Strategy

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Direct `tokio::fs` replacement | Minimal diff, follows existing codebase patterns, no new deps | Entry-level error message slightly changes | Always — this is the standard approach |

**Initial recommendation:** Direct `tokio::fs` replacement. This is a mechanical translation with no design decisions. Follow the existing `tokio::fs` patterns from `agent.rs`.

#### Path Existence Check

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Fold into `read_dir()` error handling | TOCTOU-safe, one operation, follows codebase pattern | Slightly less explicit | Always — strict improvement |
| Separate `tokio::fs::metadata().await` check | More explicit | Extra async call, TOCTOU race remains | Never — inferior approach |

**Initial recommendation:** Fold into `read_dir()` error handling as the PRD suggests. This matches the `ErrorKind::NotFound` pattern already used in `agent.rs:178`.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [tokio::fs::ReadDir docs](https://docs.rs/tokio/latest/tokio/fs/struct.ReadDir.html) | Official docs | `next_entry()` API and cancellation safety |
| [tokio::fs::DirEntry docs](https://docs.rs/tokio/latest/tokio/fs/struct.DirEntry.html) | Official docs | Async `file_type()` method |
| [tokio::fs module docs](https://docs.rs/tokio/latest/tokio/fs/index.html) | Official docs | `create_dir_all` API |
| agent.rs:178-191 | Codebase | Established `ErrorKind::NotFound` pattern |
| agent.rs:323-335 | Codebase | Established `.map_err()` error handling pattern |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Light external research on tokio::fs patterns | Confirmed `next_entry().await` pattern, TOCTOU-safe existence checking |
| 2026-02-13 | Light internal research on codebase patterns | Found 3 existing `tokio::fs` patterns in agent.rs, confirmed tokio "full" features |
| 2026-02-13 | PRD assumption verification | All assumptions confirmed, no concerns |
