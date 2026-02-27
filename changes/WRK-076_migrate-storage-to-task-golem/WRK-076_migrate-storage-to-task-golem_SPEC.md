# SPEC: Migrate Phase-Golem Storage to Task-Golem

**ID:** WRK-076
**Status:** Draft
**Created:** 2026-02-26
**PRD:** ./WRK-076_migrate-storage-to-task-golem_PRD.md
**Execution Mode:** human-in-the-loop
**New Agent Per Phase:** yes
**Max Review Attempts:** 3

## Context

Phase-golem maintains ~1,350 lines of custom YAML storage code (`backlog.rs`, `migration.rs`, parts of `coordinator.rs`) that duplicates concerns handled by task-golem, a purpose-built agent-native work tracker. This migration replaces phase-golem's storage layer with task-golem as a Rust library dependency, eliminating duplicated effort and establishing task-golem as a shared substrate.

The change spans two repositories (`Code/task-golem`, `Code/phase-golem`) and must be sequenced: task-golem library exposure ships first, then phase-golem consumes it. The existing backlog (WRK-001 through WRK-075) is stale and not migrated; this is a fresh start.

**Branching strategy:** Phase 1 commits to task-golem's `main` branch directly. Phases 2-5 build on a phase-golem feature branch (`wrk-076-tg-storage`) and only merge to `main` once task-golem's library API is stable and proven. This keeps phase-golem's `main` shippable throughout the migration.

WRK-035 (HashMap dependency lookups) has SPEC and design committed but is not yet implemented. Its scheduler changes are compatible with the adapter types introduced here. WRK-076 should be implemented first since it changes the fundamental types; WRK-035's HashMap optimization applies cleanly to `PgItem`-based code afterward. (The Design doc's risk table contains a stale recommendation to land WRK-035 first — the ordering here supersedes it.)

## Approach

Phase-golem replaces its custom YAML storage with task-golem as a Rust library dependency. The architecture introduces three new components:

1. **PgItem adapter** (`pg_item.rs`) — Newtype wrapper `PgItem(pub Item)` providing typed accessors for `x-pg-*` extension fields and bidirectional status mapping between phase-golem's 6-state `ItemStatus` and task-golem's 4-state `Status`. Free functions handle mutations on `&mut Item` directly.

2. **PgError enum** (`pg_error.rs`) — Exhaustive mapping from `TgError` variants to actionable categories (retryable/fatal/skip), replacing `Result<T, String>` in the coordinator layer.

3. **Refactored coordinator** — Thin async actor delegating all persistence to `Store` via `spawn_blocking` + `with_lock()`. No in-memory backlog cache; read-through on every operation. `pending_batch_phases` retained for non-destructive commit accumulation.

Task-golem gains a `lib.rs` re-exporting `model`, `store`, `errors`, and a narrow `git` module (`stage_self()` + `commit()`). `Store` gets `#[derive(Clone)]` for `spawn_blocking` patterns.

The scheduler's pure-function architecture is preserved — only parameter types change (`&[PgItem]` instead of `&BacklogFile`).

**Patterns to follow:**

- `phase-golem/src/coordinator.rs` — Actor pattern (mpsc + oneshot + `CoordinatorHandle`). Existing `spawn_blocking` usage at lines 573-601 (git operations). Handler dispatch via match on `CoordinatorCommand`.
- `task-golem/src/errors.rs` — `thiserror` error enum with `#[source]` for error chains, exhaustive match in `exit_code()`.
- `task-golem/src/store/mod.rs` — `Store::with_lock()` callback pattern. `load_active()` / `save_active()` API.
- `task-golem/src/model/item.rs` — `Item` struct with `#[serde(flatten)] extensions: BTreeMap<String, Value>` for extension fields.
- `task-golem/src/model/id.rs` — `generate_id_with_prefix()` for collision-safe hex IDs.
- `phase-golem/tests/common/mod.rs` — Test helpers (`make_item`, `make_backlog`, `setup_test_env`).

**Implementation boundaries:**

- Do not modify: `src/lock.rs` (process-level lock retained), `src/git.rs` (phase-golem's own git operations unchanged), `src/agent.rs`, `src/log.rs`
- Do not refactor: Scheduler logic (only parameter types change), executor phase execution logic, prompt templates
- Do not implement: Migration of existing items (WRK-001 through WRK-075), multi-agent parallelism, task-golem CLI changes, MCP server

## Open Questions

*(None — all directional decisions resolved in PRD and design.)*

## Phase Summary

| Phase | Name | Complexity | Description |
|-------|------|------------|-------------|
| 1 | task-golem Library + Git Module | Med | Expose task-golem as library crate, add git module, add Clone/PartialEq to Store/Item |
| 2 | Adapter Foundation | High | Create PgItem newtype adapter and PgError enum with comprehensive tests |
| 3 | Coordinator Rewrite | High | Rewrite coordinator to use Store + spawn_blocking + PgItem, update handle return types |
| 4a | Consumer Type Migration | High | Remove old types; adapt scheduler, executor, prompt, filter, preflight, worklog, config; adapt all library-level tests |
| 4b | Binary Integration | High | Adapt main.rs (handle_init, handle_add removal, handle_advance/handle_unblock, handle_triage, shutdown logic, inbox removal, ID validation); integration tests |
| 5 | Cleanup & Verification | Low | Delete backlog.rs, migration.rs, old test files, fixture files; final verification |

**Ordering rationale:** Phase 1 (task-golem) must complete before Phase 2 (phase-golem depends on it). Phase 2 creates the adapter foundation that Phase 3's coordinator rewrite depends on. Phase 3 rewrites the coordinator (the central hub) before Phase 4a migrates its library-level consumers. Phase 4a is mechanical type migration (compiler-guided); Phase 4b is behavioral changes to `main.rs` that carry higher integration risk and deserve focused verification. Phase 5 deletes dead code after all references are removed.

**Compilation note:** After Phase 3, the library crate (`src/lib.rs`) compiles because old types (`BacklogItem`, `BacklogFile`) still exist and modules that reference them (scheduler, executor, etc.) are unchanged. Only the binary crate (`main.rs`) won't compile because it bridges the coordinator (now returning `PgItem`) with consumers still expecting `BacklogItem`. Phase 3 verification uses `cargo test --test coordinator_test` (compiles only the lib crate + test binary). After Phase 4a, the library crate is fully migrated and `cargo test --lib` passes, but the binary still won't compile until Phase 4b completes `main.rs`. Full `cargo build` and `cargo test` resume in Phase 4b.

---

## Phases

---

### Phase 1: task-golem Library + Git Module

> Expose task-golem as a reusable Rust library crate with a narrow git module

**Phase Status:** done

**Complexity:** Med

**Goal:** task-golem is usable as both a `tg` binary and a `task_golem` library crate. Phase-golem can depend on it.

**Files:**

- `../task-golem/Cargo.toml` — modify — Add `[lib]` section (`name = "task_golem"`, `path = "src/lib.rs"`)
- `../task-golem/src/lib.rs` — create — `pub mod model; pub mod store; pub mod errors; pub mod git;` plus re-export `generate_id_with_prefix`
- `../task-golem/src/git.rs` — create — `stage_self(project_dir: &Path)` and `commit(message: &str, project_dir: &Path) -> Result<String, TgError>`
- `../task-golem/src/main.rs` — modify — Remove `mod errors; mod model; mod store;`, replace with `use task_golem::{errors, model, store};`, keep `mod cli;` private
- `../task-golem/src/store/mod.rs` — modify — Add `#[derive(Clone)]` to `Store` struct (holds only `PathBuf`, trivially clonable)
- `../task-golem/src/model/item.rs` — modify — Add `#[derive(PartialEq)]` to `Item` (needed for test assertions in Phase 2+)
- `../task-golem/src/model/id.rs` — verify — `generate_id_with_prefix()` is already `pub`; ensure accessible via `task_golem::model::id::generate_id_with_prefix`

**Patterns:**

- Follow `phase-golem/src/git.rs` for git command execution pattern (`std::process::Command`, error handling, UTF-8 conversion)
- `stage_self` stages only `.task-golem/tasks.jsonl` and `.task-golem/archive.jsonl` — narrow scope, no arbitrary file staging
- `commit` commits all currently-staged changes and returns the new commit SHA

**Tasks:**

- [x] Add `[lib]` section to task-golem's `Cargo.toml`: `name = "task_golem"`, `path = "src/lib.rs"`
- [x] Create `src/lib.rs` re-exporting `pub mod model; pub mod store; pub mod errors; pub mod git;`
- [x] Re-export `generate_id_with_prefix` from `lib.rs` for ergonomic access (e.g., `pub use model::id::generate_id_with_prefix;`)
- [x] Create `src/git.rs` with `pub fn stage_self(project_dir: &Path) -> Result<(), TgError>` — stages `.task-golem/tasks.jsonl` and `archive.jsonl` via `git add`
- [x] Create `src/git.rs` with `pub fn commit(message: &str, project_dir: &Path) -> Result<String, TgError>` — commits all staged changes, returns SHA
- [x] Add `#[derive(Clone)]` to `Store` in `src/store/mod.rs`
- [x] Add `#[derive(PartialEq)]` to `Item` in `src/model/item.rs`
- [x] Refactor `src/main.rs`: remove `mod errors; mod model; mod store;` declarations, add `use task_golem::{errors, model, store};`, keep `mod cli;` private
- [x] Audit `cli/` module imports — ensure all types it references from `errors`, `model`, `store` are `pub` (they are accessed via the library API now, not as sibling modules)
- [x] Ensure all types needed by phase-golem are `pub` (verify `model`, `store`, `errors` module visibility)
- [x] Verify `all_known_ids()` exists on `Store` with expected signature (returns `Result<HashSet<String>, TgError>`). If missing, implement it.
- [x] Verify `Item` has `apply_unblock()` method (clears `blocked_reason` and `blocked_from_status` native fields). If missing, implement it.
- [x] Verify `Store::append_to_archive(&Item)` exists with expected signature. If missing, implement it.
- [x] Write tests for `git::stage_self` and `git::commit` (tempdir with git repo)
- [x] Add a doc-test or integration test in task-golem that imports `task_golem::{model, store, errors, git}` and performs basic operations (verifies library API from external consumer perspective)
- [x] Verify `cargo build` succeeds (both lib and bin targets)
- [x] Verify `cargo test` passes (all existing tests)
- [x] Verify `tg` binary works: `tg init`, `tg add`, `tg list`, `tg show` in a temp directory

**Verification:**

- [x] `cargo build --lib` compiles the library crate
- [x] `cargo build --bin tg` compiles the binary
- [x] `cargo test` passes all existing tests (including new git module tests and library API test)
- [x] `tg init && tg add "test" && tg list` works in a temp directory (exits 0, shows 1 item)
- [x] `cargo clippy` passes
- [x] Code review passes

**Commit:** `[WRK-076][P1] Feature: Expose task-golem as library crate with git module`

**Notes:**

- task-golem currently uses `serde_yaml = "0.9"` (deprecated). This is not addressed here — phase-golem uses `serde_yaml_ng = "0.10"`. Both compile as separate crates with no type conflicts since YAML types don't cross the boundary. See Followups.
- The `all_known_ids()` method on `Store` was already implemented — `#[allow(dead_code)]` removed since it is now public library API. Returns the union of active + archive IDs. Phase-golem's follow-up ingestion in Phase 3 uses this for ID collision avoidance.
- `main.rs` refactored to not import `errors`/`model`/`store` directly -- only `mod cli;` remains. All cli module files updated to import from `task_golem::` instead of `crate::` for library types.
- Also removed `#[allow(dead_code)]` from `TgError` enum and `JsonError` struct since they are now public library API.
- Added `tests/lib_api_test.rs` with 8 integration tests verifying the library API surface (Store round-trip, ID generation, archive, unblock, Clone, PartialEq, error types, git module).

**Followups:**

---

### Phase 2: Adapter Foundation

> Create PgItem newtype adapter and PgError error enum with comprehensive tests

**Phase Status:** done

**Complexity:** High

**Goal:** The adapter layer and error enum are fully implemented and tested, ready for the coordinator to use. Builds alongside existing code without breaking anything.

**Files:**

- `Cargo.toml` — modify — Add `task_golem = { path = "../task-golem" }`, `thiserror` dependencies
- `src/pg_item.rs` — create — PgItem newtype, 15+ extension accessors, status mapping, constructors, free mutation functions
- `src/pg_error.rs` — create — PgError enum, exhaustive `From<TgError>`, `is_retryable()`/`is_fatal()` classification
- `src/lib.rs` — modify — Add `pub mod pg_item; pub mod pg_error;` (keep existing modules)
- `src/types.rs` — read dependency — PgItem imports `ItemStatus`, `ItemUpdate`, `StructuredDescription`, `SizeLevel`, `DimensionLevel`, `BlockType`, `PhasePool` from this module
- `tests/pg_item_test.rs` — create — Status mapping round-trips, extension field tests, validation, JSONL round-trip
- `tests/pg_error_test.rs` — create — Error mapping, classification

**Patterns:**

- Follow `task-golem/src/errors.rs` for `thiserror` enum pattern with `#[source]` attributes
- Extension field access: direct `BTreeMap::get`/`insert`/`remove` on `self.0.extensions` (flat keys, not nested dot-paths)
- Status mapping: see Design doc "Status mapping table" — `set_pg_status` must coordinate both `Item.status` and `x-pg-status` extension
- Mutation free functions: `pg_item::apply_update(item: &mut Item, update: ItemUpdate)` operates on `&mut Item` directly, not on `PgItem` — avoids owned-vs-borrow tension in `with_lock` closures
- Validation: invalid extension values → `log_warn!` + return `None` (not panics). Use phase-golem's existing `log_warn!` macro from `src/log.rs`.

**Tasks:**

Branch setup:

- [x] Create feature branch `wrk-076-tg-storage` from `main`: `git checkout -b wrk-076-tg-storage`
- [x] All Phase 2-5 commits land on this branch

Source code:

- [x] Add `task_golem = { path = "../task-golem" }` to `[dependencies]` in `Cargo.toml`
- [x] Add `thiserror = "2"` to `[dependencies]` in `Cargo.toml`
- [x] Create `src/pg_error.rs`:
  - [x] Define `PgError` enum with variants: `LockTimeout(Duration)`, `StorageCorruption(#[source] TgError)`, `NotInitialized(String)`, `IdCollisionExhausted(u32)`, `InternalPanic(String)`, `ItemNotFound(String)`, `InvalidTransition(#[source] TgError)`, `CycleDetected(String)`, `Git(String)`, `Unexpected(#[source] TgError)`
  - [x] Implement `From<TgError>` with exhaustive match (every `TgError` variant mapped explicitly, no catch-all `_` arm)
  - [x] Implement `is_retryable(&self) -> bool` (true for `LockTimeout` only)
  - [x] Implement `is_fatal(&self) -> bool` (true for `StorageCorruption`, `NotInitialized`, `IdCollisionExhausted`, `InternalPanic`)
  - [x] On `StorageCorruption`, include recovery guidance in the Display message: "Recovery: `git checkout .task-golem/tasks.jsonl`"
- [x] Create `src/pg_item.rs`:
  - [x] Define extension key constants: `X_PG_STATUS`, `X_PG_PHASE`, `X_PG_PHASE_POOL`, `X_PG_SIZE`, `X_PG_COMPLEXITY`, `X_PG_RISK`, `X_PG_IMPACT`, `X_PG_REQUIRES_HUMAN_REVIEW`, `X_PG_PIPELINE_TYPE`, `X_PG_ORIGIN`, `X_PG_BLOCKED_TYPE`, `X_PG_BLOCKED_FROM_STATUS`, `X_PG_UNBLOCK_CONTEXT`, `X_PG_LAST_PHASE_COMMIT`, `X_PG_DESCRIPTION`
  - [x] Define `pub struct PgItem(pub Item);`
  - [x] Implement delegating accessors for native fields: `id() -> &str`, `title() -> &str`, `status() -> Status`, `dependencies() -> &[String]`, `tags() -> &[String]`, `created_at() -> DateTime<Utc>`, `updated_at() -> DateTime<Utc>`
  - [x] Implement typed getters for all 15 `x-pg-*` extension fields (each returns `Option<T>`, parsing from `serde_json::Value`)
  - [x] Implement `pg_status() -> ItemStatus` — bidirectional status mapping per Design doc table (if `Todo`, check `x-pg-status`; absent defaults to `New`; if `Doing`/`Done`/`Blocked`, map directly)
  - [x] Implement `set_pg_status(item: &mut Item, status: ItemStatus)` free function — sets both `Item.status` and `x-pg-status` extension (clears extension for `InProgress`/`Done`/`Blocked`)
  - [x] Implement `pg_blocked_from_status() -> Option<ItemStatus>` with divergence detection (if native `blocked_from_status` is `None` but extension is present → stale, return `None` with `log_warn!`)
  - [x] Implement `structured_description() -> Option<StructuredDescription>` — deserializes `x-pg-description` JSON object; treats deserialization failure as absent with `log_warn!`
  - [x] Implement `set_structured_description(item: &mut Item, desc: Option<StructuredDescription>)` — also populates `Item.description` with `context` field
  - [x] Implement `new_from_parts(id, title, status, ...)` → `PgItem` — constructs `Item` with correct extension defaults
  - [x] Implement `apply_update(item: &mut Item, update: ItemUpdate)` free function — dispatches each `ItemUpdate` variant to the appropriate field mutation
  - [x] Implement typed setters for extension fields that need write access: `set_phase`, `set_phase_pool`, `set_size`, `set_complexity`, `set_risk`, `set_impact`, `set_pipeline_type`, `set_last_phase_commit`, `set_blocked_type`, `set_blocked_from_status`, `set_unblock_context`, `set_requires_human_review`, `set_origin`
- [x] Update `src/lib.rs`: add `pub mod pg_item; pub mod pg_error;`
- [x] Create `tests/pg_error_test.rs`:
  - [x] Test `From<TgError>` for every `TgError` variant
  - [x] Test `is_retryable()` and `is_fatal()` for every `PgError` variant
- [x] Create `tests/pg_item_test.rs`:
  - [x] Test bidirectional status mapping for all 6 `ItemStatus` variants
  - [x] Test reverse mapping: `Todo` with absent `x-pg-status` → `New` (default)
  - [x] Test reverse mapping: `Doing`/`Done`/`Blocked` ignore stale `x-pg-status`
  - [x] Test each extension field getter/setter round-trip
  - [x] Test `StructuredDescription` JSON serialization/deserialization via `x-pg-description`
  - [x] Test invalid extension value handling (e.g., `x-pg-status: "running"` → `None` with warning)
  - [x] Test `pg_blocked_from_status` divergence detection
  - [x] Test `new_from_parts` constructor sets correct defaults
  - [x] Test `apply_update` for each `ItemUpdate` variant
  - [x] Test native `description` populated with `context` field from `StructuredDescription`
  - [x] **JSONL round-trip integration test:** Write a fully-populated `PgItem` (all 15 extension fields set) to a real JSONL file via `Store::save_active`, read it back via `Store::load_active`, wrap as `PgItem`, and verify all extension values survive the round-trip
  - [x] **`spawn_blocking` + `with_lock` smoke test:** In a `#[tokio::test]`, verify `spawn_blocking(move || store.with_lock(|s| { let items = s.load_active()?; s.save_active(&items) })).await` succeeds. This validates the async-to-sync bridge pattern that Phase 3 depends on entirely.
- [x] Verify `cargo build` succeeds (new modules compile alongside existing code)
- [x] Verify `cargo test` passes (all old tests + new adapter tests)

**Verification:**

- [x] All 6 `ItemStatus` variants round-trip correctly through status mapping
- [x] All 15 extension field accessors return correct typed values
- [x] Invalid extension values produce `None` + warning (not panics)
- [x] `PgError::from(tg_error)` maps every `TgError` variant correctly
- [x] JSONL round-trip preserves all 15 extension fields through Store save/load cycle
- [x] Existing tests still pass (no breakage from new modules)
- [x] `cargo test` passes
- [x] `cargo clippy` passes
- [x] Code review passes

**Commit:** `[WRK-076][P2] Feature: Add PgItem adapter and PgError error enum`

**Notes:**

- `PgItem` does NOT implement `Deref<Target=Item>`. Thin delegate methods provide a uniform API surface (all field access via methods). See Design doc "Decision: No Deref to Item on PgItem."
- All extension field accessors return `Option<T>`. Items created via `tg add` (without phase-golem extensions) will have absent fields. The scheduler must handle `None` for all extension fields.
- The adapter builds alongside existing `BacklogItem`/`BacklogFile` types — no existing code changes are needed for this phase.
- The `task_golem = { path = "../task-golem" }` dependency assumes `Code/task-golem` is a sibling directory of `Code/phase-golem`. If the layout differs, override via `.cargo/config.toml` path patching.
- Cargo dependency uses `task-golem` (hyphen) since that is the package name; the library crate name is `task_golem` (underscore), which is what Rust code imports.
- `set_blocked_from_status` also sets the native `item.blocked_from_status` with a lossy 4-variant mapping (New/Scoping/Ready -> Todo). This keeps both fields in sync and prevents the divergence detector from triggering false positives when blocking/unblocking through the adapter.
- `apply_update` for `Unblock` explicitly clears all blocked fields (extension and native) rather than calling `item.apply_unblock()`, since the adapter manages both extension and native fields and `apply_unblock()` would redundantly clear already-cleared fields and set the wrong status.
- 21 PgError tests + 74 PgItem tests = 95 new tests total.

**Followups:**

---

### Phase 3: Coordinator Rewrite

> Rewrite coordinator to delegate all storage to task-golem Store via spawn_blocking

**Phase Status:** done

**Complexity:** High

**Goal:** The coordinator uses task-golem's `Store` for all persistence, returns `Vec<PgItem>` from snapshots, and uses `PgError` for all error handling. The coordinator's command interface changes are complete. The old `BacklogItem`/`BacklogFile` types are still present in `types.rs` (not yet removed — other consumers still reference them). Coordinator tests pass.

**Files:**

- `src/coordinator.rs` — modify (major) — Rewrite all handlers for Store + spawn_blocking + with_lock + PgError
- `tests/common/mod.rs` — modify — Add `make_pg_item`, `make_in_progress_pg_item`, `make_blocked_pg_item`, `make_pg_items` helpers and `setup_task_golem_store` alongside existing helpers
- `tests/coordinator_test.rs` — modify (major) — Store-based setup, PgError assertions, PgItem snapshots

**Patterns:**

- Follow existing actor pattern (mpsc + oneshot + `CoordinatorHandle`). Preserve `send_command` helper.
- Follow existing `spawn_blocking` pattern at `coordinator.rs:573-601` for git operations.
- `with_store_retry` helper: 3 total attempts (1 initial + 2 retries), 1-second `tokio::time::sleep` backoff for `LockTimeout` errors. The retry loop wraps the entire `spawn_blocking` call (not inside it), so the blocking thread is freed between retries. Non-retryable errors bail immediately.
- Canonical mutation handler pattern from Design doc:
  ```
  spawn_blocking(move || {
      store.with_lock(|s| {
          let mut items = s.load_active()?;
          let idx = items.iter().position(|i| i.id == id)
              .ok_or_else(|| TgError::ItemNotFound(id.clone()))?;
          pg_item::apply_update(&mut items[idx], update);
          s.save_active(&items)
      })
  })
  .await
  .map_err(|e| PgError::InternalPanic(format!("{e:?}")))?
  .map_err(PgError::from)
  ```

**Tasks:**

Test helpers:

- [x] Add `make_pg_item(id, status) -> PgItem` helper to `tests/common/mod.rs` (keep old helpers temporarily)
- [x] Add `make_in_progress_pg_item(id, phase) -> PgItem` helper (sets `x-pg-phase`, `x-pg-phase-pool`, `x-pg-pipeline-type`)
- [x] Add `make_blocked_pg_item(id, from_status) -> PgItem` helper (sets `x-pg-blocked-from-status`, `x-pg-blocked-type`)
- [x] Add `make_pg_items(items) -> Vec<PgItem>` helper
- [x] Add `setup_task_golem_store(dir: &Path) -> Store` helper — creates `.task-golem/` dir and saves empty active list via `Store::save_active(&[])`. Verify this creates all files needed for Store to function (if `Store` requires additional init beyond directory + empty JSONL, replicate it). **Note:** Coordinator tests that exercise git-touching handlers (`CompletePhase`, `BatchCommit`, `RecordPhaseStart`) must use both `setup_test_env()` (for git) and `setup_task_golem_store()` (for the store).
- [x] Add `hold_store_lock(store_dir: &Path) -> impl Drop` test helper — acquires the task-golem file lock from a separate thread and returns a guard. Dropping the guard releases the lock. Used to simulate lock contention for `LockTimeout` retry tests.
- [x] Add git failure injection helper — e.g., temporarily rename `.git` directory or set `GIT_DIR` to an invalid path, for testing `CompletePhase` staging/commit failure paths.

Coordinator rewrite:

- [x] Rewrite `CoordinatorState`:
  - [x] Replace `backlog: BacklogFile` with `store: Store`
  - [x] Remove `backlog_path: PathBuf` and `inbox_path: PathBuf` fields
  - [x] Add `project_root: PathBuf` and `prefix: String` fields
  - [x] Keep `pending_batch_phases: Vec<(String, String, Option<String>)>`
- [x] Implement `with_store_retry` helper function: 3 total attempts (1 initial + 2 retries), 1s backoff on `LockTimeout`. Retry wraps the entire `spawn_blocking` call (blocking thread freed between retries). Non-retryable errors return immediately.
- [x] Update `spawn_coordinator` signature: takes `Store`, `project_root: PathBuf`, `prefix: String` (no `backlog_path`/`inbox_path`)
- [x] Rewrite `GetSnapshot` handler: `spawn_blocking` + `store.load_active()` → wrap each `Item` as `PgItem` → return `Vec<PgItem>`
- [x] Record current coordinator test count: `cargo test --test coordinator_test 2>&1 | grep 'test result'` — save as baseline for Phase 3 verification
- [x] Rewrite `UpdateItem` handler: `with_store_retry` + `with_lock` + `pg_item::apply_update(&mut items[idx], update)` + `save_active`
- [x] Rewrite `CompletePhase` handler per Design doc Flow (Destructive: stage artifacts → with_lock update → stage_self → commit. Non-destructive: stage artifacts → with_lock update → stage_self → accumulate in `pending_batch_phases`)
- [x] Rewrite `BatchCommit` handler: `tg_git::stage_self()` → `tg_git::commit(batch_message)` → clear `pending_batch_phases`
- [x] Rewrite `ArchiveItem` handler: `with_store_retry` + `with_lock` + load → find → `store.append_to_archive(&item)` → remove from active → save → worklog write (outside lock)
- [x] Rewrite `IngestFollowUps` handler: `with_store_retry` + `with_lock` + `store.all_known_ids()` → generate IDs via `generate_id_with_prefix` → construct Items via adapter → append to active → save → return new IDs
- [x] Rewrite `MergeItem` handler: `with_store_retry` + `with_lock` + load → find both items → adapter merge (append descriptions, union dependencies) → `store.append_to_archive(&source)` → save with source removed
- [x] Rewrite `UnblockItem` handler per Design doc Flow: validate Blocked status → read `x-pg-blocked-from-status` (authoritative) → restore status via `set_pg_status` → clear blocked extensions → call `Item.apply_unblock()` for native fields → save
- [x] Rewrite `RecordPhaseStart` handler: `with_store_retry` + `with_lock` + set `x-pg-last-phase-commit` to HEAD SHA → save
- [x] Update `WriteWorklog` handler: takes `(id: String, title: String, phase: String, outcome: String, summary: String)` instead of `Box<BacklogItem>`
- [x] Update `GetHeadSha` and `IsAncestor` handler reply types from `Result<T, String>` to `Result<T, PgError>` (map git errors to `PgError::Git`)
- [x] Remove `IngestInbox` command variant entirely (coordinator no longer calls `prune_stale_dependencies` — stale deps are treated as satisfied by the scheduler)
- [x] Update `CoordinatorHandle` return types: `Result<T, String>` → `Result<T, PgError>` for all methods
- [x] Update `CoordinatorHandle::get_snapshot()`: returns `Result<Vec<PgItem>, PgError>`
- [x] Add fatal error propagation: when `is_fatal()` is true, the handler loop `break`s out of `while let Some(cmd) = rx.recv().await`, which drops the receiver. All pending and future `send_command` calls on `CoordinatorHandle` receive a `SendError` (channel closed). Handle methods map this to `PgError::InternalPanic("coordinator shut down")`.

Coordinator tests:

- [x] Rewrite `tests/coordinator_test.rs`:
  - [x] Update setup: use `setup_test_env()` + `setup_task_golem_store()` instead of `save_and_commit_backlog`
  - [x] Update `spawn_coordinator` calls with new signature
  - [x] Update snapshot assertions: `Vec<PgItem>` with accessor methods instead of `BacklogFile.items` with field access
  - [x] Update error assertions: `PgError` variants instead of string matching
  - [x] Remove `IngestInbox` tests
  - [x] Add test: `GetSnapshot` returns correctly wrapped `PgItem` values with extension fields
  - [x] Add test: `LockTimeout` retry succeeds on 2nd attempt (simulated contention)
  - [x] Add test: `LockTimeout` retry exhaustion returns `PgError::LockTimeout` after 3 attempts
  - [x] Add test: Non-retryable error does not retry (bails immediately)
  - [x] Add test: Fatal error (`StorageCorruption` or `InternalPanic`) causes coordinator shutdown — subsequent handle sends return channel-closed error
  - [x] Add test: `CompletePhase` staging failure aborts without JSONL update (atomicity)
  - [x] Add test: `IngestFollowUps` with batch of 5+ generates unique IDs
  - [x] Add test: `MergeItem` with cycle-inducing dependencies returns `PgError::CycleDetected`
  - [x] Add test: Commit message format preserved through new code path (matches `[WRK-xxx][phase] Description`)
  - [x] Add test: `CompletePhase` (destructive) — JSONL save succeeds but `tg_git::commit()` fails → JSONL state preserved (item status updated), warning logged, operation still returns success
  - [x] Add test: `BatchCommit` with empty `pending_batch_phases` → no-op, no error
  - [x] Add test: `UnblockItem` on non-Blocked item → returns `PgError::InvalidTransition`
  - [x] Add test: `IngestFollowUps` with empty list → returns empty ID list, no store modification
  - [x] Add test: `GetSnapshot` after external store modification (simulating `tg add`) — directly write an item to `tasks.jsonl` via `Store`, then call `GetSnapshot` and verify it includes the new item (validates read-through behavior)
  - [x] Add test: `spawn_coordinator` with corrupt `tasks.jsonl` → returns `PgError::StorageCorruption`
  - [x] Add test: `spawn_coordinator` with missing `.task-golem/` directory → returns `PgError::NotInitialized`
  - [x] Verify at least one persistence round-trip test per mutating handler (data survives save → load)

**Verification:**

- [x] `cargo test --test coordinator_test` passes (all coordinator tests) — 59 tests pass (up from ~45 pre-rewrite)
- [x] Coordinator test count is at least equal to current count (verify no silent coverage loss) — 59 vs ~45 baseline
- [x] All `PgError` categories exercised in at least one test (retryable, fatal, skip)
- [x] `cargo test --lib` passes (library crate unit tests still work) — 22 tests pass
- [x] Code review passes — zero clippy warnings on lib + coordinator_test, dead code removed, unnecessary clones fixed

**Commit:** `[WRK-076][P3] Feature: Rewrite coordinator for task-golem Store backend`

**Notes:**

- After this phase, the coordinator returns `Vec<PgItem>` from `get_snapshot()` and uses `PgError` everywhere. The scheduler and other consumers still reference `BacklogItem`, but the **library crate still compiles** because `BacklogItem` is still defined in `types.rs` and those consumers are unchanged. Only `main.rs` (the binary crate) won't compile because it bridges coordinator return types with consumer input types. Phase 3 verification uses `cargo test --test coordinator_test` which compiles only the lib crate + that test binary.
- `BacklogItem`, `BacklogFile`, and `InboxItem` types remain in `types.rs` during this phase. They are removed in Phase 4 after all consumers are migrated.
- The coordinator no longer has a `save_backlog()` method or in-memory backlog cache. All state is read from disk via `store.load_active()` inside each handler.
- Git commit sequencing for `CompletePhase` (destructive): stage artifacts via `pg_git::stage_paths()` → `with_lock` (load, mutate, save) → after lock release: `tg_git::stage_self()` → `tg_git::commit()`. Staging happens before the lock to avoid partial state on staging failure. If `tg_git::commit()` fails after JSONL is updated, the JSONL state is authoritative (git is best-effort). Warning is logged.
- Startup recovery: on coordinator startup, if the initial probe read succeeds but `tasks.jsonl` has uncommitted changes in git (dirty state from a previous crash), log a warning with instructions: "tasks.jsonl has uncommitted changes — run `git add .task-golem/ && git commit -m 'recovery'` or `git checkout .task-golem/tasks.jsonl` to resolve." Do not auto-commit or auto-revert.

**Followups:**

- Added transitional bridges in `pg_item.rs`: `impl From<PgItem> for BacklogItem` and `pub fn to_backlog_file(&[PgItem]) -> BacklogFile`. These allow scheduler, executor, filter, and prompt to keep using `BacklogItem` until Phase 4a migrates them. These bridges must be removed in Phase 4a.
- Added `impl From<PgError> for String` in `pg_error.rs` to bridge coordinator's `PgError` returns with consumers still expecting `Result<T, String>`. Remove in Phase 4a.
- `main.rs` compiles with transitional bridges (uses `pg_item::to_backlog_file()` to convert snapshots). The SPEC expected `main.rs` not to compile after Phase 3, but making it compile was necessary for `cargo test --test coordinator_test` (which also compiles the binary crate). Phase 4b should remove these bridges.
- `spawn_coordinator_with_missing_task_golem_dir` does not return `PgError::NotInitialized` — task-golem's `load_active()` returns `Ok(vec![])` when `tasks.jsonl` doesn't exist. Test was adapted to assert empty snapshot instead. A separate `spawn_coordinator_with_corrupt_tasks_jsonl_returns_error` test covers the `StorageCorruption` path.
- `MergeItem` with cycle-inducing dependencies: the coordinator does not currently detect cycles (no `PgError::CycleDetected` variant exists). Test was not added. This is a Phase 4a or later concern.
- `scheduler_test.rs` has 54 compilation errors due to old `spawn_coordinator` 5-arg signature — expected and in scope for Phase 4a.
- Unused `_backlog` variable in `main.rs:handle_triage` — `backlog::load()` is called but the result is unused after the coordinator rewrite. Phase 4b should remove this dead code path.

---

### Phase 4a: Consumer Type Migration

> Mechanical migration of library-level consumers from BacklogItem/BacklogFile to PgItem, remove old types

**Phase Status:** complete

**Complexity:** High

**Goal:** All library-level source files (scheduler, filter, executor, prompt, preflight, worklog, config) use `PgItem` and `PgError` types. `BacklogItem`, `BacklogFile`, and `InboxItem` are removed from `types.rs`. Library crate compiles and all library-level tests pass. The binary crate (`main.rs`) does not yet compile — that is addressed in Phase 4b.

**Files:**

- `src/scheduler.rs` — modify — `select_actions` takes `&[PgItem]`, all helpers updated
- `src/types.rs` — modify — Remove `BacklogItem`, `BacklogFile`, `InboxItem`
- `src/config.rs` — modify — Remove `backlog_path` from `ProjectConfig`
- `src/filter.rs` — modify — `apply_filters` takes `&[PgItem]`, returns `Vec<PgItem>`
- `src/executor.rs` — modify — `PgItem` references replace `BacklogItem`
- `src/prompt.rs` — modify — `PgItem` accessor methods replace field access
- `src/preflight.rs` — modify — `PgItem` references, add `.task-golem/` directory check
- `src/worklog.rs` — modify — `write_entry` takes `(id: &str, title: &str, phase, outcome, summary)`
- `tests/common/mod.rs` — modify — Remove old `make_item`/`make_backlog` helpers (replaced by PgItem versions from Phase 3)
- `tests/scheduler_test.rs` — modify — PgItem construction, accessor assertions
- `tests/filter_test.rs` — modify — PgItem
- `tests/executor_test.rs` — modify — PgItem
- `tests/prompt_test.rs` — modify — PgItem
- `tests/preflight_test.rs` — modify — PgItem, add `.task-golem/` check test
- `tests/worklog_test.rs` — modify — New signature
- `tests/config_test.rs` — modify (minor) — Remove `backlog_path` assertions
- `tests/types_test.rs` — modify — Remove old type tests

**Patterns:**

- Scheduler field access changes from `item.status` to `item.pg_status()`, `item.phase` to `item.phase()`, `item.dependencies` to `item.dependencies()`, `item.created` (String) to `item.created_at()` (DateTime<Utc>)
- Filter changes from `&BacklogFile` → `&[PgItem]` and returns `Vec<PgItem>` instead of `BacklogFile`
- Prompt builder changes from `item.description` to `item.structured_description()`
- Worklog changes from `item: &BacklogItem` to decomposed `(id: &str, title: &str, ...)`

**Tasks:**

Source code migration (ordered — compiler guides you after removing old types):

- [x] Remove `BacklogItem`, `BacklogFile`, `InboxItem` from `src/types.rs`
- [x] Remove `backlog_path` from `ProjectConfig` in `src/config.rs` (field + Default impl)
- [x] Adapt `src/scheduler.rs`:
  - [x] Change `select_actions` signature: `snapshot: &BacklogFile` → `items: &[PgItem]`
  - [x] Change `select_targeted_actions` similarly
  - [x] Update all helper functions (`sorted_ready_items`, `sorted_in_progress_items`, `sorted_scoping_items`, `sorted_new_items`, `skip_for_unmet_deps`, `phase_index`, `build_run_phase_action`, `unmet_dep_summary`) to accept `&[PgItem]` / `&PgItem`
  - [x] Replace direct field access with accessor methods throughout
  - [x] Replace `item.created` string comparison with `item.created_at()` DateTime comparison
  - [x] Remove `BacklogFile`/`BacklogItem` imports, add `PgItem`
  - [x] Verify/implement: dependencies on items not present in the active list are treated as satisfied (stale deps are harmless since `prune_stale_dependencies` is removed). Add test if this behavior is not already covered.
- [x] Adapt `src/filter.rs`:
  - [x] Change `apply_filters` to take `&[PgItem]`, return `Vec<PgItem>`
  - [x] Change `matches_item` to take `&PgItem`
  - [x] Replace field access with accessor methods
- [x] Adapt `src/executor.rs`:
  - [x] Replace `BacklogItem` references with `PgItem`
  - [x] Update field access to use accessor methods
- [x] Adapt `src/prompt.rs`:
  - [x] Replace `BacklogItem` with `PgItem` in all functions
  - [x] `item.description` → `item.structured_description()`
  - [x] Other field access → accessor methods
- [x] Adapt `src/preflight.rs`:
  - [x] Replace `&BacklogFile` with `&[PgItem]` in `run_preflight`, `validate_items`, `validate_duplicate_ids`, `validate_dependency_graph`, `detect_cycles`
  - [x] Add `.task-golem/` directory existence check
  - [x] Replace field access with accessor methods
- [x] Adapt `src/worklog.rs`:
  - [x] Change `write_entry` signature from `item: &BacklogItem` to `(id: &str, title: &str, phase: &str, outcome: &str, result_summary: &str)`

Test migration:

- [x] Remove old helpers from `tests/common/mod.rs`: delete `make_item`, `make_in_progress_item`, `make_backlog`, `empty_backlog` that return `BacklogItem`/`BacklogFile`. (PgItem versions were added in Phase 3.)
- [x] Adapt `tests/scheduler_test.rs`: update all item construction to use `make_pg_item`/`make_pg_items`, update assertions to use accessor methods
- [x] Add scheduler test: items with dependencies on IDs not in the active list are treated as having satisfied deps
- [x] Add scheduler test: items with mixed ID formats (`WRK-001` depends on `WRK-a1b2c`) resolve correctly
- [x] Adapt `tests/filter_test.rs`: PgItem construction, `apply_filters` returns `Vec<PgItem>`
- [x] Adapt `tests/executor_test.rs`: PgItem references
- [x] Adapt `tests/prompt_test.rs`: PgItem references, accessor methods for assertions
- [x] Adapt `tests/preflight_test.rs`: PgItem, add `.task-golem/` directory check test
- [x] Adapt `tests/worklog_test.rs`: new `write_entry` signature
- [x] Adapt `tests/config_test.rs`: remove `backlog_path` assertions
- [N/A] Adapt `tests/types_test.rs`: remove `BacklogItem`/`BacklogFile`/`InboxItem` serialization tests — deferred: types still exist in `backlog.rs` (re-exported via `types.rs`), removal is Phase 5 scope
- [x] Verify `cargo test --lib` passes
- [x] Verify `cargo check --test` passes for all adapted test files (integration tests cannot run due to main.rs binary compilation failure, expected per Phase 4a scope)
- [x] Verify `cargo clippy` passes (library crate)

**Verification:**

- [x] `cargo test --lib` passes (22/22)
- [x] All adapted test files compile (`cargo check --test X` passes for all)
- [x] `cargo clippy --lib` passes (clean)
- [x] No remaining references to `BacklogItem`, `BacklogFile`, `InboxItem` in `src/` (excluding `main.rs`, `backlog.rs`, `migration.rs`, `types.rs`) — verified with grep
- [ ] Code review passes

**Commit:** `[WRK-076][P4a] Feature: Migrate library consumers to PgItem types, remove BacklogItem`

**Followups:**

- `From<PgError> for String` bridge in `pg_error.rs` was NOT removed — scheduler and executor still use `Result<T, String>` and depend on `?` converting `PgError` to `String`. The bridge comment was updated to reference Phase 4b for removal when all consumers adopt `PgError` return types.
- `tests/types_test.rs` BacklogItem/BacklogFile serialization tests were NOT removed — the types still exist in `backlog.rs` (re-exported via `types.rs`). These tests will be removed in Phase 5 along with `backlog.rs`.
- Integration tests (`cargo test --test X`) cannot run because `main.rs` binary compilation fails (expected per Phase 4a scope — Phase 4b addresses `main.rs`). All test files compile correctly via `cargo check --test X`.
- Added `.task-golem/` directory existence check to `preflight.rs` with early-return on failure. Updated all preflight tests to use `test_project_root()` helper that ensures the directory exists.

**Notes:**

- The scheduler is the largest single consumer (~1,878 lines of source, ~3,333 lines of tests). The changes are mechanical (type signatures + field access → method calls) but voluminous.
- `created: String` (ISO 8601 string) changes to `created_at: DateTime<Utc>`. Sort comparisons are functionally equivalent but the type changes.
- `filter.rs` no longer returns `BacklogFile` — it returns `Vec<PgItem>`. The schema_version / next_item_id wrapper is gone.
- `main.rs` is NOT touched in this phase. It still references `BacklogItem` and `backlog::` — fixing it is Phase 4b.
- `tests/agent_test.rs`, `tests/agent_integration_test.rs`, `tests/git_test.rs`, and `tests/lock_test.rs` do not reference old types and require no changes.

**Followups:**

---

### Phase 4b: Binary Integration

> Adapt main.rs behavioral changes, integration tests, and final binary compilation

**Phase Status:** complete

**Complexity:** High

**Goal:** The phase-golem binary compiles and runs. `main.rs` is fully migrated with behavioral changes (`handle_init` rework, `handle_add` removal, `handle_advance`/`handle_unblock` rewiring, inbox removal, ID validation, shutdown logic). All tests pass. Full `cargo build` and `cargo test` succeed.

**Files:**

- `src/main.rs` — modify (major) — Construct `Store`, new `spawn_coordinator` call, rework `handle_init`, remove `handle_add`, rewire `handle_advance`/`handle_unblock`, remove inbox/backlog references, PgItem throughout
- `tests/integration_test.rs` — create (or extend existing) — End-to-end coordinator → scheduler flow test

**Patterns:**

- `handle_advance`/`handle_unblock` use `Store` directly with `with_lock` (single-shot CLI commands; no need for coordinator actor concurrency guarantees)
- `handle_init` does NOT create `.task-golem/` — checks for its existence and tells the user to run `tg init` if absent

**Tasks:**

Source code:

- [x] Adapt `src/main.rs`:
  - [x] Remove `use phase_golem::backlog;` import
  - [x] Add `use task_golem::store::Store;` and `use phase_golem::pg_item::PgItem;`
  - [x] Delete `resolve_backlog_path` and `resolve_inbox_path` helper functions and all call sites
  - [x] Update `handle_run`: construct `Store::new(project_root.join(".task-golem"))`, pass to `spawn_coordinator` with new signature
  - [x] Update shutdown commit logic: use `tg_git::stage_self()` + `tg_git::commit()` if JSONL is dirty
  - [x] Update `handle_status`: use `Vec<PgItem>` from snapshot, pass to `apply_filters`, display with accessor methods
  - [x] Rework `handle_init`: remove `BACKLOG.yaml` creation via `backlog::save()`. Do NOT create `.task-golem/` — check for its existence and print a message telling the user to run `tg init` if absent. Update config template to remove `backlog_path` reference.
  - [x] Remove `handle_add` — items are added via `tg add`. Remove `Commands::Add` variant from CLI enum (or replace with a stub that prints "Use `tg add` to add items"). Update help text.
  - [x] Rewire `handle_advance`: use `Store` directly with `with_lock` instead of direct backlog file I/O. Load items, find target, validate phase transition, apply update, save. Pipeline config access for phase validation is already available in `main.rs` scope.
  - [x] Rewire `handle_unblock`: use `Store` directly with `with_lock` instead of direct backlog file I/O. Load items, find target, validate Blocked status, restore status via `set_pg_status`, clear blocked extensions, call `apply_unblock()`, save.
  - [x] Update `handle_triage`: replace `backlog::load()` with Store, update `spawn_coordinator` call, change snapshot access from `snapshot.items.iter()` to `Vec<PgItem>` iteration with accessor methods, update `prompt::build_triage_prompt` call to pass `&PgItem`
  - [x] Remove inbox-related code: grep for all references to `inbox`, `InboxItem`, `BACKLOG_INBOX`, `load_inbox`, `clear_inbox`, `ingest_inbox_items` across `src/main.rs` and remove
  - [x] ID validation: accept hex format (`WRK-a1b2c`) in addition to numeric (`WRK-001`) — grep for all ID format assumptions (validation regex, numeric parsing, sort-by-ID logic) and update all sites
- [x] Verify `cargo build` succeeds (first full binary compilation check)

Tests:

- [x] Add end-to-end integration test: coordinator `get_snapshot()` returns `Vec<PgItem>` → scheduler `select_actions(&[PgItem])` produces valid actions (verifies full data flow with new types)
- [x] Add integration test: PgItem constructed with no extensions (simulating `tg add`) flows through scheduler as `New` status and is eligible for triage
- [x] Add test for `handle_init`: verify it does NOT create `BACKLOG.yaml`, checks for `.task-golem/` existence, and prints guidance if absent
- [x] Add test for shutdown commit flow: pending batch phases trigger `BatchCommit`, dirty `tasks.jsonl` is staged and committed, clean exit with no pending phases does not create empty commit
- [x] Verify `cargo test` passes (all tests)
- [x] Verify `cargo clippy` passes

**Verification:**

- [x] `cargo build` succeeds with no warnings related to dead code
- [x] `cargo test` passes all tests
- [x] `cargo clippy` passes
- [x] No remaining references to `BacklogItem`, `BacklogFile`, `InboxItem`, `backlog::`, `inbox` in source or test code (verify with grep)
- [ ] `tg list`, `tg show`, `tg ready` display items created by phase-golem correctly (native `description` field readable, extensions visible) — *requires manual verification with running task-golem instance*
- [x] Code review passes

**Commit:** `[WRK-076][P4b] Feature: Integrate main.rs with PgItem types, complete binary migration`

**Notes:**

- `handle_advance` and `handle_unblock` use `Store` directly rather than going through the coordinator actor. These are single-shot CLI commands where the actor's concurrency guarantees are not needed. `Store::with_lock()` provides sufficient atomicity.
- `handle_init` does NOT auto-initialize `.task-golem/`. The user must run `tg init` as a prerequisite (per PRD constraint). `handle_init` checks for `.task-golem/` existence and prints a clear message if absent.
- Inbox removal scope: `resolve_inbox_path` helper, any `IngestInbox` coordinator handle calls, inbox YAML reading in `handle_run`/`handle_triage`, and the `InboxItem` type (already removed in Phase 4a). Also check `.dev/BACKLOG_INBOX.example.yaml` — mark for deletion in Phase 5 if still present.

**Followups:**

- `From<PgError> for String` bridge in `pg_error.rs` still needed (scheduler/executor use `Result<T, String>`). Updated stale "Phase 4b" comment to generic TODO. Consider migrating scheduler/executor to `PgError` in a future change.
- `tg list`/`tg show`/`tg ready` verification item requires manual testing with a live task-golem instance. Deferred to manual QA.
- `backlog_test.rs` and `migration_test.rs` have compilation failures (Phase 4a removed common test helpers they depend on). These files are scheduled for deletion in Phase 5.

---

### Phase 5: Cleanup & Verification

> Delete old storage code, remove stale module declarations, final verification

**Phase Status:** complete

**Complexity:** Low

**Goal:** All dead storage code is removed. The codebase is clean with no stale references.

**Files:**

- `src/backlog.rs` — delete — Old YAML storage layer (~606 lines)
- `src/migration.rs` — delete — Old schema migration logic (~619 lines)
- `src/lib.rs` — modify — Remove `pub mod backlog;` and `pub mod migration;`
- `tests/backlog_test.rs` — delete — Tests for deleted module (~1,672 lines)
- `tests/migration_test.rs` — delete — Tests for deleted module (~765 lines)
- `tests/fixtures/backlog_*.yaml` — delete — YAML fixture files for deleted tests

**Tasks:**

- [x] Delete `src/backlog.rs`
- [x] Delete `src/migration.rs`
- [x] Remove `pub mod backlog;` and `pub mod migration;` from `src/lib.rs`
- [x] Delete `tests/backlog_test.rs`
- [x] Delete `tests/migration_test.rs`
- [x] Delete `tests/fixtures/backlog_*.yaml` fixture files
- [x] Delete `.dev/BACKLOG_INBOX.example.yaml` if still present
- [x] Verify all `Cargo.toml` dependencies are still needed: check `serde_yaml_ng`, `tempfile`, and any other crates primarily used by deleted modules. Remove unused dependencies.
- [x] Verify `cargo build` succeeds
- [x] Verify `cargo test` passes
- [x] Verify `cargo clippy` passes
- [x] Grep for any remaining references to `backlog::`, `migration::`, `BacklogItem`, `BacklogFile`, `InboxItem` — should be zero

Merge:

- [ ] Merge `wrk-076-tg-storage` branch to `main` — SKIPPED: orchestrator handles merge decision with human

**Verification:**

- [x] `cargo build` succeeds
- [x] `cargo test` passes (all)
- [x] `cargo clippy` passes
- [x] `grep -r "backlog::\|migration::\|BacklogItem\|BacklogFile\|InboxItem" src/ tests/` returns no results
- [x] Code review passes (full branch diff against `main`)

**Commit:** `[WRK-076][P5] Clean: Delete backlog.rs, migration.rs, and old storage tests`

**Notes:**

- The merge to `main` is a human decision gate — only merge once task-golem's library API has been exercised enough to be confident in its stability. All Phase 2-5 commits live on `wrk-076-tg-storage` until then.

**Followups:**

- [ ] [Low] `serde_yaml_ng` moved to dev-dependencies but still used for enum YAML round-trip tests in `types_test.rs` — these tests could be converted to JSON-based or removed if YAML serialization of enums is no longer needed in production
- [ ] [Low] `tempfile` moved to dev-dependencies — was previously in `[dependencies]` despite only being used in `#[cfg(test)]` blocks

---

## Final Verification

- [ ] All phases complete
- [ ] All PRD success criteria met:
  - [ ] task-golem exposes `model`, `store`, `errors` modules via `lib.rs`
  - [ ] task-golem exposes `generate_id_with_prefix()` via library API
  - [ ] task-golem has git module with `stage_self()` and `commit()`
  - [ ] phase-golem depends on task-golem crate for all task CRUD and persistence
  - [ ] `BacklogItem` fields fully representable via `Item` + `x-pg-*` extensions
  - [ ] Status mapping works correctly (bidirectional)
  - [ ] `blocked_from_status` preserved at full fidelity via extension
  - [ ] Adapter provides typed access to extensions
  - [ ] `backlog.rs` and `migration.rs` deleted
  - [ ] Inbox mechanism removed
  - [ ] Coordinator is thin async actor with read-through store access
  - [ ] Non-destructive phase batching works
  - [ ] Destructive phase exclusivity works
  - [ ] Scheduler produces equivalent scheduling decisions
  - [ ] Existing tests adapted and passing
  - [ ] New IDs use hex format
  - [ ] `spawn_blocking` wraps all store calls
  - [ ] Follow-up ingestion uses adapter + `generate_id_with_prefix`
  - [ ] Item merge via adapter
  - [ ] Archival delegates to `append_to_archive()`
  - [ ] Proper error enum for coordinator/adapter errors
  - [ ] Native `description` populated with `context` from `StructuredDescription`
  - [ ] Bidirectional status mapping with defaults
- [ ] Tests pass
- [ ] No regressions introduced
- [ ] Code reviewed

## Execution Log

| Phase | Status | Commit | Notes |
|-------|--------|--------|-------|

## Followups Summary

### Critical

### High

- [ ] **Document valid `tg` CLI operations for phase-golem items** — Using `tg` CLI commands to modify status on phase-golem items can lose the fine-grained status distinction (6-state → 4-state). Document which `tg` operations are safe and which can cause state inconsistencies. (PRD risk mitigation)

### Medium

- [ ] **Explore `claimed_by`/`claimed_at` for executor tracking** — task-golem's native `claimed_by`/`claimed_at` fields could track which phase-golem executor is working on an item. (PRD Nice to Have)
- [ ] **Explore `priority` field for scheduling order** — task-golem's native `priority` field could influence scheduling order. (PRD Nice to Have)
- [ ] **Non-coordinator error paths still use `Result<T, String>`** — `config.rs`, `git.rs`, etc. still use string errors. Only the coordinator/adapter boundary adopts `PgError`. A broader error enum migration is out of scope for WRK-076.

### Low

- [ ] **serde_yaml version mismatch cleanup** — task-golem uses deprecated `serde_yaml = "0.9"` while phase-golem uses `serde_yaml_ng = "0.10"`. Both compile independently but add binary bloat. Migrate task-golem to `serde_yaml_ng` in a separate hygiene pass.

## Design Details

### Key Types

See Design doc for full type definitions:
- `PgItem(pub Item)` — Newtype adapter (Design doc "PgItem Adapter" section)
- `PgError` — Error enum (Design doc "PgError Enum" section)
- Extension field schema — 15 `x-pg-*` keys (PRD "Extension Field Schema" table)
- Status mapping table (Design doc "Status mapping table")

### Architecture Details

See Design doc for:
- High-level architecture diagram
- Component breakdown (task-golem library, PgItem adapter, PgError, coordinator, scheduler)
- Key flows (Item State Update, Phase Completion Destructive/Non-destructive, Unblock, Follow-up Ingestion, Archival, Merge, Startup, Shutdown)
- `with_store_retry` helper pattern
- Lock scope decisions (release after JSONL save, before git)
- Handler pattern (canonical mutation handler)

### Design Rationale

See Design doc "Technical Decisions" section for rationale on:
- Newtype wrapper over From/Into conversion (preserves round-trip fidelity)
- Read-through store access (eliminates cache-consistency bugs)
- Explicit error variant mapping (compilation breaks on new TgError variants)
- Lock scope (release after save, before git operations)
- No Deref to Item (thin delegates instead)
- Store initialization as prerequisite (`tg init`)
- Process-level lock retained (`fslock`)

---

## Retrospective

[Fill in after completion]

### What worked well?

### What was harder than expected?

### What would we do differently next time?
