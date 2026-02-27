# tg-ffd7b: Migrate scheduler/executor error returns from String to PgError

## Problem

The scheduler (10 functions) and executor (2 functions) still return `Result<_, String>` while the coordinator has already been migrated to `Result<_, PgError>`. This means:

- Structured error information (retryable vs fatal vs skip) is lost at the scheduler/executor boundary
- The `From<PgError> for String` bridge in `pg_error.rs:69-76` exists solely to paper over this gap
- Callers cannot make intelligent retry/halt decisions based on error category
- Error messages are opaque strings instead of typed, matchable variants

## Proposed Approach

### Phase 1: Migrate function signatures

Update all 12 functions to return `Result<_, PgError>`:

**scheduler.rs** (10 functions):
- `run_scheduler()` — public entry point
- `handle_task_completion()`
- `handle_phase_success()`
- `handle_subphase_complete()`
- `handle_phase_failed()`
- `handle_phase_blocked()`
- `process_merges()`
- `handle_triage_success()`
- `handle_promote()`
- `apply_triage_result()` — public

**executor.rs** (2 functions):
- `validate_result_identity()` — public
- `resolve_or_find_change_folder()`

### Phase 2: Convert error creation sites

Replace stringly-typed errors with appropriate `PgError` variants:
- `format!("...")` / `"...".to_string()` → proper `PgError::*` variant
- `.map_err(|e| format!(...))` → `.map_err(|e| PgError::*)`
- Some errors may need new `PgError` variants or may map to `PgError::Unexpected`

### Phase 3: Update callers and tests

- `main.rs` may need to handle `PgError` from `run_scheduler()`
- Test files (`scheduler_test.rs`, `executor_test.rs`) need assertion updates
- Remove `From<PgError> for String` bridge (covered by follow-up tg-17988)

## Assessment

- **Size:** Medium (2 core files + test files + main.rs ≈ 4-5 files)
- **Complexity:** Medium — pattern is repetitive but requires deciding which `PgError` variant each string error maps to
- **Risk:** Medium — modifies public interfaces of scheduler and executor; changes error types for core pipeline functions
- **Impact:** Medium — enables proper error-based retry/halt logic; prerequisite for tg-17988 (bridge removal)

## Dependencies

- **Prerequisite for:** tg-17988 (Remove `From<PgError> for String` bridge)
- **Infrastructure ready:** `PgError` type, `From<TgError> for PgError`, coordinator migration all complete

## Assumptions

- No new `PgError` variants are likely needed — existing variants (Unexpected, Git, ItemNotFound, InvalidTransition, etc.) should cover current string error cases
- The `run_scheduler()` public return type change is acceptable since callers can pattern-match on PgError
