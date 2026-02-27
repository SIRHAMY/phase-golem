# Tech Research: Migrate Phase-Golem Storage to Task-Golem

**ID:** WRK-076
**Status:** In Review
**Created:** 2026-02-25
**PRD:** ./WRK-076_migrate-storage-to-task-golem_PRD.md
**Mode:** Medium

## Overview

Researching patterns and approaches for migrating phase-golem's storage layer from a custom YAML-based backlog to task-golem as a Rust library dependency. Key areas: exposing a Rust crate as both binary and library, adapter layers for mapping between data models with extension fields, async/sync bridging for file-locking APIs, and cross-repo dependency management. Need to understand what both codebases currently look like and what patterns exist for this kind of integration.

## Research Questions

- [x] How should task-golem expose its internals as a library crate alongside the existing binary?
- [x] What patterns exist for adapter layers that map between two different data models with extension fields?
- [x] How should phase-golem bridge async code with task-golem's synchronous `with_lock()` API?
- [x] What are best practices for cross-repo Rust path dependencies?
- [x] What does the current phase-golem storage layer look like (backlog.rs, migration.rs, coordinator.rs)?
- [x] What does task-golem's store/model API look like today?
- [x] How should error types be mapped across crate boundaries?

---

## External Research

### 1. Rust Binary + Library Crate Pattern

#### Landscape Overview

Cargo natively supports packages containing both a library crate (`src/lib.rs`) and one or more binary crates (`src/main.rs` or `src/bin/*.rs`). A package can have at most one library crate and any number of binary crates. The binary automatically has access to the library's public API without explicit dependency configuration -- Cargo links them together. This is a well-established pattern used extensively in the Rust ecosystem for CLI tools that want to expose their core logic as a reusable library.

#### Patterns Found

**Pattern A: Same-Package lib.rs + main.rs**

How it works: Add a `[lib]` section to `Cargo.toml` alongside the existing `[[bin]]` section. Create `src/lib.rs` that re-exports the desired public modules. The binary (`src/main.rs`) imports the library by its crate name (with dashes converted to underscores).

Cargo.toml configuration:
```toml
[package]
name = "task-golem"
version = "0.1.0"

[lib]
name = "task_golem"
path = "src/lib.rs"

[[bin]]
name = "tg"
path = "src/main.rs"
```

The `src/lib.rs` file selectively re-exports modules:
```rust
pub mod model;
pub mod store;
pub mod errors;
```

The binary imports via `use task_golem::store::Store;`.

When to use: When the library and binary share the same dependency tree and the binary is a thin CLI wrapper around library functionality. This is the most common pattern and the simplest to set up.

Tradeoffs:
- (+) Zero configuration overhead -- Cargo handles linking automatically
- (+) Single `Cargo.toml`, single `Cargo.lock`, shared dependency versions
- (+) The binary automatically sees the library's public API
- (-) Library and binary share the same dependency set (binary-only deps like `clap` become library deps too, though they are not re-exported)
- (-) `pub` visibility is all-or-nothing per module -- need `pub(crate)` for internal-only items

**Pattern B: src/lib.rs + src/bin/ Directory**

How it works: Place the library root at `src/lib.rs` and binaries under `src/bin/`. Each file in `src/bin/` becomes a separate binary target. This avoids having two "root" files at the same level.

When to use: When you have multiple binaries that all depend on the same library, or when you want clearer separation between "this is the library" and "these are CLI entry points."

Tradeoffs:
- (+) Cleaner separation of concerns
- (+) Scales to multiple binaries naturally
- (-) Slightly more directory structure to navigate
- (-) For a single binary, adds a level of indirection without much benefit

**Pattern C: Cargo Workspace with Separate Packages**

How it works: Create a workspace with two member packages: one library crate and one binary crate. Each has its own `Cargo.toml` with independent dependencies.

When to use: When the binary needs significantly different dependencies than the library (e.g., the binary needs `clap`, `owo-colors`, `clap_complete` but the library should not pull these in). Also useful when you want independent versioning.

Tradeoffs:
- (+) Clean dependency separation -- library consumers do not pull in CLI dependencies
- (+) Independent versioning and publishing
- (-) More complex project structure
- (-) Overkill for this use case where the consumer (phase-golem) already has its own dependency management

**Pattern D: Selective Re-export with pub use**

How it works: Instead of making entire modules public, selectively re-export specific types and functions at the `lib.rs` level using `pub use`. This creates a curated public API surface.

```rust
// lib.rs
mod model;
mod store;
mod errors;

pub use errors::TgError;
pub use model::item::Item;
pub use model::status::Status;
pub use model::id::generate_id_with_prefix;
pub use store::Store;
```

When to use: When you want tight control over what consumers can access, hiding internal module structure.

Tradeoffs:
- (+) Minimal public API surface -- consumers see only what you intend
- (+) Internal restructuring does not break consumers
- (-) More maintenance burden keeping the re-export list current
- (-) For this use case, phase-golem needs fairly deep access to `model`, `store`, and `errors`, so full module re-export is simpler

#### Technologies & Tools

- **Cargo targets**: Built-in support via `[lib]` and `[[bin]]` sections. Well-documented, stable, no additional dependencies. [Cargo Targets Reference](https://doc.rust-lang.org/cargo/reference/cargo-targets.html)
- **`pub(crate)` visibility**: Use for items that should be accessible within the crate but not to library consumers. Zero-cost, built into the language.

#### Common Pitfalls

- **Forgetting `pub` on modules/types**: If `src/lib.rs` declares `mod model;` without `pub`, the module is private to the library and invisible to both the binary and external consumers.
- **Crate name vs package name**: The library crate name uses underscores (e.g., `task_golem`) while the package name may use dashes (`task-golem`). The binary imports using the underscore form.
- **Binary-only dependencies**: Dependencies like `clap` become part of the library's dependency tree even if only the binary uses them. This is usually harmless (they are not re-exported) but increases compile time for library-only consumers. Workspace separation is the clean fix but is overkill here.
- **Breaking the binary**: Adding `[lib]` and `src/lib.rs` can break `src/main.rs` if it still uses `mod model;` directly -- it must switch to `use task_golem::model;` for the shared modules.

#### Standards/Best Practices

- Start with Pattern A (same-package lib.rs + main.rs) unless you have a compelling reason for workspace separation.
- Use `pub mod` for modules that consumers need full access to; use `pub use` for a curated API surface.
- Mark internal-only items with `pub(crate)` to prevent accidental exposure.
- Re-export dependency types that appear in your public API (see [Effective Rust Item 24](https://www.lurklurk.org/effective-rust/re-export.html)).

#### Key References

- [Cargo Targets - The Cargo Book](https://doc.rust-lang.org/cargo/reference/cargo-targets.html) -- Authoritative reference for `[lib]` and `[[bin]]` configuration
- [Packages and Crates - The Rust Programming Language](https://doc.rust-lang.org/book/ch07-01-packages-and-crates.html) -- Explains the relationship between packages, crates, and modules
- [Crate Layout Best Practices - DEV Community](https://dev.to/sgchris/crate-layout-best-practices-librs-modrs-and-srcbin-4abd) -- Practical guide to layout patterns
- [thoughts on the src/main.rs and src/lib.rs pattern - API Guidelines Discussion #167](https://github.com/rust-lang/api-guidelines/discussions/167) -- Community discussion on tradeoffs between approaches
- [Re-export dependencies whose types appear in your API - Effective Rust](https://www.lurklurk.org/effective-rust/re-export.html) -- When and how to re-export dependency types

#### Initial Recommendations

**Use Pattern A (same-package lib.rs + main.rs)** with `pub mod` re-exports. This is the simplest approach and matches the PRD's requirements exactly. task-golem's current structure already has clean module boundaries (`model`, `store`, `errors`). The change is:

1. Add `[lib]` section to `Cargo.toml` with `name = "task_golem"`
2. Create `src/lib.rs` with `pub mod model; pub mod store; pub mod errors;`
3. Update `src/main.rs` to remove direct `mod` declarations for shared modules and import from the library crate instead (the `cli` module stays as a private `mod cli;` in `main.rs` since it is binary-only)

No workspace restructuring needed. The CLI-only dependencies (`clap`, `owo-colors`, `clap_complete`) will be in the shared dependency tree but are not re-exported and will not affect phase-golem.

---

### 2. Adapter/Wrapper Layers Between Data Models

#### Landscape Overview

When one system stores data in a generic extensible format (like key-value extension fields) and another system needs typed access to that data, an adapter layer provides the translation. In Rust, this is typically implemented using a combination of the newtype pattern, `From`/`Into` trait implementations, and serde custom deserialization. The challenge here is specific: task-golem stores items with `BTreeMap<String, serde_json::Value>` extension fields, and phase-golem needs typed access to ~15 fields stored as `x-pg-*` keys.

#### Patterns Found

**Pattern A: Newtype Wrapper with Accessor Methods**

How it works: Define a newtype struct that wraps the upstream type and provides typed getter/setter methods for the extension fields. The newtype owns the inner value and exposes domain-specific accessors.

```rust
pub struct PgItem(pub task_golem::model::item::Item);

impl PgItem {
    pub fn phase(&self) -> Option<Phase> {
        self.0.extensions.get("x-pg-phase")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    pub fn set_phase(&mut self, phase: Phase) {
        self.0.extensions.insert(
            "x-pg-phase".to_string(),
            serde_json::to_value(phase).unwrap(),
        );
    }

    pub fn pg_status(&self) -> ItemStatus {
        // Bidirectional status mapping logic
        match self.0.status {
            Status::Todo => {
                self.0.extensions.get("x-pg-status")
                    .and_then(|v| v.as_str())
                    .map(|s| match s {
                        "new" => ItemStatus::New,
                        "scoping" => ItemStatus::Scoping,
                        "ready" => ItemStatus::Ready,
                        _ => ItemStatus::New, // default for unknown values
                    })
                    .unwrap_or(ItemStatus::New) // absent defaults to New
            }
            Status::Doing => ItemStatus::InProgress,
            Status::Done => ItemStatus::Done,
            Status::Blocked => ItemStatus::Blocked,
        }
    }
}
```

When to use: When you need ergonomic typed access to extension fields and want to hide the raw JSON manipulation from callers. This is the primary pattern for this use case.

Tradeoffs:
- (+) Callers never touch raw `serde_json::Value` -- all access is through typed methods
- (+) Validation can happen in the accessors (invalid values treated as absent with warning)
- (+) The inner `Item` is still accessible for passing to task-golem store operations
- (-) Each field requires a getter and setter method -- boilerplate for ~15 fields
- (-) The newtype must be explicitly constructed/destructured at boundaries

**Pattern B: From/Into Trait Implementations for Conversion**

How it works: Define separate phase-golem-native types and implement `From<Item>` / `Into<Item>` to convert between them. Each conversion handles the extension field mapping.

```rust
pub struct PgItem {
    pub id: String,
    pub title: String,
    pub status: ItemStatus,
    pub phase: Option<Phase>,
    pub description: Option<StructuredDescription>,
    // ... all typed fields
}

impl From<Item> for PgItem {
    fn from(item: Item) -> Self {
        PgItem {
            id: item.id,
            title: item.title,
            status: map_status_from_tg(&item),
            phase: item.extensions.get("x-pg-phase")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
            // ...
        }
    }
}

impl From<PgItem> for Item {
    fn from(pg: PgItem) -> Self {
        let mut extensions = BTreeMap::new();
        if let Some(phase) = pg.phase {
            extensions.insert("x-pg-phase".to_string(), serde_json::to_value(phase).unwrap());
        }
        // ...
        Item {
            id: pg.id,
            title: pg.title,
            status: map_status_to_tg(&pg.status),
            extensions,
            // ...
        }
    }
}
```

When to use: When you want a completely separate type hierarchy and are willing to pay the conversion cost at every boundary crossing.

Tradeoffs:
- (+) Phase-golem code works with fully typed structs -- no Option/JSON handling
- (+) Clean separation between the two data models
- (-) Lossy round-trips possible if the conversion drops unknown extensions
- (-) Must convert back to `Item` before every store operation -- repeated allocation
- (-) Native `Item` fields (tags, dependencies, description) must be duplicated in `PgItem`
- (-) Risk of divergence if `Item` gains new fields

**Pattern C: Hybrid Newtype with Lazy Deserialization**

How it works: Newtype wrapper that lazily deserializes extension fields on first access and caches the results. Extensions are only serialized back to the inner `Item` when explicitly flushed.

When to use: When extension field parsing is expensive and you want to avoid repeated deserialization.

Tradeoffs:
- (+) Amortizes deserialization cost
- (-) Adds interior mutability (RefCell or similar) for caching
- (-) Complexity not justified for this use case (extension fields are simple scalar values, not large nested objects)

**Pattern D: Serde Custom Deserializer for Extensions**

How it works: Implement a custom serde `Deserialize` that reads the raw JSON and maps `x-pg-*` fields directly into typed struct fields during deserialization.

When to use: When you control the deserialization path and want zero intermediate representations.

Tradeoffs:
- (+) Single-pass deserialization into typed fields
- (-) Complex to implement and maintain
- (-) Does not work well here because task-golem's `Item` is the canonical serde target and phase-golem should not redefine it

#### Technologies & Tools

- **serde `#[serde(flatten)]`**: Already used by task-golem's `Item` to capture extension fields into `BTreeMap<String, serde_json::Value>`. [Serde Flatten Reference](https://serde.rs/attr-flatten.html)
- **Newtype pattern**: Zero runtime overhead wrapper type. [Effective Rust: Embrace the newtype pattern](https://www.lurklurk.org/effective-rust/newtype.html)
- **`serde_json::from_value` / `to_value`**: For converting between `serde_json::Value` and typed Rust structs in extension fields.

#### Common Pitfalls

- **Round-trip extension loss**: If converting `Item -> PgItem -> Item`, unknown extensions (not `x-pg-*`) could be dropped if `PgItem` does not carry them through. The newtype wrapper (Pattern A) avoids this because it wraps the original `Item`.
- **Validation on deserialization**: Extension values could be invalid (e.g., `x-pg-status: "running"` instead of `"new"`). Accessors should treat invalid values as absent with a warning log, not panic.
- **`serde(flatten)` quirks**: The flatten attribute on BTreeMap can cause issues with nested flattening and makes the struct serialize/deserialize as a map internally. This is already handled in task-golem's `Item` and should not be modified.
- **StructuredDescription as JSON**: The `x-pg-description` field stores a JSON object `{context, problem, solution, impact, sizing_rationale}`. This round-trips cleanly through `serde_json::from_value::<StructuredDescription>()` and `serde_json::to_value()` but displays as raw JSON in `tg show`.

#### Standards/Best Practices

- Prefer the newtype wrapper (Pattern A) when the underlying type is the source of truth and you want typed views over it.
- Implement `From`/`Into` only when you need genuinely separate types with different semantics.
- Define extension key names as constants to avoid string typos.
- Validate extension values defensively: invalid values should be treated as absent, not cause panics.

#### Key References

- [Newtype Pattern - Rust Design Patterns](https://rust-unofficial.github.io/patterns/patterns/behavioural/newtype.html) -- Official pattern documentation
- [Embrace the newtype pattern - Effective Rust](https://www.lurklurk.org/effective-rust/newtype.html) -- Comprehensive guide with From/Into examples
- [Serde Struct Flattening](https://serde.rs/attr-flatten.html) -- How `#[serde(flatten)]` works with maps
- [The Newtype Pattern in Rust](https://www.worthe-it.co.za/blog/2020-10-31-newtype-pattern-in-rust.html) -- Practical walkthrough with tradeoffs
- [serde(flatten) on BTreeMap - serde Issue #2176](https://github.com/serde-rs/serde/issues/2176) -- Known limitations and workarounds

#### Initial Recommendations

**Use Pattern A (Newtype Wrapper with Accessor Methods)**. This is the best fit because:

1. The `Item` is the source of truth stored in JSONL -- we should not create a parallel struct that could diverge.
2. The newtype `PgItem(Item)` preserves all unknown extensions and native fields during round-trips.
3. Accessor methods provide typed access to `x-pg-*` fields with validation.
4. The inner `Item` is directly passable to `Store::save_active()` without conversion.
5. Define extension key constants: `const EXT_PG_STATUS: &str = "x-pg-status";` etc.

For `StructuredDescription`, define it as a separate serde struct and use `serde_json::from_value` / `to_value` for the `x-pg-description` extension field.

---

### 3. Async/Sync Bridging in Tokio

#### Landscape Overview

task-golem's `Store::with_lock()` is a synchronous, blocking function that acquires a file lock with exponential backoff (up to 5 seconds of blocking via `std::thread::sleep`). Phase-golem runs on Tokio's async runtime. Calling `with_lock()` directly from an async context would block a Tokio worker thread, potentially starving other tasks. Tokio provides `spawn_blocking` specifically for this scenario -- it moves blocking work to a dedicated thread pool.

#### Patterns Found

**Pattern A: Direct spawn_blocking per Operation**

How it works: Each coordinator operation that touches the store wraps its `with_lock()` call in `tokio::task::spawn_blocking`.

```rust
async fn update_item(&self, id: &str, update: impl FnOnce(&mut Item)) -> Result<(), PgError> {
    let store = self.store.clone(); // Store must be Clone + Send + 'static
    let id = id.to_string();
    tokio::task::spawn_blocking(move || {
        store.with_lock(|store| {
            let mut items = store.load_active()?;
            if let Some(item) = items.iter_mut().find(|i| i.id == id) {
                update(item);
            }
            store.save_active(&items)?;
            Ok(())
        })
    })
    .await
    .map_err(|e| PgError::JoinError(e))? // JoinError from panic
    .map_err(PgError::from) // TgError from store operation
}
```

When to use: The standard approach for wrapping any synchronous blocking operation in async code.

Tradeoffs:
- (+) Straightforward, well-documented, idiomatic Tokio
- (+) Each operation is independent -- no shared mutable state between blocking calls
- (+) Tokio manages the thread pool automatically
- (-) Requires `Store` to be `Clone + Send + 'static` (it is -- it only holds a `PathBuf`)
- (-) The closure must own all its data (`move` semantics, no references to the async context)
- (-) Two levels of error handling: `JoinError` from `spawn_blocking` and `TgError` from the store

**Pattern B: Dedicated Blocking Actor**

How it works: Spawn a single long-lived blocking thread that receives commands via a channel. The coordinator sends commands and awaits responses.

```rust
// Blocking thread loop
fn store_actor(rx: mpsc::Receiver<StoreCommand>, store: Store) {
    while let Some(cmd) = rx.blocking_recv() {
        match cmd {
            StoreCommand::LoadActive(reply) => {
                let result = store.with_lock(|s| s.load_active());
                let _ = reply.send(result);
            }
            // ...
        }
    }
}
```

When to use: When you want to serialize all store access through a single thread and avoid repeated `spawn_blocking` overhead.

Tradeoffs:
- (+) Single point of serialization -- no concurrent file lock contention from phase-golem's side
- (-) Significantly more complex architecture (command enum, reply channels, actor lifecycle)
- (-) Redundant with the coordinator's existing mpsc actor pattern -- would be an actor within an actor
- (-) Harder to pass arbitrary closures (need to enumerate all operations as commands)

**Pattern C: async-fd-lock Crate**

How it works: Use `async-fd-lock` which provides async file locking by internally using `spawn_blocking`. This would require task-golem to switch its locking implementation.

When to use: When you want fully async file locking without manual `spawn_blocking` management.

Tradeoffs:
- (+) The async/sync bridging is handled by the crate
- (-) Requires changing task-golem's internal implementation
- (-) task-golem should remain independently usable as a synchronous CLI tool
- (-) Adds a dependency and changes the locking API

**Pattern D: block_in_place**

How it works: `tokio::task::block_in_place()` runs blocking code on the current thread, temporarily expanding the thread pool if needed. Only works with the multi-threaded runtime.

When to use: When you want to avoid the overhead of spawning a new blocking task but can tolerate blocking a worker thread temporarily.

Tradeoffs:
- (+) Slightly lower overhead than `spawn_blocking` (no cross-thread handoff)
- (+) Can use references from the current scope (no `'static` requirement)
- (-) Only works with multi-threaded runtime (not `current_thread`)
- (-) Still blocks a worker thread -- just adds a replacement thread
- (-) Less explicit about the blocking nature of the operation

#### Error Handling: JoinError and Panic Propagation

`spawn_blocking` returns `JoinHandle<R>` which `.await`s to `Result<R, JoinError>`. If the closure panics, the `JoinError` captures the panic payload.

```rust
match tokio::task::spawn_blocking(move || store_operation()).await {
    Ok(Ok(result)) => Ok(result),       // Store operation succeeded
    Ok(Err(tg_err)) => Err(tg_err.into()), // TgError from store
    Err(join_err) if join_err.is_panic() => {
        // Panic inside spawn_blocking -- treat as fatal
        // The PRD specifies: "treat as fatal coordinator error"
        Err(PgError::InternalPanic(format!("{:?}", join_err)))
    }
    Err(join_err) => {
        // Task was cancelled (should not happen -- spawn_blocking cannot be aborted)
        Err(PgError::InternalPanic(format!("Unexpected cancellation: {:?}", join_err)))
    }
}
```

Key facts about `spawn_blocking`:
- Tasks **cannot be aborted** -- `abort()` has no effect
- Runtime shutdown waits indefinitely for started blocking tasks
- The closure must be `FnOnce() -> R + Send + 'static`
- The return type `R` must be `Send + 'static`

#### Technologies & Tools

- **`tokio::task::spawn_blocking`**: The standard tool. Built into Tokio, well-documented, production-proven. [spawn_blocking docs](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html)
- **`tokio::task::block_in_place`**: Alternative for multi-threaded runtimes. Avoids `'static` requirement but blocks a worker thread.
- **`async-fd-lock`**: Async file locking crate that wraps `fd-lock` with `spawn_blocking` internally. [async-fd-lock on crates.io](https://crates.io/crates/async-fd-lock)

#### Common Pitfalls

- **Calling blocking code without spawn_blocking**: Directly calling `with_lock()` from an async function blocks the Tokio worker thread. With a single-threaded runtime this deadlocks. With multi-threaded runtime it degrades throughput.
- **Holding references across spawn_blocking boundary**: The closure must own all data. Cannot borrow from the async function's scope. Must `clone()` or `move` everything the closure needs.
- **Nested runtime creation**: Never call `Runtime::block_on()` inside an existing Tokio runtime -- it panics. If you need to call async from sync-from-async, use `spawn_blocking` for the sync part, not `block_on`.
- **Thread pool exhaustion**: Tokio's blocking thread pool has a large but finite limit (512 by default). If many `spawn_blocking` calls are in flight simultaneously, new ones queue. This is not a concern for phase-golem (serialized through coordinator actor).
- **Panic propagation**: A panic inside `spawn_blocking` does not crash the process -- it is captured in `JoinError`. The coordinator must handle this explicitly (the PRD specifies treating it as fatal).

#### Standards/Best Practices

- Use `spawn_blocking` for any operation that may block for more than a few microseconds.
- Handle `JoinError` explicitly -- do not `.unwrap()` the result of `.await` on a `JoinHandle`.
- Treat panics from `spawn_blocking` as bugs (fatal errors), not transient conditions.
- Since the coordinator already serializes operations through an mpsc channel, there is no risk of concurrent `spawn_blocking` calls contending on the file lock from phase-golem's side.
- Use the `move` keyword on the closure and clone/own all data before the closure.

#### Key References

- [spawn_blocking - Tokio docs](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) -- Authoritative API reference
- [JoinError - Tokio docs](https://docs.rs/tokio/latest/tokio/task/struct.JoinError.html) -- Error type documentation
- [Bridging with sync code - Tokio tutorial](https://tokio.rs/tokio/topics/bridging) -- Official guide on async/sync bridging
- [Bridge Async and Sync Code in Rust - GreptimeDB blog](https://greptime.com/blogs/2023-03-09-bridging-async-and-sync-rust) -- Practical walkthrough with pitfalls and solutions
- [Spawning - Tokio tutorial](https://tokio.rs/tokio/tutorial/spawning) -- Context on task spawning and Send bounds

#### Initial Recommendations

**Use Pattern A (Direct spawn_blocking per operation)**. This is the simplest correct approach:

1. The coordinator actor already serializes write operations through its mpsc channel -- this eliminates concurrent file lock contention from phase-golem's side.
2. `Store` is `Clone + Send + 'static` (it only holds a `PathBuf`) so it can be moved into the closure.
3. Two-level error handling (`JoinError` then `TgError`) maps cleanly to the PRD's error categories (panic = fatal, `LockTimeout` = retryable, etc.).
4. No need for the complexity of Pattern B (dedicated blocking actor) since the coordinator already provides serialization.

Pattern D (`block_in_place`) is worth considering as an optimization later since it avoids the `'static` requirement, but `spawn_blocking` is the safer default.

---

### 4. Cross-Repo Rust Path Dependencies

#### Landscape Overview

When two Rust projects live in separate repositories and one depends on the other, Cargo offers three mechanisms: path dependencies (local filesystem), git dependencies (remote repository), and registry dependencies (crates.io or private registry). For development between co-located repositories, path dependencies are the simplest. The choice between path and git dependencies involves tradeoffs around portability, reproducibility, and development workflow.

#### Patterns Found

**Pattern A: Simple Path Dependency**

How it works: Specify a relative or absolute path to the dependency's `Cargo.toml` directory.

```toml
[dependencies]
task_golem = { path = "../task-golem" }
```

When to use: During active development when both repositories are checked out locally at known relative positions.

Tradeoffs:
- (+) Changes to task-golem are immediately visible in phase-golem (no publish/fetch cycle)
- (+) Single `cargo build` compiles both crates together
- (+) Works with `cargo check`, `cargo test`, etc.
- (-) Assumes a specific directory layout on every developer's machine
- (-) Cannot be published to crates.io with only a path dependency
- (-) Not portable -- other developers must clone both repos in the same relative positions

**Pattern B: Git Dependency**

How it works: Specify a git URL and optional branch/tag/rev.

```toml
[dependencies]
task_golem = { git = "https://github.com/sirhamy/task-golem.git", branch = "main" }
```

When to use: When you want reproducible builds pinned to specific commits, or when the dependency is shared with other developers who may not have both repos locally.

Tradeoffs:
- (+) Reproducible -- locked to a specific commit in `Cargo.lock`
- (+) Works without local checkout of the dependency
- (+) Cargo traverses the git repo to find `Cargo.toml` anywhere in the tree
- (-) Changes require commit + push to the dependency repo before they are visible
- (-) Slower development iteration cycle
- (-) Cannot be published to crates.io with only a git dependency

**Pattern C: Path + Version (Dual Source)**

How it works: Specify both a path and a version. Cargo uses the path during local development and the registry version when publishing.

```toml
[dependencies]
task_golem = { path = "../task-golem", version = "0.1.0" }
```

When to use: When you plan to eventually publish both crates and want a smooth transition.

Tradeoffs:
- (+) Best of both worlds for development and publishing
- (+) `cargo publish` uses the registry version, local development uses the path
- (-) Must keep the version in sync
- (-) Not relevant for this use case (neither crate is published to a registry)

**Pattern D: .cargo/config.toml Path Override**

How it works: Use Cargo's path override mechanism in `.cargo/config.toml` to redirect a dependency to a local path without modifying `Cargo.toml`. This is developer-specific and not committed to the repository.

```toml
# .cargo/config.toml
paths = ["../task-golem"]
```

When to use: When you want to temporarily test local changes without modifying the committed `Cargo.toml`.

Tradeoffs:
- (+) Does not modify committed files
- (+) Per-developer customization
- (-) Easy to forget to remove, causing confusion
- (-) Limited to same-name crate replacement
- (-) The PRD explicitly chose path dependency for simplicity; this adds unnecessary indirection

**RFC 3529: Path Bases (Unstable)**

A new Cargo feature (not yet stable) that allows defining named path bases in `config.toml` and referencing them in dependencies. This would solve the "relative path portability" problem by letting each developer configure their own base path. Not yet stable enough to rely on for this project.

#### Technologies & Tools

- **Cargo path dependencies**: Built-in, stable, zero configuration. [Specifying Dependencies - The Cargo Book](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html)
- **Cargo git dependencies**: Built-in, stable, supports branch/tag/rev pinning.
- **Cargo `[patch]` section**: Override any dependency (registry or git) with a local path for development. [Overriding Dependencies - The Cargo Book](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html)
- **RFC 3529 Path Bases**: Unstable feature for named path bases. [RFC 3529](https://rust-lang.github.io/rfcs/3529-cargo-path-bases.html)

#### Common Pitfalls

- **Relative path fragility**: `path = "../task-golem"` assumes both repos are siblings. If someone clones them differently, builds break. Document the expected layout.
- **Cargo.lock divergence**: With path dependencies, `Cargo.lock` does not record a version or commit for the dependency -- it just uses whatever is on disk. This means two developers can build different code without knowing it.
- **Edition mismatches**: task-golem uses Rust 2024 edition, phase-golem uses 2021. This is fine -- each crate compiles with its own edition. But the minimum rustc version must be compatible with both (1.85+ per the PRD).
- **Cannot combine path and git**: `{ git = "...", path = "..." }` is invalid Cargo.toml. You must choose one or the other (or use `[patch]` for development overrides).

#### Standards/Best Practices

- Use path dependencies during active co-development when both repos are on the same machine.
- Document the expected repository layout (sibling directories) in a README or contributing guide.
- Consider switching to git dependencies once task-golem's library API stabilizes, for better reproducibility.
- Do not commit `.cargo/config.toml` path overrides to the repository.

#### Key References

- [Specifying Dependencies - The Cargo Book](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html) -- Authoritative reference for path, git, and registry dependencies
- [Overriding Dependencies - The Cargo Book](https://doc.rust-lang.org/cargo/reference/overriding-dependencies.html) -- `[patch]` and path overrides
- [RFC 3529: Cargo Path Bases](https://rust-lang.github.io/rfcs/3529-cargo-path-bases.html) -- Future solution for portable path dependencies
- [Cargo Path Bases Tracking Issue](https://github.com/rust-lang/cargo/issues/14355) -- Implementation status

#### Initial Recommendations

**Use Pattern A (Simple Path Dependency)** as specified in the PRD. This is the right choice for single-developer co-development of sibling repositories:

```toml
[dependencies]
task_golem = { path = "../task-golem" }
```

Rationale:
1. Both repos are on the same machine in a known layout.
2. Tight iteration cycle is valuable during the migration -- changes to task-golem's library API are immediately testable in phase-golem.
3. Neither crate is published to a registry, so the publishing restrictions are irrelevant.
4. Can transition to a git dependency later once the library API stabilizes.

---

### 5. Error Type Mapping Across Crate Boundaries

#### Landscape Overview

When crate A depends on crate B, crate A needs to handle crate B's error types. The standard Rust approach is to define an error enum in crate A that wraps crate B's errors using `From` implementations (often generated by `thiserror`). The challenge is mapping between error semantics: crate B's errors have their own categorization (user errors vs. system errors) and crate A may need different categories (retryable vs. fatal vs. skip).

#### Patterns Found

**Pattern A: Direct #[from] Wrapping with thiserror**

How it works: Add a variant to the downstream error enum that wraps the upstream error using `#[from]`.

```rust
#[derive(Debug, thiserror::Error)]
pub enum PgError {
    #[error("Storage error: {0}")]
    Storage(#[from] task_golem::errors::TgError),

    #[error("Internal panic: {0}")]
    InternalPanic(String),
}
```

When to use: When you want the simplest possible integration and do not need to differentiate between upstream error variants.

Tradeoffs:
- (+) Minimal boilerplate -- `?` operator works automatically
- (+) Full error chain preserved (`.source()` returns the inner `TgError`)
- (-) All `TgError` variants collapse into a single `PgError::Storage` -- callers cannot distinguish `LockTimeout` from `StorageCorruption` without downcasting
- (-) Does not support the PRD's requirement for explicit error categorization (retryable vs. fatal)

**Pattern B: Explicit Variant Mapping**

How it works: Map each upstream error variant (or group of variants) to specific downstream variants with different semantics. Implement `From` manually rather than using `#[from]`.

```rust
#[derive(Debug, thiserror::Error)]
pub enum PgError {
    // Retryable errors
    #[error("Lock timeout: {0}")]
    LockTimeout(String),

    // Fatal errors
    #[error("Storage corruption: {0}")]
    StorageCorruption(String),

    // Skip errors (log and continue)
    #[error("Item not found: {0}")]
    ItemNotFound(String),

    // Internal errors
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Internal panic in storage thread: {0}")]
    InternalPanic(String),

    #[error("Unexpected storage error: {0}")]
    Unexpected(String),
}

impl From<TgError> for PgError {
    fn from(err: TgError) -> Self {
        match err {
            TgError::LockTimeout(d) => PgError::LockTimeout(format!("after {:?}", d)),
            TgError::StorageCorruption(msg) => PgError::StorageCorruption(msg),
            TgError::ItemNotFound(id) => PgError::ItemNotFound(id),
            TgError::IoError(e) => PgError::Io(e),
            other => PgError::Unexpected(other.to_string()),
        }
    }
}
```

When to use: When different upstream error variants require different handling strategies in the downstream code (retry, halt, skip).

Tradeoffs:
- (+) Explicit mapping makes error handling intent clear
- (+) Enables pattern matching on error categories in the coordinator
- (+) Can add context during mapping (e.g., annotating with retry counts)
- (-) More boilerplate -- must enumerate all variants
- (-) Must be updated when upstream adds new error variants (but this is a feature, not a bug -- it forces you to decide how to handle new errors)
- (-) Loses the original `TgError` type (only retains its string message for some variants)

**Pattern C: Category-Based Error Enum with is_retryable()**

How it works: Define the error enum with category methods rather than category-based variants.

```rust
#[derive(Debug, thiserror::Error)]
pub enum PgError {
    #[error(transparent)]
    Tg(#[from] TgError),

    #[error("Internal panic: {0}")]
    InternalPanic(String),
}

impl PgError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, PgError::Tg(TgError::LockTimeout(_)))
    }

    pub fn is_fatal(&self) -> bool {
        matches!(self, PgError::Tg(TgError::StorageCorruption(_)) | PgError::InternalPanic(_))
    }

    pub fn is_skip(&self) -> bool {
        matches!(self, PgError::Tg(TgError::ItemNotFound(_)))
    }
}
```

When to use: When you want to keep the original error type accessible for inspection while adding semantic categorization.

Tradeoffs:
- (+) Preserves the full original error type
- (+) Clean `is_retryable()` / `is_fatal()` API for callers
- (+) Less boilerplate than Pattern B
- (-) Error categorization is separate from the type system -- easy to forget to check
- (-) `#[error(transparent)]` hides the `PgError::Tg` wrapper in display output

**Pattern D: Hybrid with thiserror + anyhow**

How it works: Use structured `thiserror` enums for errors that need programmatic handling, and wrap unexpected errors in `anyhow::Error` for diagnostic-only reporting.

```rust
#[derive(Debug, thiserror::Error)]
pub enum PgError {
    #[error("Lock timeout: retry in {retry_after_seconds}s")]
    LockTimeout { retry_after_seconds: u64 },

    #[error("Storage corruption: {0}")]
    StorageCorruption(String),

    #[error(transparent)]
    Unexpected(#[from] anyhow::Error),
}
```

When to use: Large projects where many error sources exist and you want to avoid exhaustive variant enumeration for rare/unexpected errors.

Tradeoffs:
- (+) Focused enum variants for actionable errors, catch-all for everything else
- (+) `anyhow::Error` provides rich context chains via `.context()`
- (-) Phase-golem does not currently use `anyhow` -- would add a dependency
- (-) The catch-all `Unexpected` variant can become a dumping ground

#### Technologies & Tools

- **`thiserror` (v2)**: Derive macro for error enums. Generates `Display`, `Error`, and optional `From` implementations. Used by task-golem already. [thiserror on crates.io](https://crates.io/crates/thiserror)
- **`anyhow` (v1)**: Opaque error type with context chaining. Best for application-level error handling. [anyhow on crates.io](https://crates.io/crates/anyhow)
- **`snafu`**: Alternative to thiserror with built-in context and location tracking. Used by GreptimeDB. [snafu on crates.io](https://crates.io/crates/snafu)
- **`backoff` crate**: Retry logic with transient/permanent error classification. [backoff on GitHub](https://github.com/ihrwein/backoff)

#### Common Pitfalls

- **Losing error source chains**: When converting with `.to_string()`, the `.source()` chain is lost. Preserve the original error type when possible.
- **Blanket `#[from]` for conflicting types**: If two dependencies both use `std::io::Error`, you cannot have two `#[from] std::io::Error` variants. Use manual `From` implementations or different wrapper variants.
- **Over-categorization**: Creating too many error variants makes matching exhausting. Group by handling strategy (retryable, fatal, skip) rather than by cause.
- **Forgetting new upstream variants**: When the upstream crate adds new error variants, a manual `From` implementation will fail to compile (if using `match`) -- this is desirable as it forces explicit handling decisions.

#### Standards/Best Practices

- Use `thiserror` for error enums in library-like code (coordinator, adapter).
- Map upstream errors to downstream categories based on **handling strategy**, not origin.
- Preserve the error source chain for debugging (`#[source]` or `#[from]` attributes).
- Define a method like `is_retryable()` for callers that need to decide on retry behavior.
- Log errors at the handling site, not at the creation site.

#### Key References

- [Error Handling in Rust - A Deep Dive (Luca Palmieri)](https://lpalmieri.com/posts/error-handling-rust/) -- Comprehensive guide to error handling patterns, From implementations, and thiserror vs anyhow
- [thiserror docs](https://docs.rs/thiserror) -- API reference and examples
- [Wrapping errors - Rust By Example](https://doc.rust-lang.org/rust-by-example/error/multiple_error_types/wrap_error.html) -- Basic error wrapping pattern
- [Error Handling for Large Rust Projects - GreptimeDB](https://greptime.com/blogs/2024-05-07-error-rust) -- Real-world error handling at scale with snafu
- [Prefer idiomatic Error types - Effective Rust](https://www.lurklurk.org/effective-rust/errors.html) -- Guidelines for designing error types
- [Modular Errors with thiserror - GitHub Gist](https://gist.github.com/quad/a8a7cc87d1401004c6a8973947f20365) -- Practical example of modular error enums

#### Initial Recommendations

**Use Pattern B (Explicit Variant Mapping) combined with Pattern C's `is_retryable()` method**. This gives the coordinator explicit control over error handling while keeping the categorization API clean:

1. Define `PgError` variants grouped by handling strategy: `LockTimeout` (retryable), `StorageCorruption` (fatal/halt), `ItemNotFound` (skip), `InternalPanic` (fatal), etc.
2. Implement `From<TgError>` manually to map each `TgError` variant to the appropriate `PgError` variant.
3. Add `is_retryable()` and `is_fatal()` methods for the coordinator's retry logic.
4. The `JoinError` from `spawn_blocking` maps to `PgError::InternalPanic`.

This matches the PRD's explicit requirement: "`LockTimeout` -> retryable, `StorageCorruption` -> halt, `ItemNotFound` -> log and skip."

---

### 6. JSONL Storage Patterns

#### Landscape Overview

JSONL (JSON Lines, also called newline-delimited JSON) is a file format where each line is a valid JSON object. It is well-suited for append-only storage, streaming, and line-by-line processing. task-golem already implements a mature JSONL storage layer with schema versioning, atomic writes via temp-file-rename, file locking for concurrent access, and separate active/archive files. Phase-golem will consume this layer as-is through the library API.

#### Patterns Found

**Pattern A: Full Rewrite via Atomic Rename (task-golem's current pattern)**

How it works: For the active store, the entire file is rewritten on every save. A temporary file is written in the same directory, fsynced, and atomically renamed to the target path.

```rust
// task-golem's write_atomic() implementation
let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
writeln!(tmp, "{}", schema_header)?;
for item in sorted_items {
    writeln!(tmp, "{}", serde_json::to_string(item)?)?;
}
tmp.as_file().sync_all()?;  // fsync for durability
tmp.persist(path)?;          // atomic rename
```

When to use: When the dataset fits in memory and you need atomic, consistent snapshots.

Tradeoffs:
- (+) Readers always see a complete, consistent file (rename is atomic on POSIX)
- (+) No partial writes or corruption on crash -- the old file remains until rename completes
- (+) Deterministic output (items sorted by ID)
- (-) O(n) write cost for every save, even for single-item changes
- (-) Not suitable for very large datasets (but fine for phase-golem's tens of items)

**Pattern B: Append-Only with Compaction**

How it works: New items are appended to the end of the file. Periodically, the file is compacted (rewritten to remove deleted/superseded entries).

When to use: Write-heavy workloads where append is much more common than full reads.

Tradeoffs:
- (+) O(1) write for appends
- (+) Natural event log / audit trail
- (-) Reads must scan the entire file and deduplicate
- (-) File grows unboundedly without compaction
- (-) Not used by task-golem for active items (only for archive appends)

**Pattern C: File Locking for Concurrent Access (task-golem's current pattern)**

How it works: A separate lock file is used with `fd-lock` (OS-level advisory file lock). The `with_lock()` function acquires an exclusive write lock with exponential backoff (10ms to 500ms, total timeout 5 seconds).

```rust
// task-golem's locking implementation
let mut lock = RwLock::new(file);  // fd-lock RwLock on lock file
loop {
    match lock.try_write() {
        Ok(_guard) => return callback(),
        Err(_) => {
            if elapsed >= 5s { return Err(LockTimeout) }
            sleep(backoff_with_jitter);
        }
    }
}
```

When to use: When multiple processes may access the same file concurrently (phase-golem + human running `tg` CLI).

Tradeoffs:
- (+) OS-level advisory lock -- works across processes
- (+) Exponential backoff with jitter prevents thundering herd
- (+) RAII guard ensures lock release on drop (including panics)
- (-) Advisory locks are not enforced -- a buggy reader could ignore the lock
- (-) The 5-second timeout can cause `LockTimeout` under heavy contention
- (-) Lock acquisition uses `std::thread::sleep` -- blocking, must be wrapped in `spawn_blocking` from async code

**Pattern D: Schema Versioning Header**

How it works: The first line of each JSONL file is a schema header (`{"schema_version": 1}`) that identifies the format version. Readers validate the version before parsing items.

When to use: When the data format may evolve over time and old/new readers may encounter files from different versions.

Tradeoffs:
- (+) Forward-compatible -- new versions can be detected and handled
- (+) Fail-fast on version mismatch rather than silent data corruption
- (-) One extra line to parse on every read

#### Technologies & Tools

- **`tempfile` crate**: Creates temporary files in the same directory for atomic rename. [tempfile on crates.io](https://crates.io/crates/tempfile)
- **`fd-lock` crate**: Cross-platform advisory file locking using file descriptors. [fd-lock on crates.io](https://crates.io/crates/fd-lock)
- **`serde_json`**: JSON serialization/deserialization. Line-by-line parsing with `BufReader`.
- **`atomic-write-file` crate**: Higher-level atomic write abstraction. [atomic-write-file on crates.io](https://crates.io/crates/atomic-write-file) -- Not needed here since task-golem already implements atomic writes.

#### Common Pitfalls

- **Missing fsync before rename**: Without `sync_all()`, the renamed file could contain only partial data if the system crashes between write and rename. task-golem correctly fsyncs.
- **Cross-filesystem rename**: `rename()` fails if the temp file and target are on different filesystems. task-golem avoids this by creating the temp file in the same directory (`NamedTempFile::new_in(dir)`).
- **Archive truncation recovery**: If a crash occurs during an archive append, the last line may be truncated. task-golem's archive reader handles this by skipping malformed lines. The active store reader fails fast on malformed lines (correct, since active store uses atomic rename and should never be partially written).
- **Lock file cleanup**: Lock files are never deleted in task-golem (they are empty sentinel files). This is correct -- deleting them could cause races.

#### Standards/Best Practices

- JSONL format specification: each line is a self-contained JSON value, separated by newlines. [jsonlines.org](https://jsonlines.org/)
- Always fsync before atomic rename for durability.
- Create temp files in the same directory as the target to ensure same-filesystem rename.
- Use advisory file locks for multi-process access; accept that they are advisory (not enforced).
- Schema version in the first line enables forward-compatible evolution.

#### Key References

- [JSON Lines Specification](https://jsonlines.org/) -- Format specification
- [tempfile crate](https://crates.io/crates/tempfile) -- Temporary file creation for atomic writes
- [fd-lock crate](https://crates.io/crates/fd-lock) -- Cross-platform file locking
- [Atomic File Writes in Rust - rust-atomicwrites](https://github.com/untitaker/rust-atomicwrites) -- Reference implementation of atomic writes
- [A way to do atomic writes - LWN.net](https://lwn.net/Articles/789600/) -- Kernel-level discussion of atomic write guarantees

#### Initial Recommendations

**Phase-golem should use task-golem's existing JSONL storage layer as-is.** The implementation is already mature:

1. Atomic writes via temp-file-rename with fsync (Pattern A) -- correct and sufficient for the scale.
2. File locking via fd-lock with exponential backoff (Pattern C) -- handles the concurrent access case.
3. Schema versioning (Pattern D) -- enables future format evolution.
4. Archive append with truncation recovery (Pattern B for archive only) -- handles crash safety.

Phase-golem does not need to implement any of these patterns itself. It consumes them through `Store::with_lock()`, `Store::load_active()`, `Store::save_active()`, and `Store::append_to_archive()`.

The only phase-golem-side concern is wrapping these synchronous calls in `spawn_blocking` (covered in Section 3) and handling errors appropriately (covered in Section 5).

---

## Internal Research

### Phase-Golem Architecture

Phase-golem is a CLI orchestrator managing a multi-phase pipeline workflow. Its storage layer uses YAML-based persistence with schema migrations. The architecture follows an async actor pattern for coordination.

**Relevant files/modules:**

- `src/types.rs` — Core type definitions: `BacklogItem` (24 fields), `BacklogFile`, `ItemStatus` (6 variants), `StructuredDescription`, `FollowUp`, `InboxItem`, `ItemUpdate` enum, `SizeLevel`, `DimensionLevel`, `BlockType`, `PhasePool`
- `src/backlog.rs` — Storage layer (~550 lines): load/save YAML, auto-migrate schemas, sequential ID generation, item CRUD, inbox mechanism, stale dependency pruning, merge, archive
- `src/migration.rs` — Schema migration (~620 lines): v1→v2→v3 migration chain with legacy struct definitions
- `src/coordinator.rs` — Async actor (~860 lines): mpsc channel with 12 command variants, in-memory `BacklogFile`, batch commit logic, git operations
- `src/scheduler.rs` — Scheduling logic (~1000+ lines): pure `select_actions()` function, dependency checking, phase ordering
- `src/config.rs` — `ProjectConfig` with `prefix` and `backlog_path` fields
- `src/git.rs` — Git operations: `stage_paths`, `commit`, `get_head_sha`, `is_ancestor`
- `src/lib.rs` — Module declarations (currently exposes: `agent`, `backlog`, `config`, `coordinator`, `executor`, `filter`, `git`, `lock`, `log`, `migration`, `preflight`, `prompt`, `scheduler`, `types`, `worklog`)
- `Cargo.toml` — Binary-only crate, edition 2021. Key deps: `chrono`, `serde`, `serde_json`, `serde_yaml_ng = "0.10"`, `tokio`, `tempfile`, `fslock`
- `tests/common/mod.rs` — Test helpers: `make_item()`, `make_in_progress_item()`, `make_backlog()`, `setup_test_env()`
- 16 test files in `tests/` — `scheduler_test.rs`, `coordinator_test.rs`, `backlog_test.rs`, `migration_test.rs`, etc.

### Phase-Golem Storage Layer Details

**`BacklogItem` (24 fields):** `id`, `title`, `status: ItemStatus`, `phase: Option<String>`, `size: Option<SizeLevel>`, `complexity: Option<DimensionLevel>`, `risk: Option<DimensionLevel>`, `impact: Option<DimensionLevel>`, `requires_human_review: bool`, `origin: Option<String>`, `blocked_from_status: Option<ItemStatus>`, `blocked_reason: Option<String>`, `blocked_type: Option<BlockType>`, `unblock_context: Option<String>`, `tags: Vec<String>`, `dependencies: Vec<String>`, `created: String`, `updated: String`, `pipeline_type: Option<String>`, `description: Option<StructuredDescription>`, `phase_pool: Option<PhasePool>`, `last_phase_commit: Option<String>`

**`BacklogFile`:** `schema_version: u32`, `items: Vec<BacklogItem>`, `next_item_id: u32`

**`ItemStatus` (6 variants):** `New`, `Scoping`, `Ready`, `InProgress`, `Done`, `Blocked` — with `is_valid_transition()` method

**`StructuredDescription`:** 5 fields — `context`, `problem`, `solution`, `impact`, `sizing_rationale` (all `String`)

**`ItemUpdate` enum (10 variants):** `TransitionStatus`, `SetPhase`, `SetPhasePool`, `ClearPhase`, `SetBlocked`, `Unblock`, `UpdateAssessments`, `SetPipelineType`, `SetLastPhaseCommit`, `SetDescription`

**Key `backlog.rs` operations (all to be deleted):**

- `load(path, project_root)` — Loads YAML, auto-migrates v1→v2→v3
- `save(path, backlog)` — Atomic write-temp-rename
- `generate_next_id(backlog, prefix)` — Sequential: `max(items_max_suffix, next_item_id) + 1`
- `add_item(backlog, title, size, risk, prefix)` — Creates new item with status `New`
- `transition_status(item, new_status)` — Validates transition, sets `blocked_from_status` on block, clears blocked fields on unblock
- `update_assessments(item, assessments)` — Merges non-None assessment fields
- `archive_item(backlog, item_id, backlog_path, worklog_path)` — Removes item, strips dep refs, saves, writes worklog
- `ingest_follow_ups(backlog, follow_ups, origin, prefix)` — Creates new items from phase results
- `load_inbox/ingest_inbox_items/clear_inbox` — YAML drop-file mechanism
- `prune_stale_dependencies(backlog)` — Removes refs to non-existent IDs
- `merge_item(backlog, source_id, target_id)` — Appends source description to target, union-merges deps, strips source refs

### Phase-Golem Coordinator Details

**`CoordinatorCommand` enum (12 variants):** `GetSnapshot`, `UpdateItem`, `CompletePhase`, `BatchCommit`, `GetHeadSha`, `IsAncestor`, `RecordPhaseStart`, `WriteWorklog`, `ArchiveItem`, `IngestFollowUps`, `UnblockItem`, `IngestInbox`, `MergeItem`

**`CoordinatorState` holds:** `backlog: BacklogFile`, `backlog_path`, `inbox_path`, `project_root`, `prefix`, `pending_batch_phases: Vec<(String, String, Option<String>)>`

**Key patterns:**

- `spawn_coordinator()` creates `(mpsc::Sender, mpsc::Receiver)` channel, spawns actor loop
- `CoordinatorHandle` is clone-able, provides async methods for each command
- Handler functions mutate in-memory `BacklogFile`, then call `backlog::save()` to persist
- `CompletePhase` handler: uses `spawn_blocking` for git operations, stages dirty files, commits if destructive. Accumulates `pending_batch_phases` for non-destructive.
- `BatchCommit` handler: commits all staged changes with batch message, clears `pending_batch_phases`
- `IngestInbox` handler: loads inbox, ingests items, saves backlog, clears inbox (with rollback on save failure)
- Git operations (`GetHeadSha`, `IsAncestor`) already wrapped in `spawn_blocking`

### Phase-Golem Scheduler Details

**`select_actions(snapshot: &BacklogFile, running: &RunningTasks, config: &ExecutionConfig, pipelines: &HashMap<String, PipelineConfig>) -> Vec<SchedulerAction>`**: Pure function. Accesses `snapshot.items` slice. Reads `status`, `phase`, `phase_pool`, `pipeline_type`, `impact`, `created`, `dependencies`, `id` from `BacklogItem`.

**Key scheduler functions:**

- `sorted_ready_items` — by impact desc, created asc
- `sorted_in_progress_items` / `sorted_scoping_items` — by phase index desc, created asc
- `sorted_new_items` — by created asc
- `skip_for_unmet_deps(item, all_items)` — absent dep IDs = met (archived/missing)
- `build_run_phase_action(item, pipelines)` — reads `pipeline_type`, `phase`, `phase_pool`, looks up `PhaseConfig.is_destructive`
- `run_scheduler()` — main async loop with snapshot, promotions, phase spawning, follow-up ingestion, merge/archive

**WRK-035 uncommitted change:** `unmet_dep_summary` signature changed from `&[BacklogItem]` to `&HashMap<&str, &BacklogItem>`, new `build_item_lookup` helper added. This work needs to be resolved before or as part of WRK-076.

### Task-Golem Architecture

**`Item` struct:** `id: String`, `title: String`, `status: Status`, `priority: i64`, `description: Option<String>`, `tags: Vec<String>`, `dependencies: Vec<String>`, `created_at: DateTime<Utc>`, `updated_at: DateTime<Utc>`, `blocked_reason: Option<String>`, `blocked_from_status: Option<Status>`, `claimed_by: Option<String>`, `claimed_at: Option<DateTime<Utc>>`, `extensions: BTreeMap<String, serde_json::Value>` (via `#[serde(flatten)]`)

Key methods: `validate_title()`, `validate_extensions()` (enforces `x-` prefix, rejects known field name collisions), `apply_do()`, `apply_done()`, `apply_block()`, `apply_unblock()`, `apply_todo()`. None fields serialize as JSON `null` (not omitted).

**`Status` (4 variants):** `Todo`, `Doing`, `Done`, `Blocked`. Serializes as lowercase. Transition rules: Todo→Doing/Done/Blocked, Doing→Done/Blocked/Todo, Blocked→nothing (use `apply_unblock`), Done→nothing (terminal).

**`generate_id_with_prefix(existing_ids: &HashSet<String>, prefix: &str) -> Result<String, TgError>`**: Generates `{prefix}-{5-hex-chars}`. Retries up to 10 times on collision.

**`Store` struct:** Holds only `project_dir: PathBuf`. Methods: `new()`, `tasks_path()`, `archive_path()`, `lock_path()`, `with_lock()`, `load_active()`, `save_active()`, `load_archive_ids()`, `load_archive_item()`, `load_all_archive()`, `all_known_ids()`, `append_to_archive()`.

**`with_lock()` semantics:** Exclusive `fd-lock::RwLock` with exponential backoff (10ms→500ms cap), total timeout 5 seconds. Returns `TgError::LockTimeout` on failure.

**`TgError` variants:**
- User errors (exit 1): `ItemNotFound`, `InvalidTransition`, `AmbiguousId`, `CycleDetected`, `AlreadyClaimed`, `InvalidInput`, `NotInitialized`, `DependentExists`
- System errors (exit 2): `StorageCorruption`, `LockTimeout(Duration)`, `IoError`, `IdCollisionExhausted`, `SchemaVersionUnsupported`

**Store config:** `.task-golem/config.yaml` with `id_prefix` field (default: `"tg"`). Uses `serde_yaml = "0.9"`.

**`Cargo.toml`:** Binary-only, edition 2024. Key deps: `chrono`, `serde`, `serde_json`, `serde_yaml = "0.9"`, `fd-lock`, `hex`, `rand`, `tempfile`, `thiserror`. No `[lib]` section currently.

### Existing Patterns

1. **Atomic writes** — Both codebases use write-temp-rename-fsync. Phase-golem uses `tempfile::NamedTempFile::persist()`. Task-golem uses the same pattern in `write_atomic()`.
2. **Actor pattern** — Phase-golem coordinator uses `tokio::sync::mpsc` with oneshot reply channels. All state mutations serialized through the actor. Retained post-migration.
3. **Pure scheduling** — `select_actions()` is a pure function (no I/O, no async). Takes a snapshot, returns actions. This separation makes it testable.
4. **`spawn_blocking` for sync ops** — Phase-golem already wraps blocking git operations in `spawn_blocking`. Same pattern for task-golem's `with_lock()`.
5. **Extension field discipline** — Task-golem enforces `x-` prefix on all extension keys, rejects collisions with known field names, validates on load. `BTreeMap` for deterministic JSON order.
6. **Error types** — Task-golem uses `thiserror` for `TgError` with `exit_code()` method. Phase-golem currently uses `Result<T, String>` throughout — the PRD specifies adopting a proper error enum.
7. **Timestamp format difference** — Phase-golem: `String` timestamps (`chrono::Utc::now().to_rfc3339()`). Task-golem: `DateTime<Utc>` natively (ISO 8601 via serde).

### Reusable Components

**From task-golem (to be exposed via `lib.rs`):**
- `Store` — Full CRUD: `load_active()`, `save_active()`, `append_to_archive()`, `all_known_ids()`, `with_lock()`
- `Item` — Native fields cover most needs; `extensions` BTreeMap for phase-golem-specific fields
- `Status` with transition methods (`apply_do`, `apply_done`, `apply_block`, `apply_unblock`, `apply_todo`)
- `generate_id_with_prefix()` — Random hex IDs with collision avoidance
- `TgError` — Well-structured error enum
- Dependency utilities — `would_create_cycle()`, `compute_ready_queue()`

**From phase-golem (to be retained/adapted):**
- `pending_batch_phases` coordinator state — stays in-memory, outside task-golem's lock
- Commit message formatting (`build_phase_commit_message`, `build_batch_commit_message`)
- `RunningTasks` tracking — scheduling concern, stays in scheduler
- `StructuredDescription`, `PhasePool`, `SizeLevel`, `DimensionLevel`, `BlockType` enums — stay as phase-golem types
- `FollowUp` type with custom deserializer
- `ItemUpdate` enum — adapted to mutate task-golem `Item` + extensions
- Worklog writes (`_worklog/YYYY-MM.md`)
- All scheduler logic — parameter types change but logic is equivalent

### Constraints from Existing Code

1. **No `lib.rs` in task-golem** — Must be added (Part 1) before phase-golem can depend on it
2. **No git module in task-golem** — Must be created (Part 1) with `stage_self()` and `commit(message)`
3. **Status enum width** — Phase-golem 6 variants vs. task-golem 4 variants; requires `x-pg-status` extension for sub-states
4. **`blocked_from_status` type width** — Task-golem's `Option<Status>` (4 variants) insufficient; adapter must use `x-pg-blocked-from-status`
5. **Timestamp type difference** — Phase-golem `String` vs. task-golem `DateTime<Utc>`; adapter exposes `DateTime<Utc>`, phase-golem code updated
6. **`serde_yaml` version mismatch** — Task-golem `0.9` (deprecated) vs. phase-golem `serde_yaml_ng 0.10`; separate crates, no type conflicts, but binary bloat
7. **Sync vs. async** — Task-golem's `with_lock()` blocks via `std::thread::sleep`; must use `spawn_blocking`
8. **`BacklogFile` fields with no task-golem equivalent** — `schema_version` (handled by JSONL header), `next_item_id` (irrelevant with hex IDs)
9. **Store requires `.task-golem/` directory** — User must run `tg init`; phase-golem does not auto-initialize
10. **ID prefix config divergence** — Task-golem in `.task-golem/config.yaml`, phase-golem in `phase-golem.toml`; phase-golem calls `generate_id_with_prefix("WRK")` directly
11. **WRK-035 uncommitted work** — `scheduler.rs` has uncommitted HashMap-based dependency lookup changes; must be resolved before/during WRK-076
12. **Edition difference** — Task-golem Rust 2024, phase-golem 2021; doesn't prevent cross-crate dependency

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Coordinator "remains a thin async actor" with read-through | Current coordinator holds in-memory `BacklogFile` and performs 12 command types — it is not thin today. Removing in-memory state is a significant refactor touching every handler. | Every handler (`UpdateItem`, `ArchiveItem`, `IngestFollowUps`, `MergeItem`, `CompletePhase`, `BatchCommit`, `GetSnapshot`, `IngestInbox`, `UnblockItem`) must be rewritten from "mutate in-memory state + save" to "`spawn_blocking` + `with_lock` + load/mutate/save". This is the largest single piece of work. |
| "Scheduler produces equivalent scheduling decisions" | `select_actions()` reads 8+ `BacklogItem` fields directly. Changing the parameter type from `&BacklogFile` to adapted items requires touching every sorting/filtering/dependency-checking function. | The adapter type (newtype `PgItem`) must expose all fields the scheduler reads. If the scheduler takes `&[PgItem]`, the `Deref` pattern or explicit accessor methods are needed for each field. |
| "New IDs use hex format (e.g., WRK-a1b2c)" | `generate_id_with_prefix()` requires `existing_ids: &HashSet<String>` — loading all known IDs (active + archive) on every ID generation. Task-golem's `all_known_ids()` does this. | At current scale (tens of items) this is negligible. But it means every follow-up ingestion and merge operation must call `all_known_ids()` inside the lock, adding a read of both active and archive files. |
| "Git module stages its own files and commits all currently-staged changes" | No git module exists in task-golem. `stage_self()` + `commit(message)` must be built. Phase-golem already has `git.rs` with `stage_paths()` and `commit()`. | The task-golem git module is simple but requires testing to ensure it works correctly with phase-golem's staging workflow (phase-golem stages artifacts first, then task-golem stages `tasks.jsonl`, then commit all). |
| WRK-035 HashMap dependency lookups | Uncommitted changes in `scheduler.rs` change `unmet_dep_summary` to take `&HashMap<&str, &BacklogItem>`. | Must decide: commit WRK-035 first and then adapt its HashMap pattern to the new types, or fold the HashMap change into WRK-076. The HashMap pattern itself is compatible with the adapter approach — just needs `&HashMap<&str, &PgItem>`. |
| `ItemUpdate` enum used for item mutations | The 10-variant `ItemUpdate` enum drives all item mutations through the coordinator. Each variant maps to specific `BacklogItem` field changes. | Each `ItemUpdate` handler must be reimplemented to mutate a task-golem `Item` + extensions via the adapter. The adapter must expose setter methods for every mutable field (phase, status, blocked fields, assessments, description, etc.). |
| "Adapter validates extension field values on deserialization" | Task-golem's `validate_extensions()` only checks key prefixes and name collisions — it does not validate extension *values*. | The adapter must implement its own value validation (e.g., `x-pg-status` must be `"new"`, `"scoping"`, or `"ready"`). Invalid values → treated as absent with warning log. This validation lives entirely in phase-golem's adapter. |

---

## Critical Areas

### 1. Coordinator Rewrite (Largest Risk Area)

**Why it's critical:** The coordinator is the nerve center of phase-golem — all state mutations and git operations flow through it. Rewriting every handler from in-memory mutation to `spawn_blocking` + `with_lock` + load/mutate/save is the highest-risk change.

**Why it's easy to miss:** The PRD describes the coordinator as a "thin async actor" post-migration, which sounds simple. But the current coordinator has 12 command types, each with specific save/rollback semantics. The refactor is not simplifying the coordinator — it is rewriting its internals while preserving its interface.

**What to watch for:**
- Each handler must now load state inside the lock, mutate, and save — previously state was in memory
- Error handling changes: `backlog::save()` failure was the only error; now there are `LockTimeout`, `StorageCorruption`, `IoError`, `JoinError` from `spawn_blocking`
- `IngestInbox` handler had rollback-on-save-failure logic; must be rethought since inbox is being removed
- `CompletePhase` and `BatchCommit` involve git operations + state updates that must be sequenced correctly (stage artifacts → update item → stage tasks.jsonl → commit)
- The `GetSnapshot` command becomes a `spawn_blocking` + `load_active()` call instead of returning a clone of in-memory state — performance characteristics change

### 2. Status Mapping Bidirectionality

**Why it's critical:** Phase-golem's 6-variant `ItemStatus` mapped onto task-golem's 4-variant `Status` via `x-pg-status` is the central data model decision. Every read and write of status must go through this mapping correctly.

**Why it's easy to miss:** The mapping seems straightforward (New/Scoping/Ready → Todo + extension, InProgress → Doing, Done → Done, Blocked → Blocked). But the reverse mapping must handle: absent `x-pg-status` on Todo (defaults to New), invalid `x-pg-status` values, and divergence from CLI-initiated status changes.

**What to watch for:**
- `is_valid_transition()` in phase-golem must still enforce the 6-state transition rules even though task-golem only knows about 4 states
- When transitioning from Todo to Doing (phase-golem: Scoping→InProgress or Ready→InProgress), the adapter must clear `x-pg-status` since it is only meaningful when task-golem status is Todo
- `blocked_from_status` has its own mapping: phase-golem stores 4 valid variants (New, Scoping, Ready, InProgress) in `x-pg-blocked-from-status`; task-golem's native field holds a lossy 4-variant mapping
- A human running `tg do` on a Todo item with `x-pg-status: "new"` transitions to Doing — the adapter must detect that `x-pg-status` is now stale (item is Doing, not Todo) and ignore it

### 3. Git Commit Sequencing

**Why it's critical:** Phase-golem's commit flow involves multiple steps that must not be interleaved: stage artifact files → update item status in task-golem → task-golem stages `tasks.jsonl` → commit all. If these steps interleave with other git operations, the commit could include unintended changes.

**Why it's easy to miss:** The coordinator actor serializes operations from phase-golem's side, but a human could run git commands concurrently. The file lock only protects the JSONL file, not the git index.

**What to watch for:**
- The coordinator must hold the `with_lock()` scope through the stage + commit sequence, or accept that another process could modify the git index between staging and committing
- `BatchCommit` accumulates phases and commits them all at once — the staging of task-golem's files must happen after all phases are accumulated, inside the commit flow
- If `git commit` fails (e.g., empty commit, hook failure), the JSONL state is still updated (JSONL is source of truth per PRD decision). But the git index is left in a dirty state with staged-but-not-committed files.

### 4. Test Migration Volume

**Why it's critical:** 16 test files, many constructing `BacklogFile`/`BacklogItem` directly. The test helpers (`make_item`, `make_backlog`, `setup_test_env`) all need rewriting.

**Why it's easy to miss:** Tests are often underestimated. Each scheduler test that constructs a `BacklogFile` with specific items must now construct task-golem `Item`s with the right extension fields set. Coordinator tests must use task-golem's store instead of writing YAML.

**What to watch for:**
- `backlog_test.rs` and `migration_test.rs` become entirely obsolete — delete them
- `scheduler_test.rs` tests are the most valuable to preserve since they validate scheduling logic. They need adapter-aware test helpers.
- `coordinator_test.rs` tests need the most rewriting — setup changes from YAML writes to `tg init` + JSONL writes
- New tests needed for the adapter layer (status mapping, extension field round-trips, validation of invalid values)

---

## Deep Dives

*None yet.*

---

## Synthesis

### Open Questions

| Question | Why It Matters | Recommendation |
|----------|----------------|----------------|
| Module-level re-export (`pub mod model;`) vs. item-level (`pub use model::item::Item;`) in task-golem's `lib.rs`? | Affects API surface and maintenance burden | Module-level. Phase-golem needs deep access to `model`, `store`, `errors`. Selective re-export is higher maintenance. |
| `spawn_blocking` vs. `block_in_place` for async/sync bridging? | Affects closure ergonomics and runtime compatibility | `spawn_blocking`. More explicit, no single-threaded runtime issues. `block_in_place` can be revisited later. |
| Relative vs. absolute path for task-golem dependency? | Portability vs. simplicity | Relative (`../task-golem`). Cargo convention, works with sibling directories. |
| Resolve WRK-035 (HashMap dep lookups) before or during WRK-076? | Uncommitted `scheduler.rs` changes will conflict with WRK-076's type changes | Commit WRK-035 first. Its HashMap pattern is compatible with the adapter approach — just needs type update to `&PgItem`. Avoids merge conflict during a complex migration. |
| Should `PgItem` use `Deref<Target=Item>` for field access? | Scheduler reads 8+ fields from items directly | No. `Deref` to a foreign type is an anti-pattern (violates Deref coercion expectations). Use explicit accessor methods or pub field access on the newtype. |
| Should the adapter hold the lock during the entire git commit sequence (stage artifacts → update item → stage tasks.jsonl → commit)? | Holding the lock blocks `tg` CLI and other processes; releasing it risks interleaved git operations | Release the lock after updating tasks.jsonl. The coordinator actor serializes phase-golem's operations. Accept that a concurrent `tg` command could interleave with the git index between stage and commit — this is an edge case the human operator controls. |

### Recommended Approaches

| Topic | Recommended Pattern | Rationale |
|-------|-------------------|-----------|
| Binary+Library crate | Pattern A: Same-package lib.rs + main.rs | Simplest, matches task-golem's structure |
| Adapter layer | Pattern A: Newtype wrapper with accessors | Preserves inner Item, typed extension access |
| Async/sync bridging | Pattern A: Direct spawn_blocking | Coordinator serialization eliminates concurrency concerns |
| Path dependencies | Pattern A: Simple relative path dep | Active co-development, tight iteration |
| Error mapping | Pattern B+C: Explicit mapping with is_retryable() | PRD requires retryable/fatal/skip categorization |
| JSONL storage | Use task-golem's existing implementation | Mature, correct, nothing to change |

### Key References

**Primary References:**
- [Cargo Targets - The Cargo Book](https://doc.rust-lang.org/cargo/reference/cargo-targets.html) -- `[lib]` and `[[bin]]` configuration
- [Specifying Dependencies - The Cargo Book](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html) -- Path and git dependency syntax
- [spawn_blocking - Tokio docs](https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html) -- Async/sync bridging API
- [Error Handling in Rust - Luca Palmieri](https://lpalmieri.com/posts/error-handling-rust/) -- Error type design and mapping patterns
- [Newtype Pattern - Rust Design Patterns](https://rust-unofficial.github.io/patterns/patterns/behavioural/newtype.html) -- Adapter pattern foundation
- [JSON Lines Specification](https://jsonlines.org/) -- JSONL format

**Supplementary References:**
- [Bridge Async and Sync Code - GreptimeDB blog](https://greptime.com/blogs/2023-03-09-bridging-async-and-sync-rust) -- Practical async/sync bridging walkthrough
- [Error Handling for Large Rust Projects - GreptimeDB](https://greptime.com/blogs/2024-05-07-error-rust) -- Error categorization at scale
- [Embrace the newtype pattern - Effective Rust](https://www.lurklurk.org/effective-rust/newtype.html) -- Newtype with From/Into
- [Serde Struct Flattening](https://serde.rs/attr-flatten.html) -- How extension fields work
- [Bridging with sync code - Tokio tutorial](https://tokio.rs/tokio/topics/bridging) -- Official Tokio guide
- [API Guidelines Discussion #167](https://github.com/rust-lang/api-guidelines/discussions/167) -- lib.rs + main.rs tradeoffs
- [thiserror crate](https://docs.rs/thiserror) -- Error enum derive macro
- [RFC 3529: Cargo Path Bases](https://rust-lang.github.io/rfcs/3529-cargo-path-bases.html) -- Future path dependency improvements

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-25 | Initial research started | Template created |
| 2026-02-25 | External research: 6 topics | All 6 topics researched with patterns, tradeoffs, references, and recommendations |
| 2026-02-25 | Internal research: both codebases | Full audit of phase-golem storage/coordinator/scheduler + task-golem model/store/errors |
| 2026-02-25 | PRD concern analysis + critical areas | 7 PRD concerns flagged, 4 critical areas identified |
