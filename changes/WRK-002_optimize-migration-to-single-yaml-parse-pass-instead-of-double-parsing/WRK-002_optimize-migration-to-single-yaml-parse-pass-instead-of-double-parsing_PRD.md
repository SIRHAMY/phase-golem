# Change: Optimize BACKLOG.yaml to Single YAML Parse Pass

**Status:** Proposed
**Created:** 2026-02-13
**Author:** Orchestrator (autonomous)

## Problem Statement

The BACKLOG.yaml schema has versioned formats (v1, v2, v3 — where v3 is current). The `backlog::load()` function in the orchestrator parses the BACKLOG.yaml file twice for the common case (v3 schema):

1. **First parse:** Deserializes the entire file into a generic `serde_yaml_ng::Value` (the project's YAML serialization library) to extract the `schema_version` field (`backlog.rs:28`)
2. **Second parse:** Deserializes the same string again into the typed `BacklogFile` struct (`backlog.rs:65`)

This is redundant because YAML parsing involves lexing, parsing, and type coercion — all of these steps are repeated identically on the same input. The `backlog::load()` path is invoked on every CLI command (`run`, `status`, `add`, `triage`, `advance`, `unblock`) and during coordinator loops.

Migration paths are worse because each migration function independently re-reads the file from disk and re-parses the version check: v2 files incur 4 total parses (version check in `load()`, version check in `migrate_v2_to_v3()`, typed parse in `migrate_v2_to_v3()`, plus a disk re-read), and v1 files incur 5+ parses through the v1-to-v2-to-v3 chain.

While the absolute performance impact is small for the current ~28KB file (~72 items), this is a code quality improvement that eliminates clearly redundant work without adding complexity. If `schema_version` is missing or unparseable, the current code defaults to v1 and triggers the migration chain — this behavior must be preserved.

## User Stories / Personas

- **Orchestrator operator** — Runs CLI commands frequently during development. Benefits from cleaner, more efficient load path that's easier to reason about.

## Desired Outcome

After this change, loading a v3 BACKLOG.yaml file should require exactly one YAML parse operation. The file content is parsed once into a `serde_yaml_ng::Value`, the `schema_version` is extracted from that Value, and then `serde_yaml_ng::from_value()` converts the already-parsed Value to the typed `BacklogFile` struct without re-parsing the YAML text.

Behavior is identical to today — same migration support, same crash-safety guarantees. Error messages should remain helpful and include file path context, though exact wording may differ between `from_value()` and `from_str()`.

## Success Criteria

### Must Have

- [ ] v3 files are parsed exactly once (no redundant `serde_yaml_ng::from_str` calls on the same content)
- [ ] All existing tests pass without modification (43+ tests in `backlog_test.rs`, 16+ in `migration_test.rs`)
- [ ] Migration paths (v1 and v2) continue to work correctly
- [ ] Error messages remain helpful and include file path context
- [ ] Pre-implementation verification that `serde_yaml_ng::from_value()` produces identical results to `from_str()` for structs using `#[serde(default)]` (write a small unit test comparing both paths on a representative BacklogFile)

### Nice to Have

- [ ] Performance benchmark demonstrating the improvement

## Scope

### In Scope

- Refactoring `backlog::load()` to use a single parse pass for v3 files: parse to `Value`, extract version, then `from_value()` to typed struct

### Out of Scope

- Refactoring migration functions to accept pre-parsed data (separate follow-up item if desired)
- Tagged enum deserialization (`#[serde(tag = "schema_version")]`) — integer tags may not work reliably with `serde_yaml_ng`, and the added complexity isn't warranted
- Caching or memoization of parsed backlog across multiple load calls
- Streaming or incremental YAML parsing
- Optimizing other YAML loading paths (inbox, config)
- Adding new schema versions or changing the migration architecture
- Changing the atomic write-temp-rename pattern used in migrations

## Non-Functional Requirements

- **Performance:** Eliminate the redundant second parse of the YAML text for v3 files. The improvement is structural (removing duplicate work), not a specific timing target.
- **Maintainability:** Resulting code should be at least as clear as the current implementation

## Constraints

- Must use `serde_yaml_ng` (existing dependency; no new parsing libraries)
- Migration crash-safety must be preserved — each migration step must persist to disk before the next step runs
- Must maintain forward compatibility (unknown fields silently ignored; `#[serde(default)]` behavior preserved)

## Dependencies

- **Depends On:** `serde_yaml_ng::from_value()` API supporting the same deserialization semantics as `from_str()` (defaults, unknown field handling). This must be verified before implementation begins.
- **Blocks:** Nothing

## Risks

- [ ] `serde_yaml_ng::from_value()` may produce different error messages than `from_str()` — mitigated by wrapping errors to preserve file path context, and verifying error quality with intentionally malformed YAML test cases
- [ ] `from_value()` may handle `#[serde(default)]` annotations differently than `from_str()` — mitigated by the pre-implementation verification test (see Must Have criteria) and existing test coverage

## Open Questions

- [ ] Does `serde_yaml_ng::from_value()` preserve identical behavior for `#[serde(default)]` fields? (Must be verified pre-implementation — see Success Criteria)

## Assumptions

Decisions made without human input:

1. **Mode: light** — This is a well-understood, small optimization with clear boundaries.
2. **Approach: Value-then-convert** — Chosen over tagged enum (risky with integer tags in YAML) and try-first (loses clear version-check semantics). Parse once to `Value`, extract version, use `from_value()` to convert.
3. **Migration functions left unchanged** — Migration crash-safety is more important than optimizing a rarely-executed path. Migration optimization is explicitly out of scope for this item.
4. **No benchmarks required** — The improvement is structurally obvious (eliminating a redundant parse). Benchmarks are nice-to-have but not blocking.

## References

- `orchestrator/src/backlog.rs` — Main load/save logic with double parse at lines 28 and 65
- `orchestrator/src/migration.rs` — Migration functions with additional redundant parses
- `serde_yaml_ng::from_value()` — API for converting parsed Value to typed struct
