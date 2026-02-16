# Design: Optimize BACKLOG.yaml to Single YAML Parse Pass

**ID:** WRK-002
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-002_optimize-migration-to-single-yaml-parse-pass-instead-of-double-parsing_PRD.md
**Tech Research:** ./WRK-002_optimize-migration-to-single-yaml-parse-pass-instead-of-double-parsing_TECH_RESEARCH.md
**Mode:** Light

## Overview

Replace the double-parse in `backlog::load()` with a single parse to `serde_yaml_ng::Value`, version extraction from that Value, then `serde_yaml_ng::from_value()` to convert the already-parsed Value into the typed `BacklogFile` struct. This eliminates one full YAML parse for every v3 load (the common case) with a one-line change to the hot path.

---

## System Design

### High-Level Architecture

No architectural change. The existing `backlog::load()` function is the sole modification target. The function's signature, behavior, and callers remain unchanged. This parse-once-then-convert pattern is schema-agnostic — future schema versions (v4+) will work identically; only the version number constant and migration branching logic would change.

**Before:**
```
read file → from_str::<Value>() → extract version → from_str::<BacklogFile>() → return
                parse #1                                    parse #2
```

**After:**
```
read file → from_str::<Value>() → extract version → from_value::<BacklogFile>() → return
                parse #1                                    convert (no re-parse)
```

### Component Breakdown

#### `backlog::load()` (modified)

**Purpose:** Load and return a typed `BacklogFile` from a YAML file, migrating older schema versions as needed.

**Responsibilities:**
- Read file from disk
- Parse YAML once to `Value`
- Extract `schema_version` from `Value`
- Trigger migration chain for pre-v3 schemas (unchanged)
- Convert `Value` to typed `BacklogFile` via `from_value()` for v3 schemas
- Wrap errors with file path context

**Interfaces:**
- Input: `path: &Path`, `project_root: &Path`
- Output: `Result<BacklogFile, String>`

**Dependencies:** `serde_yaml_ng::from_str`, `serde_yaml_ng::from_value`, `crate::migration`

### Data Flow

1. **Read** — `fs::read_to_string(path)` reads YAML content into a `String`
2. **Parse** — `serde_yaml_ng::from_str::<Value>(&contents)` parses the string into a `Value` tree (single parse)
3. **Extract version** — `value.get("schema_version").and_then(|v| v.as_u64()).unwrap_or(1)` extracts the schema version
4. **Branch on version:**
   - **v1/v2:** Trigger migration chain (unchanged — migrations re-read from disk, which is acceptable since they're rare and must persist intermediate state)
   - **v3:** `serde_yaml_ng::from_value::<BacklogFile>(value)` converts the parsed Value into the typed struct without re-parsing
   - **Other:** Return error for unsupported version
5. **Return** — Return the `BacklogFile`

### Key Flows

#### Flow: Load v3 BacklogFile (happy path, common case)

> Load a current-version BACKLOG.yaml file with a single parse operation.

1. **Read file** — `fs::read_to_string(path)` reads the file content
2. **Parse to Value** — `serde_yaml_ng::from_str::<Value>(&contents)` — single YAML parse
3. **Extract version** — Get `schema_version` from Value; it's 3
4. **Validate version** — Confirm `schema_version == EXPECTED_SCHEMA_VERSION`
5. **Convert to typed struct** — `serde_yaml_ng::from_value::<BacklogFile>(version_check)` — no re-parse; Value is moved (consumed by value, not by reference). Version extraction (step 3) must complete before this step because the Value cannot be accessed after being moved into `from_value()`. Both `from_str()` and `from_value()` return the same error type (`serde_yaml_ng::Error`), so the existing `.map_err()` wrapper works without modification.
6. **Return** — Return `Ok(backlog)`

**Edge cases:**
- Missing `schema_version` field — defaults to 1, triggers migration (unchanged behavior)
- Non-integer `schema_version` — defaults to 1, triggers migration (unchanged behavior)
- Malformed YAML — error from `from_str` with file path context (unchanged)
- Valid YAML but invalid BacklogFile structure — error from `from_value` with file path context (new: loses line/column info, acceptable for machine-managed file)

#### Flow: Load v1/v2 BacklogFile (migration path, unchanged)

> Migration path is explicitly out of scope and remains unchanged.

1. **Parse to Value** — Same as v3 flow
2. **Extract version** — Version is 1 or 2
3. **Trigger migration chain** — Calls `migrate_v1_to_v2()` and/or `migrate_v2_to_v3()` which re-read from disk (this is correct — each migration writes to disk before the next runs)
4. **Return migrated result** — Returns the BacklogFile from the migration function

No change to migration flows. In the v1/v2 branch, `contents` is unused because migrations re-read from disk. In the v3 branch, `contents` is also unused because `from_value()` replaces the previous `from_str(&contents)`. The `contents` variable remains in scope for the `from_str::<Value>()` call that produces the parsed Value; the Rust compiler may warn about it being unused after the v3 branch, which is acceptable (the variable is consumed by `from_str` on line 28).

---

## Technical Decisions

### Key Decisions

#### Decision: Use `from_value()` instead of second `from_str()`

**Context:** The v3 load path currently parses the same YAML string twice — once for version check, once for typed deserialization.

**Decision:** Replace the second `from_str::<BacklogFile>(&contents)` with `from_value::<BacklogFile>(version_check)`, reusing the already-parsed `Value`.

**Rationale:**
- `from_value()` performs the same serde deserialization as `from_str()` but skips YAML lexing/parsing since the data is already in a `Value` tree
- Tech research confirmed all `#[serde(default)]` and `#[serde(skip_serializing_if)]` attributes work identically via `from_value()`
- Custom `Deserialize` impl on `FollowUp` works through the same `Deserializer` trait
- No new dependencies — `from_value()` is already in `serde_yaml_ng` 0.10

**Consequences:**
- Error messages from `from_value()` lose line/column position information
- The `contents` string variable becomes unused after the version check (only needed for migration path, and migrations re-read from disk)

#### Decision: Accept loss of line/column info in error messages

**Context:** `from_value()` cannot report line/column positions because the `Value` tree doesn't retain source location metadata.

**Decision:** Accept this minor degradation.

**Rationale:**
- BACKLOG.yaml is machine-managed (written by `backlog::save()` using atomic writes)
- Error descriptions (field names, type mismatches) are preserved — only positional info is lost
- File path context is added by `.map_err()` and is unaffected
- PRD explicitly anticipated and accepted this tradeoff

**Consequences:** If a user manually edits BACKLOG.yaml and introduces an error, the error message won't include line/column. The field name and file path will still be reported.

**Error message example:**
- Before (`from_str`): `"Failed to parse YAML from /path/to/BACKLOG.yaml: items[3].status: unknown variant 'invalid' at line 47 column 5"`
- After (`from_value`): `"Failed to parse YAML from /path/to/BACKLOG.yaml: items[3].status: unknown variant 'invalid'"`

The field path and error description are preserved; only the `at line N column M` suffix is lost.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Error position info | Loss of line/column in `from_value()` errors | Eliminating redundant YAML parse | File is machine-managed; error descriptions still included |
| Migration path unchanged | v1/v2 migrations still double-parse | Minimal change scope, preserving crash-safety | Migrations are one-time events; optimizing them would require changing migration function signatures to accept pre-parsed `Value`, which risks breaking the crash-safety guarantee that each migration step persists to disk before the next runs. Separate follow-up item if desired. |

---

## Alternatives Considered

### Alternative: Tagged Enum Deserialization

**Summary:** Use `#[serde(tag = "schema_version")]` on an enum with variants for each schema version, letting serde route to the correct type in a single pass.

**How it would work:**
- Define `enum VersionedBacklog { V1(V1BacklogFile), V2(V2BacklogFile), V3(BacklogFile) }`
- Annotate with `#[serde(tag = "schema_version")]`
- Parse once with `from_str::<VersionedBacklog>()`

**Pros:**
- Single parse, no manual version extraction
- Type-safe version dispatch

**Cons:**
- Integer tags may not work reliably with `serde_yaml_ng` (YAML's type coercion for integers is different from JSON's)
- Requires maintaining V1/V2 type definitions alongside the current types
- More complex than the recommended approach for minimal additional benefit
- Tech research flagged this as risky

**Why not chosen:** Higher complexity and reliability risk for negligible benefit over the simpler parse-then-convert approach.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| `from_value()` behavioral difference with `#[serde(default)]` | Incorrect deserialization of BacklogFile fields | Very Low | Tech research confirmed identical behavior; existing 43+ tests provide regression safety; PRD requires pre-implementation verification test |
| Error message quality degradation | Harder debugging of malformed BACKLOG.yaml | Low | File is machine-managed; error descriptions preserved; file path context preserved |

---

## Integration Points

### Existing Code Touchpoints

- `.claude/skills/changes/orchestrator/src/backlog.rs` line 65 — Replace `serde_yaml_ng::from_str(&contents)` with `serde_yaml_ng::from_value(version_check)` and update the `.map_err()` message

### External Dependencies

- `serde_yaml_ng` 0.10 — Already a project dependency; `from_value()` is available with signature `fn from_value<T: DeserializeOwned>(value: Value) -> Result<T, Error>`. Takes `Value` by value (ownership transfer). Returns the same `Error` type as `from_str()`.

---

## Open Questions

None. All questions from the PRD and tech research phases have been resolved:
- `#[serde(default)]` behavior: Confirmed identical between `from_str()` and `from_value()`
- Error message impact: Confirmed acceptable (loss of line/column only)
- API availability: Confirmed in `serde_yaml_ng` 0.10

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain (or they're flagged for spec phase)

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Initial design draft | Single-change design: replace `from_str` with `from_value` on line 65 of backlog.rs |
| 2026-02-13 | Self-critique (7 agents) | Auto-fixed 8 clarity issues: `contents` variable lifecycle, move semantics, error message example, migration scope rationale, test criteria, error type compatibility, schema-agnostic note, variable naming. No directional or blocking issues found. |

## Assumptions

Decisions made without human input:

1. **Light mode confirmed** — PRD specified light mode; this is a single-function, single-line change with well-understood behavior.
2. **Variable naming** — The `version_check` variable name becomes semantically inaccurate after this change since it now serves double duty (version extraction and `from_value()` input). Consider renaming to `parsed_yaml` during implementation for clarity per the project's "explicit over implicit" style principle. Left as an implementation-phase decision.
3. **`contents` variable lifecycle** — After this change, `contents` is consumed by `from_str::<Value>()` on line 28 and is not directly referenced again in the v3 path (previously it was passed to `from_str::<BacklogFile>()` on line 65). The variable remains in scope but is unused after the `from_str::<Value>()` call. The Rust compiler will not warn because `contents` is consumed (moved into `from_str`). No dead code is introduced.
4. **Pre-implementation verification test** — Per PRD requirement, a unit test will be written during implementation comparing `from_value()` vs `from_str()` output. Acceptance criteria: (a) parse a representative BacklogFile YAML string via both `from_str::<BacklogFile>()` and `from_str::<Value>()` then `from_value::<BacklogFile>()`, (b) assert both produce identical `BacklogFile` instances (`PartialEq`), (c) include a BacklogFile with missing optional fields, `#[serde(default)]` fields, and FollowUp items (custom deserializer).
