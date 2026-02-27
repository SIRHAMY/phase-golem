# Design: Migrate Phase-Golem Storage to Task-Golem

**ID:** WRK-076
**Status:** In Review
**Created:** 2026-02-25
**PRD:** ./WRK-076_migrate-storage-to-task-golem_PRD.md
**Tech Research:** ./WRK-076_migrate-storage-to-task-golem_TECH_RESEARCH.md
**Mode:** Medium

## Overview

Phase-golem replaces its custom YAML storage layer with task-golem as a Rust library dependency. The design centers on a **newtype adapter** (`PgItem`) that wraps task-golem's `Item` with typed accessors for `x-pg-*` extension fields, a **refactored coordinator** that delegates all persistence to task-golem's `Store` via `spawn_blocking`, and a **proper error enum** (`PgError`) that maps task-golem errors to actionable categories. Task-golem gains a `lib.rs` and a narrow git module. The scheduler's pure-function architecture is preserved — only parameter types change.

---

## System Design

### High-Level Architecture

```
┌──────────────────────────────────────────────────────────────────┐
│  phase-golem                                                      │
│                                                                    │
│  ┌────────────┐    ┌─────────────────┐    ┌──────────────────┐   │
│  │ Scheduler  │───▶│  Coordinator    │───▶│  Adapter         │   │
│  │ (pure fn)  │    │  (async actor)  │    │  (pg_item.rs)    │   │
│  │            │    │                 │    │                   │   │
│  │ &[PgItem]  │    │ mpsc channel    │    │ PgItem(Item)     │   │
│  │ → Actions  │    │ spawn_blocking  │    │ typed x-pg-* get │   │
│  │            │    │ retry on lock   │    │ status mapping   │   │
│  └────────────┘    │ pending_batch   │    │ PgError enum     │   │
│                    └────────┬────────┘    └────────┬─────────┘   │
│                             │                      │              │
│  ┌────────────┐             │                      │              │
│  │ Executor   │─────────────┘                      │              │
│  │ (phases)   │    stages artifacts via git add     │              │
│  └────────────┘                                    │              │
│                                                    │              │
└────────────────────────────────────────────────────┼──────────────┘
                                                     │
                    ┌────────────────────────────────┼──────────────┐
                    │  task-golem (library crate)     │              │
                    │                                │              │
                    │  ┌─────────┐  ┌──────────┐    │              │
                    │  │  Store  │  │ Git Mod  │◀───┘              │
                    │  │         │  │          │                    │
                    │  │ JSONL   │  │stage_self│                    │
                    │  │ fd-lock │  │commit()  │                    │
                    │  │ atomic  │  │          │                    │
                    │  └─────────┘  └──────────┘                    │
                    │                                                │
                    │  .task-golem/                                  │
                    │  ├── tasks.jsonl                               │
                    │  ├── archive.jsonl                             │
                    │  ├── tasks.lock                                │
                    │  └── config.yaml                               │
                    └────────────────────────────────────────────────┘
```

### Component Breakdown

#### task-golem Library Crate (Part 1 — ships independently)

**Purpose:** Expose task-golem's core functionality as a reusable Rust library alongside the existing `tg` binary.

**Responsibilities:**
- Re-export `model`, `store`, and `errors` modules via `lib.rs`
- Expose `generate_id_with_prefix()` for consumers that need custom ID prefixes
- Provide a narrow git module: `stage_self()` (stages `.task-golem/tasks.jsonl` and `archive.jsonl`) and `commit(message)` (commits all currently-staged changes)

**Interfaces:**
- Input: Library consumers import `task_golem::{model, store, errors, git}`
- Output: Public API surface including `Store`, `Item`, `Status`, `TgError`, `generate_id_with_prefix()`

**Dependencies:** None new. Existing deps (chrono, serde, serde_json, fd-lock, tempfile, thiserror) are shared with the binary.

**Changes required:**
- Add `[lib]` section to `Cargo.toml`: `name = "task_golem"`, `path = "src/lib.rs"`
- Create `src/lib.rs` with `pub mod model; pub mod store; pub mod errors; pub mod git;`
- Update `src/main.rs` to import shared modules from the library crate (`use task_golem::model;` etc.) — `mod cli;` stays private to main
- Create `src/git.rs` module with functions (signatures below)
- Add `#[derive(Clone)]` to `Store` (required for phase-golem's `spawn_blocking` pattern)
- Mark internal-only items with `pub(crate)` to prevent accidental exposure

**Git module function signatures:**

```rust
/// Stages .task-golem/tasks.jsonl and .task-golem/archive.jsonl
pub fn stage_self(project_dir: &Path) -> Result<(), TgError>

/// Commits all currently-staged changes with the given message.
/// Returns the new commit SHA on success.
pub fn commit(message: &str, project_dir: &Path) -> Result<String, TgError>
```

Note: `stage_self` only stages task-golem's own files. Phase-golem stages its artifact files separately via its own `git::stage_paths()`. The two git modules have distinct scopes: task-golem's stages JSONL state files; phase-golem's stages arbitrary artifact paths.

#### PgItem Adapter (`src/pg_item.rs`)

**Purpose:** Typed, ergonomic access to phase-golem-specific fields stored in task-golem's extension fields. This is the **data translation boundary** between the two models. The adapter is responsible for field access and status encoding/decoding only — not for business logic like state machine transitions or update dispatch.

**Responsibilities:**
- Newtype wrapper `PgItem(pub Item)` over task-golem's `Item`
- Typed getter/setter methods for all `x-pg-*` extension fields
- Delegating accessor methods for commonly-used native `Item` fields (avoid `.0.field` verbosity)
- Bidirectional status mapping: `ItemStatus` ↔ `Status` + `x-pg-status`
- Extension key constants to prevent string typos
- Validation: invalid extension values treated as absent with `tracing::warn!`
- Constructor for creating new items with correct extension defaults
- Populate task-golem's native `description` field with `StructuredDescription.context`

**Interfaces:**
- Input: `Item` from task-golem store operations
- Output: `PgItem` with typed accessors; inner `Item` accessible via `.0` for store passthrough

**Dependencies:** `task_golem::model::Item`, `serde_json` for extension field serialization

**Design details:**

```
PgItem(pub Item)
│
│ ── Native field delegates (thin wrappers over self.0) ──
├── id() -> &str
├── title() -> &str
├── status() -> Status           // task-golem's native Status
├── dependencies() -> &[String]
├── tags() -> &[String]
├── created_at() -> DateTime<Utc>
├── updated_at() -> DateTime<Utc>
│
│ ── Extension field accessors (typed x-pg-* access) ──
├── pg_status() -> ItemStatus           // bidirectional status mapping
├── set_pg_status(&mut, ItemStatus)     // sets Status + x-pg-status
├── phase() -> Option<String>           // x-pg-phase
├── set_phase(&mut, Option<String>)
├── phase_pool() -> Option<PhasePool>   // x-pg-phase-pool
├── set_phase_pool(&mut, Option<PhasePool>)
├── size() -> Option<SizeLevel>         // x-pg-size
├── complexity() -> Option<DimensionLevel>  // x-pg-complexity
├── risk() -> Option<DimensionLevel>    // x-pg-risk
├── impact() -> Option<DimensionLevel>  // x-pg-impact
├── requires_human_review() -> bool     // x-pg-requires-human-review (absent = false)
├── pipeline_type() -> Option<String>   // x-pg-pipeline-type
├── origin() -> Option<String>          // x-pg-origin
├── blocked_type() -> Option<BlockType> // x-pg-blocked-type
├── pg_blocked_from_status() -> Option<ItemStatus>  // x-pg-blocked-from-status (see divergence note)
├── unblock_context() -> Option<String> // x-pg-unblock-context
├── last_phase_commit() -> Option<String>  // x-pg-last-phase-commit
├── structured_description() -> Option<StructuredDescription>  // x-pg-description
├── set_structured_description(&mut, Option<StructuredDescription>)
│   // also populates Item.description with context field
│
│ ── Construction ──
└── new_from_parts(id, title, ...) -> PgItem
    // Sets: created_at/updated_at = Utc::now(), priority = 0,
    // status = Todo, x-pg-status = "new", claimed_by/claimed_at = None
```

All extension field accessors return `Option<T>`. Items created via `tg add` (without phase-golem extensions) will genuinely have absent fields, and the scheduler must handle `None` for all extension fields. There is no "required" vs "optional" distinction — all extensions are optional with documented defaults (e.g., `requires_human_review` defaults to `false` when absent).

**Status mapping table:**

| Phase-golem `ItemStatus` | Task-golem `Status` | `x-pg-status` |
|---|---|---|
| `New` | `Todo` | `"new"` |
| `Scoping` | `Todo` | `"scoping"` |
| `Ready` | `Todo` | `"ready"` |
| `InProgress` | `Doing` | *(absent — cleared)* |
| `Done` | `Done` | *(absent — cleared)* |
| `Blocked` | `Blocked` | *(absent — cleared)* |

Reverse mapping: if `Status::Todo`, check `x-pg-status` — absent defaults to `New`. If `Doing`/`Done`/`Blocked`, map directly and ignore any stale `x-pg-status` that may be present (e.g., from a direct `tg do` transition that bypassed the adapter — the stale extension is harmless on forward reads but would regress if the item later returns to `Todo` via `tg` CLI; this is accepted as human operator responsibility).

**`pg_blocked_from_status()` divergence detection:** If `Item.blocked_from_status` is `None` (cleared by `tg unblock`) but `x-pg-blocked-from-status` extension is still present, the extension is stale — return `None` with `tracing::warn!`. If `Item.blocked_from_status` is `Some`, return the extension value as authoritative. This handles the case where a human runs `tg unblock`, which clears the native field but cannot update the extension.

**`x-pg-description` corruption handling:** If `serde_json::from_value::<StructuredDescription>()` fails on the extension value (e.g., human edited it to a non-object), treat as absent with `tracing::warn!`. The item continues through the pipeline without a structured description — this is the same behavior as a newly-added item.

#### PgError Enum (`src/pg_error.rs`)

**Purpose:** Map task-golem's `TgError` variants to phase-golem's error handling categories (retryable, fatal, skip).

**Responsibilities:**
- Exhaustive variant mapping from every `TgError` variant to a specific `PgError` variant
- `is_retryable()` and `is_fatal()` classification methods
- Handle `JoinError` from `spawn_blocking` (panic = fatal)
- Preserve error source chain via `#[source]` attributes

**Interfaces:**
- Input: `TgError` from store operations, `JoinError` from spawn_blocking
- Output: `PgError` with categorization methods

**Design details:**

```rust
#[derive(Debug, thiserror::Error)]
pub enum PgError {
    // Retryable
    #[error("Lock timeout after {0:?}")]
    LockTimeout(Duration),  // preserves original Duration

    // Fatal — halt coordinator
    #[error("Storage corruption: {0}")]
    StorageCorruption(#[source] TgError),  // preserves source chain
    #[error("Store not initialized: {0}")]
    NotInitialized(String),
    #[error("ID collision exhausted after {0} attempts")]
    IdCollisionExhausted(u32),
    #[error("Internal panic in storage thread: {0}")]
    InternalPanic(String),

    // Skip — log and continue
    #[error("Item not found: {0}")]
    ItemNotFound(String),
    #[error("Invalid transition: {0}")]
    InvalidTransition(#[source] TgError),  // preserves source chain
    #[error("Cycle detected: {0}")]
    CycleDetected(String),

    // Git
    #[error("Git error: {0}")]
    Git(String),

    // Catch-all for unexpected variants
    #[error("Unexpected storage error: {0}")]
    Unexpected(#[source] TgError),  // preserves source chain
}
```

**Exhaustive `From<TgError>` mapping:**

| `TgError` variant | `PgError` variant | Category | Rationale |
|---|---|---|---|
| `LockTimeout(Duration)` | `LockTimeout(d)` | Retryable | Transient contention |
| `StorageCorruption(msg)` | `StorageCorruption(err)` | Fatal | Unrecoverable data issue |
| `SchemaVersionUnsupported{..}` | `StorageCorruption(err)` | Fatal | Incompatible task-golem version |
| `NotInitialized(msg)` | `NotInitialized(msg)` | Fatal | `.task-golem/` missing |
| `IdCollisionExhausted(n)` | `IdCollisionExhausted(n)` | Fatal | Random hex failure after n retries |
| `ItemNotFound(id)` | `ItemNotFound(id)` | Skip | Item deleted by human between read and write |
| `InvalidTransition{..}` | `InvalidTransition(err)` | Skip | Stale state; scheduler retries next loop |
| `CycleDetected(msg)` | `CycleDetected(msg)` | Skip | Dependency invariant violated during merge |
| `AmbiguousId{..}` | `Unexpected(err)` | Unexpected | Phase-golem uses exact IDs; should never occur |
| `AlreadyClaimed(msg)` | `Unexpected(err)` | Unexpected | Not used by phase-golem currently |
| `InvalidInput(msg)` | `Unexpected(err)` | Unexpected | Should not occur with adapter-validated input |
| `DependentExists(id, dep)` | `Unexpected(err)` | Unexpected | Phase-golem does not enforce dependent removal |
| `IoError(e)` | `Unexpected(err)` | Unexpected | Wrapped in TgError; classify at coordinator level if needed |

No `#[from] std::io::Error` variant — all IO errors arrive through `TgError::IoError` and are mapped through the explicit `From<TgError>`.

**Fatal error propagation:** When the coordinator encounters `is_fatal()`, it: (1) replies to the current command's oneshot with the error, (2) logs the error at `error!` level, (3) drops the mpsc receiver, which causes all future `CoordinatorHandle` sends to fail with a channel-closed error. The main scheduler loop detects the channel-closed condition and exits with a clear error message. This is the same shutdown mechanism as the current coordinator — dropping the receiver signals all callers.

#### Refactored Coordinator (`src/coordinator.rs`)

**Purpose:** Thin async actor that serializes write operations and delegates all persistence to task-golem's `Store`.

**Responsibilities:**
- Maintain mpsc channel and `CoordinatorHandle` (interface preserved, return types change to `Result<T, PgError>`)
- Delegate all storage to task-golem via `spawn_blocking` + `Store::with_lock()`
- No in-memory `BacklogFile` — read-through on every operation
- Retain `pending_batch_phases: Vec<(String, String, Option<String>)>` for non-destructive commit accumulation
- Retry `LockTimeout` errors up to 3 times with 1-second backoff
- Git commit sequencing: stage artifacts → update item via store (inside lock) → `stage_self()` (outside lock) → `commit()` (outside lock)

**Interfaces:**
- Input: `CoordinatorCommand` variants (command set changes documented below)
- Output: Results via oneshot channels — `Result<T, PgError>` instead of `Result<T, String>`

**Dependencies:** `task_golem::store::Store`, adapter layer

**Command set changes:**

| Command | Status | Notes |
|---|---|---|
| `GetSnapshot` | **Changed** — returns `Vec<PgItem>` | `spawn_blocking` + `load_active()` + wrap as `PgItem` |
| `UpdateItem` | **Changed** — uses store | `spawn_blocking` + `with_lock` + adapter mutation |
| `CompletePhase` | **Changed** — uses store + git | See Phase Completion flows |
| `BatchCommit` | **Changed** — uses task-golem git | `stage_self()` + `commit()` |
| `ArchiveItem` | **Changed** — uses store | `with_lock` + `append_to_archive` |
| `IngestFollowUps` | **Changed** — uses store | `with_lock` + `generate_id_with_prefix` + save |
| `MergeItem` | **Changed** — uses store | `with_lock` + adapter merge |
| `UnblockItem` | **Changed** — uses store | `with_lock` + adapter unblock (see flow) |
| `RecordPhaseStart` | **Changed** — uses store | `with_lock` + set `x-pg-last-phase-commit` |
| `WriteWorklog` | **Unchanged** — phase-golem-specific | Takes `(id, title, phase, outcome, summary)` instead of `Box<BacklogItem>` |
| `GetHeadSha` | **Unchanged** — uses phase-golem's `git.rs` | No task-golem involvement |
| `IsAncestor` | **Unchanged** — uses phase-golem's `git.rs` | No task-golem involvement |
| `IngestInbox` | **Removed** | Items added via `tg add`; coordinator sees them on next `load_active()` read-through. `inbox_path` removed from `CoordinatorState`. All call sites (scheduler loop, tests) updated. |

**`spawn_coordinator` new signature:** Takes `Store` (constructed from project root), `project_root: PathBuf`, and `prefix: String`. Does not take `backlog_path` or `inbox_path` (removed). The caller (`main.rs`) constructs `Store::new(project_root.join(".task-golem"))` and passes it.

**Handler pattern (canonical):**

Mutation handlers operate on items inside `with_lock`. Since `PgItem(pub Item)` wraps an owned `Item`, handlers find the item's index, apply mutations via adapter free functions, and save:

```rust
async fn handle_update_item(store: &Store, id: String, update: ItemUpdate) -> Result<(), PgError> {
    let store = store.clone();
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
}
```

Note: `pg_item::apply_update(item: &mut Item, update: ItemUpdate)` is a **free function** in `pg_item.rs` that takes `&mut Item` directly — no `PgItem` wrapping needed for mutation. `PgItem` is used for read access (typed getters); mutation uses free functions to avoid the owned-vs-borrow tension. This keeps the adapter as a pure data accessor and places mutation logic in explicit, testable functions.

**LockTimeout retry pattern:**

No abstract retry wrapper — each handler implements retry inline. The pattern is a simple loop:

```rust
async fn with_store_retry<R>(
    store: &Store,
    op_name: &str,
    make_op: impl Fn(Store) -> Result<R, TgError>,
) -> Result<R, PgError>
where
    R: Send + 'static,
{
    for attempt in 0..3u32 {
        let store = store.clone();
        let result = spawn_blocking(move || make_op(store))
            .await
            .map_err(|e| PgError::InternalPanic(format!("{e:?}")))?;
        match result {
            Ok(r) => return Ok(r),
            Err(TgError::LockTimeout(_)) if attempt < 2 => {
                tracing::warn!("{op_name}: lock timeout, retry {}/3", attempt + 1);
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Err(e) => return Err(PgError::from(e)),
        }
    }
    Err(PgError::LockTimeout(Duration::from_secs(5)))
}
```

The `make_op` closure takes an owned `Store` (freshly cloned each attempt) and returns the result. This avoids `Clone` constraints on captured data — each retry reconstructs the closure with fresh cloned state.

#### Adapted Scheduler

**Purpose:** Unchanged scheduling logic operating on `PgItem` types instead of `BacklogItem`.

**Responsibilities:** Identical to current scheduler. Pure function `select_actions()` that reads item state and produces scheduling actions.

**Changes:**
- `select_actions(items: &[PgItem], ...)` instead of `&BacklogFile`
- Access native fields via delegating methods: `pg_item.id()`, `pg_item.title()`, `pg_item.dependencies()`, `pg_item.created_at()`
- Access extension fields via accessor methods: `pg_item.phase()`, `pg_item.impact()`, `pg_item.phase_pool()`
- `BacklogFile` type removed — scheduler receives a `Vec<PgItem>` from coordinator snapshot
- Dependency checking: absent IDs = met (archived/missing items) — unchanged logic
- WRK-035 HashMap lookup pattern adapted to `&HashMap<&str, &PgItem>`
- Field name change: `created: String` → `created_at: DateTime<Utc>`. Sort ordering changes from string comparison to `DateTime` comparison — functionally equivalent (RFC3339 strings sort correctly) but semantically cleaner.

**What does NOT change:**
- Sorting logic (impact, created_at)
- Phase ordering and progression
- Concurrency limits (max_wip, max_concurrent)
- Pipeline configuration lookups
- Promotion rules (New → Scoping, Scoping → Ready, etc.)

### Data Flow

1. **Scheduler requests snapshot** → Coordinator receives `GetSnapshot` command → `spawn_blocking` + `store.load_active()` → Wrap each `Item` as `PgItem` → Return `Vec<PgItem>`
2. **Scheduler calls `select_actions()`** → Pure computation over `&[PgItem]` → Returns `Vec<SchedulerAction>`
3. **Executor runs phase** → Produces `PhaseResult` (follow-ups, artifacts, status updates)
4. **Coordinator processes `CompletePhase`** → Stage artifact files via `git add` → Update item via `store.with_lock()` (load → mutate status/phase/assessments/description via adapter → save) → after lock release: `tg_git::stage_self()` → `tg_git::commit(message)` (if destructive) or accumulate in `pending_batch_phases` (if non-destructive)
5. **Coordinator processes `BatchCommit`** → `tg_git::stage_self()` → `tg_git::commit(batch_message)` → Clear `pending_batch_phases`

### Key Flows

#### Flow: Item State Update (e.g., status transition)

> Coordinator receives an `UpdateItem` command and persists the change via task-golem.

1. **Receive command** — `CoordinatorCommand::UpdateItem { id, update, reply }` arrives on mpsc channel
2. **Clone store + params** — Clone `Store` (cheap: `PathBuf`) and move `id`, `update` into closure
3. **spawn_blocking** — Enter blocking thread pool
4. **Acquire lock** — `store.with_lock(|s| { ... })` acquires exclusive file lock with 5s timeout
5. **Load** — `s.load_active()` reads `tasks.jsonl` into `Vec<Item>`
6. **Find + mutate** — Locate item by index, call `pg_item::apply_update(&mut items[idx], update)`
7. **Save** — `s.save_active(&items)` atomic-writes updated JSONL
8. **Reply** — Send `Ok(())` through oneshot

**Edge cases:**
- Item not found → `PgError::ItemNotFound` → logged, reply with error
- Invalid transition → `PgError::InvalidTransition` → logged at warn, reply with error. Caller (executor) receives error via `CoordinatorHandle` and treats the phase as failed.
- Lock timeout → Retry up to 3 times with 1s backoff → `PgError::LockTimeout` if exhausted
- Panic in closure → `PgError::InternalPanic` → fatal, coordinator drops receiver (see Fatal Error Propagation)

#### Flow: Phase Completion (Destructive)

> Executor finishes a destructive phase (e.g., build). Artifacts are staged, item state is updated, and everything is committed.

1. **Executor signals** — `CompletePhase { item_id, result, is_destructive: true, reply }`
2. **Stage artifacts** — `pg_git::stage_paths(&dirty_paths)` stages artifact files in the working tree. (Dirty paths are discovered via `git status` in the change directory, same as current implementation — not via a `PhaseResult.artifact_paths` field.)
3. **Update item state** — `spawn_blocking` + `store.with_lock()`: load items, apply status transition + phase update + description/assessment updates from `PhaseResult` via adapter free functions, save JSONL. Lock is released after save.
4. **Stage task-golem files** — After lock release: `tg_git::stage_self(project_dir)` stages `tasks.jsonl`
5. **Commit** — `tg_git::commit(message, project_dir)` commits all staged changes (artifacts + task state)
6. **Reply** — Send result through oneshot

**Edge cases:**
- `git::stage_paths` failure → Abort the entire operation, reply with `PgError::Git`. JSONL is not updated (staging happens before the `with_lock` call). No partial state.
- Git commit fails → JSONL is already updated (source of truth). Log warning, continue. The commit will be retried on the next `BatchCommit` or can be recovered manually.
- Concurrent `tg` command between lock release and commit → Their change may be included in phase-golem's commit. Accepted as human operator responsibility.

#### Flow: Phase Completion (Non-Destructive, Batched)

> Non-destructive phases (research, design) accumulate and commit as a batch.

1. **Executor signals** — `CompletePhase { item_id, result, is_destructive: false, reply }`
2. **Stage artifacts** — `pg_git::stage_paths(&dirty_paths)`
3. **Update item state** — Same as destructive: `spawn_blocking` + `store.with_lock()` + save. Lock released.
4. **Stage task-golem files** — `tg_git::stage_self(project_dir)` after lock release
5. **Accumulate** — Push `(item_id, phase_name, commit_sha)` to `pending_batch_phases`
6. **Reply** — Send `Ok(())`
7. **Later: BatchCommit** — `tg_git::commit(batch_message)` commits all accumulated staged changes. Clear `pending_batch_phases`.

**Edge cases:**
- BatchCommit with nothing staged → No-op. Clear `pending_batch_phases` regardless (they represent intent, not git state).
- Crash between non-destructive `CompletePhase` and `BatchCommit` → JSONL is updated (correct state), but staged files remain uncommitted in the git index. On restart, phase-golem reads JSONL as source of truth (correct). Orphaned staged changes are either committed on next `BatchCommit` or left for operator to clean up manually via `git reset HEAD`. This is a known gap — not addressed in this migration.

#### Flow: Unblock Item

> An item is unblocked (restored to its pre-blocked status).

1. **Receive command** — `CoordinatorCommand::UnblockItem { item_id, context, reply }`
2. **spawn_blocking + with_lock** — Load active items, find item
3. **Validate** — Verify item is `Blocked` (otherwise reply with `PgError::InvalidTransition`)
4. **Read restore target** — Read `x-pg-blocked-from-status` via `pg_item::pg_blocked_from_status(&item)` — this is the **authoritative** restore target (not `Item.blocked_from_status`, which holds a lossy 4-variant mapping)
5. **Restore status** — Set status to the restore target via `pg_item::set_pg_status(&mut item, restore_target)`. Clear `x-pg-blocked-type`, `x-pg-blocked-from-status`, `x-pg-unblock-context` (set to provided context), and `x-pg-last-phase-commit` (forces re-check on next phase)
6. **Call `Item.apply_unblock()`** — Also call task-golem's native unblock to clear native `blocked_from_status` and `blocked_reason`
7. **Save** — `s.save_active(&items)`
8. **Reply** — Send `Ok(())`

#### Flow: Record Phase Start

> Executor is about to run a phase. Record the current HEAD SHA for staleness detection.

1. **Receive command** — `RecordPhaseStart { item_id, reply }`
2. **Get HEAD SHA** — `pg_git::get_head_sha()`
3. **spawn_blocking + with_lock** — Load items, find item, set `x-pg-last-phase-commit` to HEAD SHA, save
4. **Reply** — Send `Ok(())`

#### Flow: Follow-Up Ingestion

> Phase result includes follow-up items to create in the backlog.

1. **Receive command** — `IngestFollowUps { follow_ups, origin, reply }`
2. **spawn_blocking + with_lock** — Load active items. Call `store.all_known_ids()` **once** to get the full `HashSet<String>` of active + archived IDs for collision avoidance.
3. **Generate IDs** — For each follow-up: `generate_id_with_prefix(&all_ids, "WRK")` and add the new ID to the local set (prevents collisions between follow-ups in the same batch)
4. **Create Items** — For each follow-up, construct a new `Item` with: `created_at`/`updated_at` = `Utc::now()`, `priority` = 0, `status` = `Todo`, extensions set via adapter free functions (`x-pg-status` = `"new"`, `x-pg-origin` = origin, plus any size/risk/pipeline_type from `FollowUp` fields)
5. **Save** — Append new items to active list, save via `store.save_active()`
6. **Reply** — Return new item IDs

**Edge cases:**
- ID collision within the lock is impossible — `generate_id_with_prefix` checks against the full ID set (active + archive), and the lock prevents concurrent ID generation. This is stated explicitly for implementer clarity.

#### Flow: Item Archival

> Completed item is archived (moved from active to archive store).

1. **Receive command** — `ArchiveItem { item_id, reply }`
2. **spawn_blocking + with_lock** — Load active items, find target
3. **Archive** — `store.append_to_archive(&item)` appends to `archive.jsonl`
4. **Remove** — Remove from active items list
5. **Save** — `store.save_active(&remaining_items)`
6. **Worklog** — Write worklog entry (phase-golem-specific, outside task-golem and outside the lock)
7. **Reply** — Send `Ok(())`

**Edge cases:**
- Item not found → `PgError::ItemNotFound` → reply with error
- Dependencies on archived items — Scheduler treats as satisfied (absent IDs = met). No explicit dependency stripping.

#### Flow: Item Merge

> Two items are merged (source absorbed into target).

1. **Receive command** — `MergeItem { source_id, target_id, reply }`
2. **spawn_blocking + with_lock** — Load active items
3. **Find both** — Locate source and target by index. If either not found, reply with `PgError::ItemNotFound`.
4. **Merge via adapter** — Append source description to target (via `StructuredDescription` merge), union dependencies, transfer tags
5. **Archive source** — `store.append_to_archive(&source)`
6. **Save** — `store.save_active(&items)` with source removed and target updated
7. **Reply** — Send `Ok(())`

**Edge cases:**
- Source not found → `PgError::ItemNotFound(source_id)`, reply with error
- Target not found → `PgError::ItemNotFound(target_id)`, reply with error
- Cycle detected during dependency merge → `PgError::CycleDetected`, reply with error (skip, log warning)

#### Flow: Coordinator Startup

> Phase-golem initializes the coordinator with task-golem store validation.

1. **Preflight check** — `src/preflight.rs` checks for `.task-golem/` directory existence. If missing, exit with clear error: "task-golem store not initialized. Run `tg init` in the project root."
2. **Construct Store** — `Store::new(project_root.join(".task-golem"))`
3. **Probe read** — `spawn_blocking` + `store.with_lock(|s| s.load_active())`. If `SchemaVersionUnsupported` → exit with: "task-golem schema version X is not supported; upgrade task-golem." If other fatal error → exit with error details.
4. **Spawn coordinator** — Create mpsc channel, spawn actor loop with `Store`
5. **Return handle** — `CoordinatorHandle` ready for use

#### Flow: Shutdown Commit

> Phase-golem is shutting down. Ensure any staged-but-uncommitted state is committed.

1. **Scheduler loop exits** — Normal exit or error
2. **Check `pending_batch_phases`** — If non-empty, issue `BatchCommit` to flush staged changes
3. **Check git status** — If `tasks.jsonl` is dirty (modified but not staged), stage and commit with a shutdown message
4. **Exit** — Clean shutdown

---

## Technical Decisions

### Key Decisions

#### Decision: Newtype Wrapper (PgItem) over From/Into Conversion

**Context:** Need typed access to ~15 extension fields stored in task-golem's `BTreeMap<String, Value>`.

**Decision:** Use `PgItem(pub Item)` newtype with accessor methods for reads. Use free functions (`pg_item::apply_update`, `pg_item::set_pg_status`, etc.) for mutations on `&mut Item`.

**Rationale:** The `Item` is the source of truth in JSONL. The newtype preserves all native fields and unknown extensions during round-trips. Accessor methods provide typed access without converting to a separate struct. The inner `Item` is directly passable to `store.save_active()`. Free functions for mutation avoid the owned-vs-borrow tension in `with_lock` closures.

**Consequences:** Callers access phase-golem fields via methods (`pg_item.phase()`) and native fields via delegates (`pg_item.id()`). Mutations use free functions: `pg_item::apply_update(&mut items[idx], update)`.

#### Decision: Read-Through Store Access (No In-Memory Cache)

**Context:** Current coordinator holds an in-memory `BacklogFile` and syncs it to disk. Migration could keep this pattern or switch to read-through.

**Decision:** Read from disk (via task-golem store) on every operation. No in-memory backlog cache.

**Rationale:** Eliminates cache-consistency bugs. Task-golem's JSONL load for tens of items is sub-millisecond. The coordinator's actor serialization ensures no concurrent reads/writes from phase-golem's side. A human running `tg add` at any time is visible on the next `load_active()` read-through — this makes `tg add` equivalent to the old inbox mechanism with no drop-file ceremony.

**Consequences:** Every coordinator command incurs a disk read. At current scale (~50 items) this is negligible. `pending_batch_phases` remains as the only in-memory state — it accumulates non-destructive commit metadata that is not persisted to task-golem.

#### Decision: Explicit Error Variant Mapping

**Context:** Need to classify task-golem errors as retryable, fatal, or skippable.

**Decision:** Manual `From<TgError>` implementation that maps **every** `TgError` variant to a specific `PgError` variant (no catch-all match arm), plus `is_retryable()` / `is_fatal()` methods.

**Rationale:** The exhaustive match ensures compilation breaks when task-golem adds new error variants, forcing an explicit handling decision. Preserving `TgError` as `#[source]` on key variants maintains the error chain for debugging.

**Consequences:** `From<TgError>` must be updated when task-golem's error enum changes. This is a feature: it forces explicit handling decisions.

#### Decision: Lock Scope — Release After JSONL Save, Before Git Operations

**Context:** Phase completion involves: update item state in JSONL → stage `tasks.jsonl` → commit. Should the file lock be held through the entire sequence?

**Decision:** Release the lock after saving JSONL. Git staging (`stage_self`) and commit happen **outside** the lock.

**Rationale:** Holding the lock through git operations (which can be slow) would block `tg` CLI usage. The coordinator actor already serializes phase-golem's operations, so no interleaving from phase-golem's side. `stage_self` and `commit` are called sequentially after the lock is released.

**Consequences:** The git index is briefly in a state where `tasks.jsonl` is staged but not committed. A concurrent `tg` operation could modify `tasks.jsonl` again before the commit. The commit message would still reflect the phase-golem operation, and the next phase-golem operation would see the human's change on its next read-through.

#### Decision: No Deref to Item on PgItem — Thin Delegates Instead

**Context:** The scheduler accesses 8+ fields from items. `Deref<Target=Item>` would allow transparent field access, but `.0.field` is noisy.

**Decision:** Do not implement `Deref`. Add thin delegating accessor methods for commonly-used native fields (`id()`, `title()`, `status()`, `dependencies()`, `tags()`, `created_at()`, `updated_at()`). These are one-liner methods that return `&self.0.field`.

**Rationale:** `Deref` to a foreign type is a Rust anti-pattern. Thin delegates provide a uniform API surface (all field access via methods) while keeping the data model boundary visible. The call sites read cleanly: `pg_item.id()` instead of `pg_item.0.id`.

**Consequences:** Scheduler code uses `pg_item.id()`, `pg_item.impact()`, `pg_item.dependencies()` uniformly. No `.0` access in caller code.

#### Decision: Store Initialization is a Prerequisite

**Context:** task-golem requires `.task-golem/` directory to exist before the store can be used.

**Decision:** User runs `tg init` before using phase-golem. Phase-golem does not auto-initialize the store. The check lives in `src/preflight.rs`.

**Rationale:** `tg init` is a one-time operation. Auto-initialization would add complexity and hide the dependency. Failing fast with a clear error message when `.task-golem/` is missing is simpler and more explicit.

**Consequences:** `preflight.rs` checks for `.task-golem/` existence (alongside existing git preconditions). Coordinator startup does a probe read to validate schema compatibility.

#### Decision: Process-Level Lock Retained

**Context:** Phase-golem has `src/lock.rs` using `fslock` for a `.phase-golem/phase-golem.lock` process-level exclusive lock, preventing multiple orchestrator instances.

**Decision:** Retain `src/lock.rs` and the `fslock` dependency unchanged. This is a separate concern from task-golem's storage-level file lock.

**Rationale:** The process-level lock prevents multiple phase-golem instances; task-golem's `fd-lock` prevents concurrent JSONL access. These serve different purposes and coexist. `fslock` removal from `Cargo.toml` applies only if `lock.rs` itself is refactored to use `fd-lock` (deferred — separate hygiene concern).

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Extension field verbosity | ~15 fields stored as `x-pg-*` keys in a BTreeMap | Single source of truth (JSONL), no parallel struct | Adapter layer abstracts this; internal code never touches raw extensions |
| Status model mismatch | Phase-golem's 6-state model encoded via 4-state + extension | Compatibility with task-golem's simpler model; `tg list`/`tg show` work | Human-initiated `tg` transitions that skip states are operator responsibility |
| Disk read per operation | Every coordinator command loads from JSONL | No cache-consistency bugs; human `tg` changes visible immediately | Sub-millisecond at current scale (~50 items) |
| Git index interleaving | Between JSONL save and commit, concurrent `tg` could modify state | Short lock hold time; `tg` CLI remains responsive | Coordinator serialization covers phase-golem's own operations |
| StructuredDescription as JSON | `tg show` displays raw JSON for description | Full structured description preserved in task-golem | Native `Item.description` populated with `context` field for CLI readability |
| serde_yaml version mismatch | Binary includes both `serde_yaml` 0.9 (task-golem) and `serde_yaml_ng` 0.10 (phase-golem) | No type conflicts (YAML types don't cross boundary) | Binary size increase is minor; cleanable in separate task-golem hygiene pass |
| CLI deps in library tree | task-golem's `clap`, `owo-colors`, `clap_complete` become transitive deps for phase-golem | Simpler project structure (no workspace split) | Adds compile time but no runtime cost; workspace separation is overkill for single-consumer |
| New hex ID format | New items use `WRK-a1b2c` (hex) instead of `WRK-001` (sequential) | Collision-safe IDs; no high-water-mark state to maintain | Mixed ID formats coexist; existing numeric refs remain valid |

---

## Alternatives Considered

### Alternative 1: Full-Conversion Adapter (Pattern B from Research)

**Summary:** Define a standalone `PgItem` struct with all fields typed natively (no extension BTreeMap). Implement `From<Item> for PgItem` and `From<PgItem> for Item` for conversion at every boundary.

**How it would work:**
- `PgItem` has native Rust fields for every phase-golem concept (phase, size, risk, etc.)
- Converting from `Item` → `PgItem` deserializes all extensions into typed fields
- Converting back `PgItem` → `Item` serializes typed fields back into extensions
- Scheduler works with `PgItem` as a plain struct with direct field access

**Pros:**
- Cleanest API for scheduler — direct struct field access, no method calls
- Complete type safety — no `Option` wrapping on extension reads
- Familiar Rust struct ergonomics

**Cons:**
- Lossy round-trips: unknown extensions or newly-added task-golem fields could be dropped during `PgItem` → `Item` conversion
- Must duplicate all native `Item` fields (id, title, status, tags, dependencies, etc.) in `PgItem` — divergence risk when `Item` gains new fields
- Conversion cost on every boundary crossing (every load, every save)
- Requires keeping two parallel type definitions in sync

**Why not chosen:** The newtype approach (wrapping `Item`) is strictly safer for round-trips and lower maintenance. The inner `Item` is always intact and passable to store operations without conversion.

### Alternative 2: Trait-Based Adapter

**Summary:** Define a `PgItemExt` trait that adds typed extension accessors as extension methods on `&Item` and `&mut Item`.

**How it would work:**
- `trait PgItemExt` implemented for `Item` with methods like `fn pg_status(&self) -> ItemStatus`
- Scheduler uses `Item` directly with extension trait in scope
- No wrapper type — just import the trait

**Pros:**
- No wrapper type to construct/destructure
- Native `Item` fields accessible directly alongside extension methods
- Slightly less boilerplate than newtype

**Cons:**
- Trait methods pollute `Item`'s API in any module that imports the trait
- Cannot enforce invariants (e.g., "always set `x-pg-status` when setting `Status::Todo`") — the `Item` can be mutated directly bypassing the trait setters
- Cannot override behavior of `Item`'s native methods (e.g., `apply_do` doesn't know about `x-pg-status`)
- Less discoverable — new developers must know to import the trait

**Why not chosen:** The newtype provides a stronger boundary. Status transitions require coordinating both `Status` and `x-pg-status` — a wrapper type can enforce this invariant; a trait on `Item` cannot.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Coordinator rewrite introduces regressions | High — coordinator is the nerve center; every state mutation flows through it | Medium — 12 command handlers to rewrite from in-memory to read-through | Rewrite one handler at a time. Preserve existing tests (adapted to new types). Integration test each handler before moving to the next. |
| Status mapping bugs | High — incorrect mapping means items skip pipeline stages or get stuck | Medium — bidirectional mapping with defaults and edge cases | Comprehensive unit tests for every status combination. Property-based test: roundtrip (PgItem → Item → PgItem) preserves status. |
| Git commit sequencing interleave | Medium — a concurrent `tg` command could be included in phase-golem's commit | Low — requires human running `tg` in the narrow window between JSONL save and commit | Accepted risk per PRD. Coordinator serializes phase-golem's operations. Document for human operators. |
| Test migration volume | Medium — 16 test files need type updates; missing a test means silent coverage loss | High — volume is large, easy to miss edge cases | Delete obsolete tests (`backlog_test.rs`, `migration_test.rs`). Adapt scheduler tests first (highest value). Create new adapter tests. |
| Lock contention under batch operations | Medium — multiple follow-up ingestions in sequence could contend with human `tg` usage | Low — coordinator serializes phase-golem operations; lock duration is sub-millisecond for tens of items | 3-retry with 1s backoff for LockTimeout. Coordinator serialization prevents self-contention. |
| WRK-035 merge conflict | Medium — uncommitted scheduler.rs changes conflict with type migration | High — both touch the same functions | Commit WRK-035 first. Its HashMap pattern is compatible with adapter types. |
| `pending_batch_phases` lost on crash | Medium — staged-but-uncommitted git state after process crash | Low — only affects non-destructive phase batches | Documented as known gap. On restart, JSONL is correct (source of truth). Orphaned staged files are harmless — committed on next `BatchCommit` or cleaned by operator. |
| Stale `x-pg-status` after `tg` CLI transition | Low — human runs `tg do` on New item, leaving stale `"new"` in extensions | Low — requires specific CLI sequence + return to Todo | Adapter ignores `x-pg-status` when status is not Todo. Documented as operator responsibility. |

---

## Integration Points

### Existing Code Touchpoints

- `src/coordinator.rs` — Major rewrite: remove in-memory `BacklogFile`, add `Store`, `spawn_blocking`, retry logic, new error handling. All 12 command handlers updated (see Command Set Changes table). `CoordinatorHandle` methods return `Result<T, PgError>`.
- `src/scheduler.rs` — Moderate: `select_actions()` parameter changes from `&BacklogFile` to `&[PgItem]`. All sorting/filtering/dependency functions updated for `PgItem` accessor methods. WRK-035 HashMap changes adapted. `created` → `created_at` field name change.
- `src/types.rs` — Remove `BacklogItem`, `BacklogFile`, `InboxItem`. Keep `ItemStatus`, `StructuredDescription`, `ItemUpdate`, `FollowUp`, `SizeLevel`, `DimensionLevel`, `BlockType`, `PhasePool`, `PhaseResult`.
- `src/backlog.rs` — **Delete entirely**.
- `src/migration.rs` — **Delete entirely**.
- `src/config.rs` — Remove `backlog_path` from `ProjectConfig`. Store path derived from project root (`.task-golem/`).
- `src/executor.rs` — Minor: `PhaseResult` handling unchanged; coordinator interface changes are transparent via `CoordinatorHandle`.
- `src/preflight.rs` — Add `.task-golem/` directory existence check alongside existing git preconditions.
- `src/lock.rs` — **Unchanged**. Process-level lock (`fslock`) retained.
- `src/worklog.rs` — Update `write_entry` signature: takes `(id: &str, title: &str, phase, outcome, summary)` instead of `&BacklogItem`. `WriteWorklog` command variant updated to match.
- `src/filter.rs` — Update `apply_filters` to take `&[PgItem]` instead of `&BacklogFile`.
- `src/git.rs` — **Unchanged**. Phase-golem retains its own git operations (`stage_paths`, `commit`, `get_head_sha`, `is_ancestor`). Task-golem's git module is separate (stages JSONL files only).
- `src/main.rs` — `handle_run` updated: construct `Store`, pass to `spawn_coordinator`. Shutdown commit logic updated. CLI subcommands (`handle_status`, `handle_add`, `handle_advance`, `handle_unblock`, `handle_triage`) updated to use `Store` + `PgItem` instead of `BacklogFile`. ID validation updated to accept hex format (`WRK-a1b2c`) in addition to numeric.
- `src/lib.rs` — Remove `backlog` and `migration` module declarations. Add `pg_item` and `pg_error`.
- `tests/common/mod.rs` — Rewrite test helpers: `make_item()` → creates `PgItem`, `make_backlog()` → creates `Vec<PgItem>`, `setup_test_env()` → initializes `.task-golem/` store via `tg init` or direct directory setup.
- `tests/` — Delete `backlog_test.rs`, `migration_test.rs`. Adapt all scheduler/coordinator tests. Add new adapter tests (status mapping, extension round-trips, validation of invalid values).
- `Cargo.toml` — Add `task_golem = { path = "../task-golem" }` dependency. Keep `fslock` (still used by `lock.rs`). Keep `serde_yaml_ng` for non-backlog YAML (config files, etc.).

### External Dependencies

- `task-golem` crate (path dependency at `../task-golem`) — Must have `lib.rs`, git module, and `#[derive(Clone)]` on `Store` before phase-golem migration starts (Part 1 → Part 2 sequencing)
- `.task-golem/` directory — Must exist in project root (user runs `tg init`)

### Cutover Procedure

The existing backlog (WRK-001 through WRK-075) is stale and not migrated. Operator procedure at cutover:

1. Stop the orchestrator
2. Run `tg init` in the project root
3. Fresh start — add active items via `tg add` if any need to continue
4. Restart the orchestrator
5. New items enter the pipeline at `New` status (default for absent `x-pg-status`)

---

## Open Questions

*(None — all resolved during review. See Design Log for resolutions.)*

## Future Direction: Status Model Simplification

Phase-golem's 6-state model (`New`, `Scoping`, `Ready`, `InProgress`, `Done`, `Blocked`) was designed before task-golem existed. With task-golem's 4-state model (`Todo`, `Doing`, `Done`, `Blocked`) as the storage layer, there's an opportunity to simplify: treat `Scoping` and `Ready` not as special statuses but as regular pipeline phases that happen to come first. The "what stage is this item at?" question would be answered by `x-pg-phase` + `x-pg-phase-pool`, not by the status enum.

This would:
- Reduce the status mapping surface area (4 states map 1:1)
- Make `blocked_from_status` work natively (all active states are `Todo` or `Doing`)
- Simplify the adapter (no `x-pg-status` extension needed for sub-states)
- Treat pre-phases (triage, scoping) as first-class pipeline phases

**Not in scope for WRK-076** — the storage migration is already large. This should be a follow-up item once the migration is stable and the scheduler can be reworked to use phase-based progression instead of status-based promotion.

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
| 2026-02-25 | Initial design draft | Full design with recommended approach (newtype adapter), 2 alternatives, 6 key flows |
| 2026-02-25 | Self-critique (7 agents) | 29 auto-fixes applied: fixed stage_self lock contradiction, replaced broken with_retry, added 4 missing flows (UnblockItem, RecordPhaseStart, Startup, Shutdown), exhaustive TgError mapping, resolved PgItem mutation pattern (free functions), added thin delegate accessors, expanded integration points, resolved open question (all extensions return Option<T>). 6 directional items and 8 quality items presented for review. |
| 2026-02-26 | Directional review | Resolved all 6 directional items: (1) git commit failure → keep log-and-continue, (2) blocked_from_status → keep dual field (native lossy + extension authoritative), (3) pending_batch_phases → accept crash gap, (4) path dependency → keep for now, switch to git dep post-migration, (5) apply_update → free functions in pg_item.rs, (6) extension schema → defer consolidation. Added "Future Direction: Status Model Simplification" section — treat pre-phases as regular pipeline phases, defer to post-migration follow-up. |
