# Design: Persist High-Water Mark for Item IDs

**ID:** WRK-027
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-027_persist-high-water-mark-for-item-ids-in-backlog-yaml-to-prevent-id-reuse-after-archival_PRD.md
**Tech Research:** ./WRK-027_persist-high-water-mark-for-item-ids-in-backlog-yaml-to-prevent-id-reuse-after-archival_TECH_RESEARCH.md
**Mode:** Light

## Overview

Add a `next_item_id: u32` field to `BacklogFile` that persists the highest numeric suffix ever assigned. The field uses `#[serde(default)]` for backward compatibility (defaults to 0 when absent). `generate_next_id()` incorporates this value as a floor via `max(current_items_max, next_item_id) + 1`, and returns a `(String, u32)` tuple so callers can update the field. This is a textbook application of the high-water mark pattern using existing codebase infrastructure (atomic writes, serde defaults) — no new dependencies, no architectural changes.

---

## System Design

### High-Level Architecture

This change modifies two existing components with no new modules or external dependencies:

1. **`BacklogFile` struct** (`types.rs`) — gains a `next_item_id: u32` field
2. **`generate_next_id()` function** (`backlog.rs`) — gains awareness of the persisted floor and returns the new high-water mark alongside the ID string
3. **Callers** (`add_item`, `ingest_follow_ups` in `backlog.rs`) — updated to destructure the tuple and write back the high-water mark

The existing `save()` function automatically persists the new field via serde serialization — no changes needed.

### Component Breakdown

#### `BacklogFile` (modified)

**Purpose:** Top-level struct representing the BACKLOG.yaml file.

**Change:** Add `next_item_id: u32` field with `#[serde(default)]`. Add a code comment explaining that the field stores the highest suffix ever assigned (used as a floor for ID generation).

**Optional validation (PRD Nice-to-Have):** On load, if `next_item_id` is present but less than the max current item suffix, log a warning and auto-correct to the higher value. This handles manual YAML edits that lower the field incorrectly. The `generate_next_id()` formula already self-corrects via `max()`, so validation is purely a diagnostic aid.

**Interfaces:**
- Input: Deserialized from YAML (backward-compatible — missing field defaults to 0)
- Output: Serialized to YAML (field always written on save)

**Dependencies:** serde (existing)

**Note on `BacklogSnapshot`:** The `BacklogSnapshot` struct in `types.rs` is a separate read-only view used by the scheduler. It does NOT need `next_item_id` because snapshots are never used for ID generation — only the coordinator's mutable `BacklogFile` generates IDs.

#### `generate_next_id()` (modified)

**Purpose:** Compute the next sequential item ID, guaranteed to exceed all previously assigned IDs.

**Change:** Accept `next_item_id` from `BacklogFile` as an additional floor in the max computation. Return `(String, u32)` instead of `String`.

**Interfaces:**
- Input: `&BacklogFile` (reads `items` and `next_item_id`), `&str` prefix
- Output: `(String, u32)` — the formatted ID and the new numeric suffix (which callers store back as `next_item_id`)

**Dependencies:** None (pure function)

#### `add_item()` (modified)

**Purpose:** Create a new backlog item with a generated ID.

**Change:** Destructure `generate_next_id()` return value. Update `backlog.next_item_id` with the returned suffix.

#### `ingest_follow_ups()` (modified)

**Purpose:** Convert follow-up items into new backlog items with generated IDs.

**Change:** Destructure `generate_next_id()` return value inside the `.map()` closure. Update `backlog.next_item_id` after each call (before the next iteration), matching the existing pattern of `backlog.items.push()` inside the same closure.

### Data Flow

1. `BacklogFile` loaded from YAML — `next_item_id` deserialized (or defaults to 0 if absent)
2. Caller invokes `generate_next_id(&backlog, prefix)` — function computes `max(scan_of_items, next_item_id) + 1`
3. Function returns `(formatted_id, new_suffix)` — e.g., `("WRK-003", 3)`
4. Caller sets `backlog.next_item_id = new_suffix` and uses the ID string for the new item
5. On `save()`, `next_item_id` is serialized to YAML alongside items — atomically via temp-file rename

### Key Flows

#### Flow: Single Item Addition (`add_item`)

> Add a new item to the backlog with a unique, non-reusable ID.

1. **Generate ID** — Call `generate_next_id(&backlog, prefix)`, get `(id, suffix)`
2. **Update high-water mark** — Set `backlog.next_item_id = suffix`
3. **Create item** — Build `BacklogItem` with the generated ID
4. **Push item** — Append to `backlog.items`
5. **Persist** — Caller eventually calls `save()`, which atomically writes both items and `next_item_id`

**Edge cases:**
- Empty backlog with `next_item_id = 0` — produces `WRK-001`, sets `next_item_id = 1`
- Empty backlog with `next_item_id = 5` (items were archived) — produces `WRK-006`, sets `next_item_id = 6`
- Items exist with max suffix > `next_item_id` (manual edit) — uses item scan as floor, produces correct result

#### Flow: Batch Follow-Up Ingestion (`ingest_follow_ups`)

> Ingest multiple follow-ups from a phase result, each getting a unique ID.

1. **Iterate follow-ups** — For each follow-up in the `.map()` closure:
   a. **Generate ID** — Call `generate_next_id(&backlog, prefix)`, get `(id, suffix)`
   b. **Update high-water mark** — Set `backlog.next_item_id = suffix`
   c. **Create item** — Build `BacklogItem` with the generated ID
   d. **Push item** — Append to `backlog.items` (ensures next iteration's scan sees this item)
2. **Persist** — Caller calls `save()` after ingestion

**Edge cases:**
- Multiple follow-ups with empty backlog — IDs increment correctly (`WRK-001`, `WRK-002`, ...) because both `items.push()` and `next_item_id` update happen before the next iteration

#### Flow: Backward-Compatible Load

> Load an existing BACKLOG.yaml that lacks the `next_item_id` field.

1. **Deserialize** — serde applies `#[serde(default)]`, setting `next_item_id = 0`
2. **First ID generation** — `max(current_items_max, 0) + 1` produces the correct next ID based on existing items
3. **Save** — `next_item_id` is now serialized to YAML, field present from this point forward

#### Flow: Backward-Compatible Load via v1→v2 Migration

> Load a v1 BACKLOG.yaml that goes through migration before the `next_item_id` field exists.

1. **Migration** — `load()` detects `schema_version < 2` and calls `migrate_v1_to_v2()`, producing a v2 `BacklogFile`
2. **Default applied** — The migrated struct has `next_item_id = 0` via `#[serde(default)]`
3. **First ID generation** — Same as Backward-Compatible Load: `max(current_items_max, 0) + 1` produces the correct next ID
4. **Persist** — On next `save()`, `next_item_id` is written to YAML

#### Flow: Crash Recovery

> Process crashes after generating an ID but before `save()`.

1. **On restart** — `BacklogFile` loaded from last persisted state. `next_item_id` reverts to pre-crash value.
2. **No collision** — If crash was before `save()`, the new item was never persisted either (it was only in memory). The item and high-water mark are either both persisted or neither is, because `save()` writes the entire `BacklogFile` atomically.

---

## Technical Decisions

### Key Decisions

#### Decision: `next_item_id` semantics — acts as a floor for ID generation

**Context:** The PRD specifies the field as `next_item_id` with the formula `max(current_items_max, next_item_id) + 1`. After generating an ID with suffix N, `next_item_id` is set to N. On the next call, the formula produces at least N+1. The field acts as a monotonic floor — it ensures IDs never go below the highest ever assigned, regardless of which items are currently in the backlog.

**Decision:** Follow the PRD's naming and formula exactly. The stored value is the suffix most recently assigned; the formula adds 1 to derive the next. The name `next_item_id` in the YAML file signals to human readers that this field is related to ID generation.

**Rationale:** The PRD's formula is mathematically correct and matches the standard high-water mark pattern. The name is a persistent schema decision from the PRD. A code comment on the struct field will clarify the semantics for developers.

**Consequences:** Developers should read the struct-level comment to understand the semantics. Unit tests verify the exact sequence behavior.

#### Decision: Keep `.map()` pattern in `ingest_follow_ups`

**Context:** The existing code uses `.map()` with mutation inside the closure (`backlog.items.push()`). Tech research noted a `for` loop would be clearer, but `.map()` is consistent with existing code.

**Decision:** Keep `.map()` and add `backlog.next_item_id = suffix` inside the closure alongside the existing `push()`.

**Rationale:** Consistency with existing patterns. The mutation is already happening (push); adding one more assignment doesn't increase complexity. A `for` loop refactor is out of scope.

**Consequences:** The closure mutates two fields on `backlog` (`items` via push, `next_item_id` via assignment). Both mutations are sequential within the closure, so correctness is maintained.

#### Decision: Return `(String, u32)` tuple from `generate_next_id`

**Context:** The function could return a struct, take `&mut BacklogFile` and mutate directly, or return a tuple.

**Decision:** Return `(String, u32)` tuple. The function remains pure — no mutation of `BacklogFile`.

**Rationale:** Preserves the existing pure-function pattern. Callers already have `&mut BacklogFile` and can trivially update the field. A named struct adds overhead for a two-field return.

**Consequences:** All 2 callers + 6 test call sites need updating. This is mechanical and low-risk.

### Tradeoffs Accepted

| Tradeoff | We're Accepting | In Exchange For | Why This Makes Sense |
|----------|-----------------|-----------------|---------------------|
| Field naming convention | `next_item_id` stores the floor (last-assigned suffix), not a literal "next value" | Alignment with PRD schema decision, clear formula | Code comment clarifies semantics; unit tests verify behavior |
| One-time migration gap | First run on empty backlog with no items starts from WRK-001 even if archived items exist | No need to scan worklog | Identical to current behavior; self-corrects immediately |
| Breaking callers | All `generate_next_id` call sites need updating | Pure function pattern preserved | Mechanical change, caught at compile time |

---

## Alternatives Considered

### Alternative: Mutate `BacklogFile` inside `generate_next_id`

**Summary:** Have `generate_next_id` take `&mut BacklogFile` and update `next_item_id` internally.

**How it would work:**
- Function signature changes to `fn generate_next_id(backlog: &mut BacklogFile, prefix: &str) -> String`
- Function computes the ID and sets `backlog.next_item_id` before returning

**Pros:**
- Callers don't need to remember to update the field
- Fewer lines changed at call sites

**Cons:**
- Breaks the pure-function pattern established in the codebase
- Side effects hidden inside what looks like a query function
- Harder to test (need mutable backlog even for read-only assertions)

**Why not chosen:** The existing codebase convention is pure functions with mutation at the caller. Introducing hidden mutation would be inconsistent and harder to reason about.

---

## Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Off-by-one in formula | IDs collide or skip | Low | Unit tests verify exact sequences; formula is simple |
| Caller forgets to update `next_item_id` | High-water mark stale, potential reuse | Low | Only 2 callers; compile-time tuple destructuring makes it obvious. Integration tests verify `next_item_id` is correctly persisted after `add_item()` and `ingest_follow_ups()` |
| Crash between ID gen and save | Counter reverts | Low | Safe — item and counter are either both persisted or neither (atomic write) |

---

## Integration Points

### Existing Code Touchpoints

- `orchestrator/src/types.rs:190-195` — Add `next_item_id` field to `BacklogFile`
- `orchestrator/src/backlog.rs:90-105` — Update `generate_next_id()` to use floor and return tuple
- `orchestrator/src/backlog.rs:118` — Update `add_item()` to destructure and set field
- `orchestrator/src/backlog.rs:242` — Update `ingest_follow_ups()` to destructure and set field inside `.map()`
- `orchestrator/tests/backlog_test.rs:52-57` — Update `empty_backlog()` helper to include `next_item_id: 0`
- `orchestrator/tests/backlog_test.rs:217-261` — Update 6 existing test call sites to handle tuple return
- `orchestrator/tests/backlog_test.rs` — Add 4+ new tests per PRD success criteria

### Concurrency

The `next_item_id` field is only mutated inside the coordinator's serial event loop. All ID generation happens synchronously within command handlers (e.g., `handle_ingest_follow_ups`). Concurrent orchestrator instances are not supported (PRD constraint). The atomic write in `save()` ensures the backlog and `next_item_id` are persisted together or not at all.

### External Dependencies

None. No new crates or external services.

---

## Open Questions

None — the design follows the PRD formula and approach with no ambiguity. Self-critique raised one directional item (`.map()` vs `for` loop in `ingest_follow_ups`) which was resolved: keep `.map()` for consistency with the existing codebase pattern, as the function already mutates `backlog.items` in the closure. A refactor to `for` loop is out of scope for this change.

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

- **Autonomous mode:** No human available for input. All decisions made based on PRD, tech research, and codebase analysis.
- **Mode selection:** Using "light" mode given the change is small, well-understood, and the PRD is highly detailed. One alternative was briefly considered.
- **BacklogSnapshot impact:** The `BacklogSnapshot` struct in `types.rs` also has `items` and `schema_version` but is a separate type used for snapshot purposes. It does NOT need `next_item_id` since snapshots are read-only views, not used for ID generation. If this assumption is wrong, a follow-up can add the field.

---

## Design Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Initial design draft (autonomous, light mode) | Straightforward HWM design following PRD exactly; one alternative evaluated |
| 2026-02-12 | Self-critique (7 agents) and auto-fixes | Clarified `next_item_id` semantics (not a PRD contradiction), added validation note, migration flow, concurrency section, integration test mitigation, BacklogSnapshot exclusion note |
