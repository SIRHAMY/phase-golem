use std::time::Duration;

use phase_golem::pg_error::PgError;
use task_golem::errors::TgError;
use task_golem::model::status::Status;

// --- From<TgError> mapping tests ---

#[test]
fn from_lock_timeout() {
    let duration = Duration::from_secs(5);
    let tg_err = TgError::LockTimeout(duration);
    let pg_err = PgError::from(tg_err);
    assert!(matches!(pg_err, PgError::LockTimeout(d) if d == duration));
}

#[test]
fn from_storage_corruption() {
    let tg_err = TgError::StorageCorruption("bad data".to_string());
    let pg_err = PgError::from(tg_err);
    assert!(matches!(pg_err, PgError::StorageCorruption(_)));
    // Verify recovery guidance is in the Display message
    let msg = pg_err.to_string();
    assert!(
        msg.contains("git checkout .task-golem/tasks.jsonl"),
        "Expected recovery guidance in message: {}",
        msg
    );
}

#[test]
fn from_schema_version_unsupported_maps_to_storage_corruption() {
    let tg_err = TgError::SchemaVersionUnsupported {
        found: 99,
        supported: 1,
    };
    let pg_err = PgError::from(tg_err);
    assert!(matches!(pg_err, PgError::StorageCorruption(_)));
}

#[test]
fn from_not_initialized() {
    let tg_err = TgError::NotInitialized("/some/path".to_string());
    let pg_err = PgError::from(tg_err);
    assert!(matches!(pg_err, PgError::NotInitialized(ref msg) if msg == "/some/path"));
}

#[test]
fn from_id_collision_exhausted() {
    let tg_err = TgError::IdCollisionExhausted(100);
    let pg_err = PgError::from(tg_err);
    assert!(matches!(pg_err, PgError::IdCollisionExhausted(100)));
}

#[test]
fn from_item_not_found() {
    let tg_err = TgError::ItemNotFound("WRK-abc".to_string());
    let pg_err = PgError::from(tg_err);
    assert!(matches!(pg_err, PgError::ItemNotFound(ref id) if id == "WRK-abc"));
}

#[test]
fn from_invalid_transition() {
    let tg_err = TgError::InvalidTransition {
        from: Status::Done,
        to: Status::Todo,
    };
    let pg_err = PgError::from(tg_err);
    assert!(matches!(pg_err, PgError::InvalidTransition(_)));
}

#[test]
fn from_cycle_detected() {
    let tg_err = TgError::CycleDetected("a -> b -> a".to_string());
    let pg_err = PgError::from(tg_err);
    assert!(matches!(pg_err, PgError::CycleDetected(ref msg) if msg == "a -> b -> a"));
}

#[test]
fn from_ambiguous_id_maps_to_unexpected() {
    let tg_err = TgError::AmbiguousId {
        prefix: "ab".to_string(),
        matches: vec!["abc".to_string(), "abd".to_string()],
    };
    let pg_err = PgError::from(tg_err);
    assert!(matches!(pg_err, PgError::Unexpected(_)));
}

#[test]
fn from_already_claimed_maps_to_unexpected() {
    let tg_err = TgError::AlreadyClaimed("agent-1".to_string());
    let pg_err = PgError::from(tg_err);
    assert!(matches!(pg_err, PgError::Unexpected(_)));
}

#[test]
fn from_invalid_input_maps_to_unexpected() {
    let tg_err = TgError::InvalidInput("bad input".to_string());
    let pg_err = PgError::from(tg_err);
    assert!(matches!(pg_err, PgError::Unexpected(_)));
}

#[test]
fn from_dependent_exists_maps_to_unexpected() {
    let tg_err = TgError::DependentExists("a".to_string(), "b".to_string());
    let pg_err = PgError::from(tg_err);
    assert!(matches!(pg_err, PgError::Unexpected(_)));
}

#[test]
fn from_io_error_maps_to_unexpected() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let tg_err = TgError::IoError(io_err);
    let pg_err = PgError::from(tg_err);
    assert!(matches!(pg_err, PgError::Unexpected(_)));
}

// --- is_retryable() tests ---

#[test]
fn is_retryable_lock_timeout() {
    let err = PgError::LockTimeout(Duration::from_secs(5));
    assert!(err.is_retryable());
}

#[test]
fn is_retryable_false_for_non_retryable() {
    let cases: Vec<PgError> = vec![
        PgError::StorageCorruption(TgError::StorageCorruption("bad".to_string())),
        PgError::NotInitialized("path".to_string()),
        PgError::IdCollisionExhausted(10),
        PgError::InternalPanic("panic".to_string()),
        PgError::ItemNotFound("id".to_string()),
        PgError::InvalidTransition(TgError::InvalidTransition {
            from: Status::Done,
            to: Status::Todo,
        }),
        PgError::CycleDetected("cycle".to_string()),
        PgError::Git("git error".to_string()),
        PgError::Unexpected(TgError::InvalidInput("x".to_string())),
    ];

    for err in cases {
        assert!(
            !err.is_retryable(),
            "Expected is_retryable=false for {:?}",
            err
        );
    }
}

// --- is_fatal() tests ---

#[test]
fn is_fatal_for_fatal_variants() {
    let cases: Vec<PgError> = vec![
        PgError::StorageCorruption(TgError::StorageCorruption("bad".to_string())),
        PgError::NotInitialized("path".to_string()),
        PgError::IdCollisionExhausted(10),
        PgError::InternalPanic("panic".to_string()),
    ];

    for err in cases {
        assert!(err.is_fatal(), "Expected is_fatal=true for {:?}", err);
    }
}

#[test]
fn is_fatal_false_for_non_fatal() {
    let cases: Vec<PgError> = vec![
        PgError::LockTimeout(Duration::from_secs(5)),
        PgError::ItemNotFound("id".to_string()),
        PgError::InvalidTransition(TgError::InvalidTransition {
            from: Status::Done,
            to: Status::Todo,
        }),
        PgError::CycleDetected("cycle".to_string()),
        PgError::Git("git error".to_string()),
        PgError::Unexpected(TgError::InvalidInput("x".to_string())),
    ];

    for err in cases {
        assert!(!err.is_fatal(), "Expected is_fatal=false for {:?}", err);
    }
}

// --- Display / Error chain tests ---

#[test]
fn storage_corruption_display_includes_recovery_guidance() {
    let err = PgError::StorageCorruption(TgError::StorageCorruption("truncated file".to_string()));
    let msg = err.to_string();
    assert!(msg.contains("Recovery:"));
    assert!(msg.contains("git checkout .task-golem/tasks.jsonl"));
}

#[test]
fn error_source_chain_preserved_for_storage_corruption() {
    use std::error::Error;
    let tg_err = TgError::StorageCorruption("bad".to_string());
    let pg_err = PgError::StorageCorruption(tg_err);
    let source = pg_err
        .source()
        .expect("StorageCorruption should have a source");
    assert!(source.to_string().contains("bad"));
}

#[test]
fn error_source_chain_preserved_for_invalid_transition() {
    use std::error::Error;
    let tg_err = TgError::InvalidTransition {
        from: Status::Done,
        to: Status::Todo,
    };
    let pg_err = PgError::InvalidTransition(tg_err);
    let source = pg_err
        .source()
        .expect("InvalidTransition should have a source");
    assert!(source.to_string().contains("cannot transition"));
}

#[test]
fn error_source_chain_preserved_for_unexpected() {
    use std::error::Error;
    let tg_err = TgError::InvalidInput("bad input".to_string());
    let pg_err = PgError::Unexpected(tg_err);
    let source = pg_err.source().expect("Unexpected should have a source");
    assert!(source.to_string().contains("bad input"));
}
