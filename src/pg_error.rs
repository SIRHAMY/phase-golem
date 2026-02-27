use std::time::Duration;

use task_golem::errors::TgError;

/// Error enum mapping task-golem errors to phase-golem categories.
///
/// Categories:
/// - Retryable: transient contention, worth retrying
/// - Fatal: halt coordinator, unrecoverable
/// - Skip: log and continue, scheduler retries next loop
/// - Git: git operation failure
/// - Unexpected: should not occur with adapter-validated input
#[derive(Debug, thiserror::Error)]
pub enum PgError {
    // Retryable
    #[error("Lock timeout after {0:?}")]
    LockTimeout(Duration),

    // Fatal -- halt coordinator
    #[error("Storage corruption: {0}. Recovery: `git checkout .task-golem/tasks.jsonl`")]
    StorageCorruption(#[source] TgError),

    #[error("Store not initialized: {0}")]
    NotInitialized(String),

    #[error("ID collision exhausted after {0} attempts")]
    IdCollisionExhausted(u32),

    #[error("Internal panic in storage thread: {0}")]
    InternalPanic(String),

    // Skip -- log and continue
    #[error("Item not found: {0}")]
    ItemNotFound(String),

    #[error("Invalid transition: {0}")]
    InvalidTransition(#[source] TgError),

    #[error("Cycle detected: {0}")]
    CycleDetected(String),

    // Git
    #[error("Git error: {0}")]
    Git(String),

    // Catch-all for unexpected variants
    #[error("Unexpected storage error: {0}")]
    Unexpected(#[source] TgError),
}

impl PgError {
    /// Returns true if the error is transient and the operation should be retried.
    pub fn is_retryable(&self) -> bool {
        matches!(self, PgError::LockTimeout(_))
    }

    /// Returns true if the error is unrecoverable and the coordinator should halt.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            PgError::StorageCorruption(_)
                | PgError::NotInitialized(_)
                | PgError::IdCollisionExhausted(_)
                | PgError::InternalPanic(_)
        )
    }
}

/// Transitional bridge: allows `?` to convert `PgError` to `String` in code
/// that still uses `Result<T, String>` (scheduler, executor, etc.).
/// TODO: Remove when all consumers adopt `PgError` directly.
impl From<PgError> for String {
    fn from(err: PgError) -> String {
        err.to_string()
    }
}

impl From<TgError> for PgError {
    fn from(err: TgError) -> Self {
        match err {
            TgError::LockTimeout(d) => PgError::LockTimeout(d),

            TgError::StorageCorruption(_) => PgError::StorageCorruption(err),

            TgError::SchemaVersionUnsupported { .. } => PgError::StorageCorruption(err),

            TgError::NotInitialized(ref msg) => PgError::NotInitialized(msg.clone()),

            TgError::IdCollisionExhausted(n) => PgError::IdCollisionExhausted(n),

            TgError::ItemNotFound(ref id) => PgError::ItemNotFound(id.clone()),

            TgError::InvalidTransition { .. } => PgError::InvalidTransition(err),

            TgError::CycleDetected(ref msg) => PgError::CycleDetected(msg.clone()),

            TgError::AmbiguousId { .. } => PgError::Unexpected(err),

            TgError::AlreadyClaimed(_) => PgError::Unexpected(err),

            TgError::InvalidInput(_) => PgError::Unexpected(err),

            TgError::DependentExists(_, _) => PgError::Unexpected(err),

            TgError::IoError(_) => PgError::Unexpected(err),
        }
    }
}
