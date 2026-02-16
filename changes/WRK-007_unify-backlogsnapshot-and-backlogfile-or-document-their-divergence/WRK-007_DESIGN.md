# Design: Eliminate BacklogSnapshot by Using BacklogFile Directly

**ID:** WRK-007
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-007_PRD.md
**Tech Research:** ./WRK-007_TECH_RESEARCH.md
**Mode:** Light

## Overview

Eliminate the `BacklogSnapshot` type entirely and replace all its usages with `BacklogFile`. The coordinator, scheduler, and filter modules will all use `BacklogFile` (passed as `&BacklogFile` for read-only consumers). This removes a redundant projection type that differs from `BacklogFile` only by omitting a single `u32` field (`next_item_id`). Immutability is enforced by Rust's borrow checker via `&BacklogFile` references — no separate "view" type is needed.

---

## System Design

### High-Level Architecture

The change is a type-level refactoring with no architectural changes. The existing actor model is preserved:

```
BacklogFile (types.rs)     <-- single type, remove BacklogSnapshot
      |
      v
Coordinator (coordinator.rs)
  - Owns BacklogFile as internal state
  - handle_get_snapshot() clones BacklogFile (was: projected to BacklogSnapshot)
  - Sends owned BacklogFile through oneshot channel
      |
      v
Scheduler (scheduler.rs)
  - Receives &BacklogFile (was: &BacklogSnapshot)
  - Pure functions: select_actions(), select_targeted_actions(), advance_to_next_active_target()
  - Reads items and schema_version; ignores next_item_id
      |
      v
Filter (filter.rs)
  - apply_filter() takes &BacklogFile, returns BacklogFile (was: BacklogSnapshot)
  - Carries forward schema_version and next_item_id from input
```

### Prerequisites

**Add `Default` derive to `BacklogFile`** — This must be done before updating test construction sites. Without it, `..Default::default()` won't compile. Per tech research, this is easy to miss because the PRD's success criteria assume `Default` exists but don't list adding it as a prerequisite. Default values (`schema_version: 0`, `items: vec![]`, `next_item_id: 0`) are sensible and match `#[serde(default)]` behavior.

### Component Breakdown

#### types.rs — BacklogFile (modified)

**Purpose:** Single struct representing backlog state.

**Changes:**
- Add `Default` derive to `BacklogFile` (prerequisite for test construction)
- Remove `BacklogSnapshot` struct entirely

**After:**
```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct BacklogFile {
    pub schema_version: u32,
    #[serde(default)]
    pub items: Vec<BacklogItem>,
    #[serde(default)]
    pub next_item_id: u32,
}
```

#### coordinator.rs — Snapshot handling (modified)

**Purpose:** Coordinator sends backlog state to consumers.

**Changes:**
- `CoordinatorCommand::GetSnapshot` reply type: `oneshot::Sender<BacklogFile>`
- `handle_get_snapshot()` returns `BacklogFile` by cloning the coordinator's internal state
- `CoordinatorHandle::get_snapshot()` returns `Result<BacklogFile, String>`

**After (`handle_get_snapshot`):**
```rust
fn handle_get_snapshot(state: &CoordinatorState) -> BacklogFile {
    state.backlog.clone()
}
```

This is simpler than the current version — a direct clone instead of manual field mapping. The `next_item_id` is included but harmless to readers. Since the coordinator clones and sends an owned value through the oneshot channel, consumers receive owned copies with no lifetime annotation changes needed (avoiding a common pitfall when eliminating projection types).

#### scheduler.rs — Function signatures (modified)

**Purpose:** Schedule actions based on backlog state.

**Changes:** Replace `&BacklogSnapshot` with `&BacklogFile` in:
- `select_actions(snapshot: &BacklogFile, ...)`
- `advance_to_next_active_target(..., snapshot: &BacklogFile)`
- `select_targeted_actions(snapshot: &BacklogFile, ...)`

No body changes needed — these functions access only `.items` and `.schema_version`, which exist on both types.

#### filter.rs — apply_filter (modified)

**Purpose:** Filter backlog items by criteria.

**Changes:** `apply_filter()` takes `&BacklogFile` and returns `BacklogFile`.

**After:**
```rust
pub fn apply_filter(criterion: &FilterCriterion, backlog: &BacklogFile) -> BacklogFile {
    let items = backlog
        .items
        .iter()
        .filter(|item| matches_item(criterion, item))
        .cloned()
        .collect();

    // next_item_id is carried forward for structural completeness only.
    // Filtered results are never persisted; the coordinator owns ID generation.
    BacklogFile {
        items,
        schema_version: backlog.schema_version,
        next_item_id: backlog.next_item_id,
    }
}
```

**Implementation note:** The comment above `next_item_id` carry-forward is required — it prevents future developers from assuming the value is semantically meaningful in filtered output.

#### main.rs — Filter preview (modified)

**Purpose:** Preview filter results from CLI.

**Changes:** Replace manual `BacklogSnapshot` construction (lines 343-346) with `BacklogFile` construction. Since `backlog` is already a `BacklogFile` loaded from disk, pass `&backlog` directly to `apply_filter()`.

**After:**
```rust
let matching = filter::apply_filter(criterion, &backlog);
```

This is simpler — no manual field mapping needed since `backlog` is already a `BacklogFile`.

### Data Flow

1. **Load:** `backlog::load()` reads YAML into `BacklogFile`
2. **Store:** Coordinator holds `BacklogFile` as internal state
3. **Snapshot:** `handle_get_snapshot()` clones `BacklogFile` and sends through channel
4. **Schedule:** Scheduler receives owned `BacklogFile`, passes `&BacklogFile` to pure functions
5. **Filter:** `apply_filter()` takes `&BacklogFile`, returns new `BacklogFile` with filtered items

### Key Flows

#### Flow: Scheduler Snapshot Consumption

> Scheduler requests and consumes a backlog snapshot for action selection.

1. **Request** — `run_scheduler()` calls `coordinator.get_snapshot().await`
2. **Clone** — Coordinator clones its `BacklogFile` and sends through oneshot channel
3. **Receive** — Scheduler receives owned `BacklogFile`
4. **Filter** — If a filter criterion is active, `apply_filter(criterion, &snapshot)` returns filtered `BacklogFile`
5. **Select** — `select_actions(&snapshot, ...)` or `select_targeted_actions(&snapshot, ...)` produces actions
6. **Discard** — Owned `BacklogFile` is dropped at end of scheduler tick

**Edge cases:**
- Filter active: `apply_filter()` carries forward `next_item_id` and `schema_version` from input — `next_item_id` is semantically irrelevant in filtered output but carried forward for structural completeness

#### Flow: CLI Filter Preview

> User previews filter results from the command line.

1. **Load** — `backlog::load()` returns `BacklogFile` from disk
2. **Filter** — `apply_filter(criterion, &backlog)` returns filtered `BacklogFile`
3. **Display** — Matching items are printed to console

No manual `BacklogSnapshot` construction needed — `backlog` is already a `BacklogFile`.

---

## Technical Decisions

### Key Decisions

#### Decision: Direct Clone Instead of Field Mapping

**Context:** `handle_get_snapshot()` currently maps `BacklogFile` fields individually to construct `BacklogSnapshot`. With the unified type, we can simply clone.

**Decision:** Use `state.backlog.clone()` instead of field-by-field construction.

**Rationale:** Simpler code, impossible to forget a field, no mapping to maintain. The `Clone` derive already exists on `BacklogFile`.

**Consequences:** The clone includes `next_item_id`, adding 4 bytes to the cloned value. This is negligible compared to the `Vec<BacklogItem>` which dominates clone cost. If cloning becomes a performance concern at scale, WRK-024 tracks optimization via `Arc`/`im::Vector`.

#### Decision: Add Default Derive to BacklogFile

**Context:** Test helper functions construct `BacklogFile` instances and need a convenient way to omit irrelevant fields.

**Decision:** Add `Default` to `BacklogFile`'s derive list.

**Rationale:** Enables `BacklogFile { items: vec![...], ..Default::default() }` pattern in tests. Default values (0 for `u32`, empty `Vec`) are sensible and match `#[serde(default)]` behavior.

**Consequences:** No impact on production code. Tests become more concise.

#### Decision: Rename `snapshot` Parameter to `backlog` in Filter

**Context:** `apply_filter()` currently takes a parameter named `snapshot`. With the type change, this name is misleading.

**Decision:** Rename to `backlog` to match the type name.

**Rationale:** Parameter names should reflect the type. `snapshot` implied the old `BacklogSnapshot` type.

**Consequences:** Pure cosmetic — no behavioral change.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Exposing `next_item_id` | Scheduler/filter can see the field (but not mutate via `&`) | Single type, no projection maintenance, compiler-enforced immutability | `next_item_id` is a harmless `u32`; readers ignore it; the borrow checker prevents mutation |
| Larger clone | Clone includes one extra `u32` (4 bytes) | Simpler clone via `state.backlog.clone()` | 4 bytes is negligible; `Vec<BacklogItem>` dominates clone cost. WRK-024 tracks clone optimization if needed |

---

## Alternatives Considered

### Alternative: Newtype Wrapper (`BacklogView<'a>(&'a BacklogFile)`)

**Summary:** Create a newtype that wraps `&BacklogFile` and exposes only `items` and `schema_version` via methods, hiding `next_item_id`.

**How it would work:**
- Define `pub struct BacklogView<'a>(&'a BacklogFile)` in `types.rs`
- Add methods: `fn items(&self) -> &[BacklogItem]`, `fn schema_version(&self) -> u32`
- Scheduler/filter functions take `BacklogView` instead of `&BacklogFile`

**Pros:**
- Explicitly hides `next_item_id` at the type level
- Zero runtime cost (reference wrapper)

**Cons:**
- Requires manual method forwarding (boilerplate)
- Introduces lifetime annotations into scheduler/filter signatures
- Over-engineered for hiding a single harmless `u32`
- Still requires maintaining the wrapper if `BacklogFile` gains new fields

**Why not chosen:** The cure is worse than the disease. The whole point is to eliminate a projection type — replacing `BacklogSnapshot` with `BacklogView` just trades one projection for another. The `next_item_id` field carries no side effects and readers can trivially ignore it.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Missed `BacklogSnapshot` reference | Compile error (not a runtime risk) | Very Low | Compiler catches all references since the type is deleted; post-change grep confirms zero references |
| Test construction sites broken | Compile error | Low | `Default` derive enables `..Default::default()` pattern; compiler guides all fixes |

---

## Integration Points

### Existing Code Touchpoints

- `orchestrator/src/types.rs:231-240` — Add `Default` derive to `BacklogFile`
- `orchestrator/src/types.rs:264-268` — Delete `BacklogSnapshot` struct
- `orchestrator/src/coordinator.rs:13-16` — Change `GetSnapshot` reply type
- `orchestrator/src/coordinator.rs:95-99` — Change `get_snapshot()` return type
- `orchestrator/src/coordinator.rs:354-359` — Simplify `handle_get_snapshot()` to `state.backlog.clone()`
- `orchestrator/src/scheduler.rs:145,462,839` — Change parameter types from `&BacklogSnapshot` to `&BacklogFile`
- `orchestrator/src/filter.rs:144-156` — Change `apply_filter()` signature and add `next_item_id` carry-forward
- `orchestrator/src/main.rs:343-346` — Simplify to pass `&backlog` directly
- Test files — Update `BacklogSnapshot` construction to `BacklogFile` construction. Test helpers like `make_snapshot()` should be renamed to `make_backlog()` or similar, adding `next_item_id: 0` (or using `..Default::default()`). This is the bulk of the mechanical work.

### External Dependencies

None — this is a purely internal refactoring. `CoordinatorCommand::GetSnapshot` is internal to the orchestrator crate and not exposed as a public API, so there are no backward compatibility concerns.

### Serialization Note

Serde uses field names (not positions) for YAML serialization. The field order difference between `BacklogSnapshot` (`items`, `schema_version`) and `BacklogFile` (`schema_version`, `items`, `next_item_id`) has no effect on deserialization. The `#[serde(default)]` on `next_item_id` ensures backward compatibility with YAML fixtures that predate the field.

---

## Open Questions

None — all decisions resolved. The approach is validated by tech research.

---

## Design Review Checklist

Before moving to SPEC:

- [x] Design addresses all PRD requirements
- [x] Key flows are documented and make sense
- [x] Tradeoffs are explicitly documented and acceptable
- [x] Integration points with existing code are identified
- [x] No major open questions remain

---

## Assumptions

- **Autonomous mode** — No human available for review. Design decisions follow PRD direction and tech research recommendations directly.
- **Light mode appropriate** — This is a small, mechanical refactoring with one clear approach. The PRD and tech research are aligned and thorough.

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Initial design draft | Direct elimination approach documented; one alternative (newtype wrapper) considered and rejected |
| 2026-02-13 | Self-critique (7 agents) | Auto-fixed: elevated Default prerequisite, added next_item_id comment requirement, referenced WRK-024 for clone cost, added serialization/backward-compat notes, clarified test migration pattern |
