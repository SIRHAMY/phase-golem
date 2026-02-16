# Tech Research: Persist High-Water Mark for Item IDs

**ID:** WRK-027
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-027_persist-high-water-mark-for-item-ids-in-backlog-yaml-to-prevent-id-reuse-after-archival_PRD.md
**Mode:** Medium

## Overview

Researching how to safely persist a monotonic counter (`next_item_id: u32`) in `BACKLOG.yaml` to prevent sequential item ID reuse after archival. The counter serves as a floor for ID generation — even when all items are archived and the backlog is empty, new IDs continue incrementing past previously-used values. Key questions: what patterns exist for file-persisted counters, how to handle crash safety, and what the existing codebase already provides.

## Research Questions

- [x] What patterns exist for persisting monotonic counters in file-based systems?
- [x] How does the existing codebase handle atomic writes and backward-compatible schema changes?
- [x] What are common pitfalls with file-persisted counters (off-by-one, crash safety, naming)?
- [x] What changes are needed to the existing `generate_next_id()` and its callers?

---

## External Research

### Landscape Overview

The problem of persisting monotonic sequence counters to prevent ID reuse is well-understood across file-based systems, databases, and distributed architectures. For single-writer, file-based systems like this orchestrator, the solution is straightforward: store a high-water mark alongside the data and use atomic write-rename to ensure crash consistency. The main categories of approaches are:

1. **High-Water Mark Pattern** — persist the highest value ever assigned as a floor for future generation
2. **Atomic Write-Rename** — ensure counter and data persist together atomically
3. **Write-Ahead Logging (WAL)** — overkill for this use case but used in databases
4. **Timestamp-Based IDs (ULID/Snowflake)** — alternative that avoids counters entirely but breaks human-readable format

### Common Patterns & Approaches

#### Pattern: High-Water Mark (HWM)

**How it works:** Maintain a persistent counter that only moves forward. Compute next value as `max(current_max_in_dataset, persisted_counter) + 1`. Persist the counter atomically with data changes.

**When to use:** When items can be removed from the active dataset but IDs must never be reused. Single-writer systems where coordination overhead is undesirable.

**Tradeoffs:**
- Pro: Simple to implement, prevents ID reuse, crash-safe when persisted atomically with data
- Pro: Recovery invariant is straightforward: `next = max(persisted_counter, max(existing_items) + 1)`
- Con: Creates gaps in ID sequences (acceptable for this use case)
- Con: Doesn't work for multiple distributed writers without coordination

**References:**
- [High-Water Mark Pattern - Martin Fowler](https://martinfowler.com/articles/patterns-of-distributed-systems/high-watermark.html)

#### Pattern: Atomic Write-Rename

**How it works:** Write new state to temp file, fsync, rename to target path. Either old or new state is visible after crash, never partial state.

**When to use:** Any time you persist critical state to a file and need crash consistency.

**Tradeoffs:**
- Pro: Provides atomicity guarantees on POSIX systems
- Pro: Well-understood, used by SQLite, LevelDB, etc.
- Con: Requires explicit fsync (often forgotten)
- Con: Rename is not cross-filesystem

**References:**
- [Files are hard - Dan Luu](https://danluu.com/file-consistency/)
- [Durability: Linux File APIs](https://www.evanjones.ca/durability-filesystem.html)
- [On Complexity of Crash-Consistent Applications (USENIX)](https://www.usenix.org/system/files/conference/osdi14/osdi14-paper-pillai.pdf)

### Technologies & Tools

#### Rust Crates

| Technology | Purpose | Relevance |
|------------|---------|-----------|
| [tempfile](https://docs.rs/tempfile/latest/tempfile/) | Atomic file writes via NamedTempFile | Already used in codebase |
| serde `#[serde(default)]` | Backward-compatible field addition | Already used in codebase |

No new dependencies needed. The codebase already has everything required.

### Standards & Best Practices

1. **Counter naming:** Use `next_item_id` (the value to assign next) rather than `last_assigned_id` to avoid off-by-one confusion
2. **Recovery validation:** On load, verify `next_item_id >= max(current_items)` and auto-correct if not
3. **Atomic persistence:** Counter and data must be saved together in the same atomic write operation
4. **Backward compatibility:** `#[serde(default)]` with a zero default is the standard Rust/serde pattern for additive schema changes

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Off-by-one in naming (`last_id` vs `next_id`) | Unclear whether to increment before or after use | Name it `next_item_id` — the value to assign next |
| Crash between ID generation and save | Counter reverts, ID could be reused | Persist counter and items atomically in same file; on recovery, scan items as floor |
| Missing fsync before rename | File contents may not be durable despite rename succeeding | Call `sync_all()` on temp file before `persist()` (already done in codebase) |
| Not testing backward compatibility | Old YAML without field fails to load | Write integration test loading old YAML, verify default applied |
| Updating counter only at end of batch | Intermediate iterations don't see updated high-water mark | Update `next_item_id` after each call in loops |

### Key Learnings

- The PRD's approach is well-aligned with industry best practices — this is a textbook application of the high-water mark pattern
- No new dependencies or complex patterns are needed
- The existing codebase already provides all required infrastructure (atomic writes, serde defaults)

---

## Internal Research

### Existing Codebase State

The orchestrator generates sequential IDs via `generate_next_id()` in `backlog.rs`. It scans all current items for the highest numeric suffix and increments by 1. Items are archived via `archive_item()` which removes them from `BACKLOG.yaml`. The `save()` function uses `NamedTempFile` + `sync_all()` + `persist()` for atomic writes.

**Relevant files/modules:**

- `orchestrator/src/types.rs:190-195` — `BacklogFile` struct definition
- `orchestrator/src/backlog.rs:85-105` — `generate_next_id()` implementation
- `orchestrator/src/backlog.rs:110-145` — `add_item()` (caller 1)
- `orchestrator/src/backlog.rs:227-271` — `ingest_follow_ups()` (caller 2)
- `orchestrator/src/backlog.rs:50-83` — `save()` atomic write implementation
- `orchestrator/tests/backlog_test.rs:217-261` — existing ID generation tests (6 test call sites)
- `orchestrate.toml` — contains `prefix = "WRK"` configuration

**Current `BacklogFile` struct:**
```rust
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BacklogFile {
    pub schema_version: u32,
    #[serde(default)]
    pub items: Vec<BacklogItem>,
}
```

**Current `generate_next_id()`:**
```rust
pub fn generate_next_id(backlog: &BacklogFile, prefix: &str) -> String {
    let prefix_with_dash = format!("{}-", prefix);
    let max_num = backlog.items.iter()
        .filter_map(|item| {
            item.id.strip_prefix(&prefix_with_dash)
                .and_then(|suffix| suffix.parse::<u32>().ok())
        })
        .max()
        .unwrap_or(0);
    format!("{}-{:03}", prefix, max_num + 1)
}
```

**Callers of `generate_next_id()`:**
1. `add_item()` (line 118) — has `&mut BacklogFile`, single call
2. `ingest_follow_ups()` (line 242) — has `&mut BacklogFile`, called once per follow-up inside `.map()` closure

**Existing patterns in use:**
- `#[serde(default)]` for backward-compatible field additions (used on `items` field)
- Pure function pattern: `generate_next_id()` takes `&BacklogFile`, returns computed value
- Atomic writes via `NamedTempFile` + `sync_all()` + `persist()`
- Test helpers: `empty_backlog()`, `make_item()` for test setup

### Reusable Components

- `save()` function — already handles atomic persistence, no changes needed
- `#[serde(default)]` pattern — already established for `items` field
- Test helpers (`empty_backlog()`, `make_item()`) — can be reused for new tests
- Test fixtures in `tests/fixtures/` — add new fixture for backward compatibility test

### Constraints from Existing Code

- `generate_next_id()` is a pure function — PRD preserves this by returning `(String, u32)` tuple instead of mutating
- `ingest_follow_ups()` uses `.map()` with mutable borrow — updating `backlog.next_item_id` inside the closure is valid (same as `backlog.items.push()`), OR can be refactored to a `for` loop for clarity
- `BacklogFile` derives `PartialEq` — new field is `u32` which implements `PartialEq`, no issue
- Schema version stays at 2 — no migration code needed

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| `.map()` closure in `ingest_follow_ups` needs refactoring to a loop | Mutation inside `.map()` is valid Rust (same as existing `backlog.items.push()`) | Either approach works; a `for` loop is more readable but `.map()` is valid |
| Formula: `max(current_items_max, next_item_id) + 1` | Correct and matches the standard HWM pattern | No change needed |
| `#[serde(default)]` defaulting to 0 | Standard Rust/serde pattern, already used in codebase | No change needed |
| No schema version bump | Correct — additive optional field is backward-compatible | No change needed |

No significant conflicts between PRD and research findings. The PRD is well-aligned with both external best practices and internal codebase patterns.

---

## Critical Areas

### Off-by-one in the formula

**Why it's critical:** Getting the formula wrong means IDs either collide or skip values unnecessarily.

**Why it's easy to miss:** The `next_item_id` stores the *next* value to assign, but the formula adds 1 to the max. Need to clarify: after `generate_next_id` returns suffix N, `next_item_id` should be set to N (the suffix just assigned), and the formula should compute `max(current_max, next_item_id - 1) + 1`. Wait — actually the PRD says the returned suffix *is* the new `next_item_id`, meaning `next_item_id` stores one *past* the last assigned. Let me re-examine...

Actually, looking at the PRD formula: `max(current_items_max, next_item_id) + 1`. If `next_item_id` starts at 0 and no items exist, the formula produces `max(0, 0) + 1 = 1`. Then `next_item_id` is updated to 1 (the returned suffix). Next call: `max(0, 1) + 1 = 2`. This means `next_item_id` stores the *last assigned* suffix, not the *next to assign*. The naming says "next" but the value is "last assigned."

**Resolution:** This is actually fine. The name `next_item_id` in YAML is slightly misleading, but the formula is correct and the unit tests will verify behavior. The PRD explicitly states: "the returned suffix is `max + 1`, which becomes the new `next_item_id`." So `next_item_id = 1` means "the last ID used suffix 1; next computation will start from max(scan, 1) + 1 = 2." This works correctly.

**What to watch for:** Unit tests must verify the exact sequence: empty backlog with `next_item_id=0` produces suffix 1, then `next_item_id=1` produces suffix 2, etc.

### Updating counter inside `.map()` vs `for` loop

**Why it's critical:** If the counter isn't updated between iterations, multiple follow-ups in a single `ingest_follow_ups` call could get the same ID (if items haven't been pushed yet).

**Why it's easy to miss:** The current code works without the counter because `backlog.items.push()` inside `.map()` ensures each subsequent `generate_next_id` scan sees the previously-pushed items. Adding `backlog.next_item_id = next_id` inside the same closure maintains the same pattern.

**What to watch for:** Both the item push AND the counter update must happen before the next iteration. The current `.map()` pattern handles this since operations are sequential within the closure.

---

## Deep Dives

No deep dives needed — the change is well-understood from initial research.

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| Should `ingest_follow_ups` use `.map()` or `for` loop? | Readability vs consistency with existing code | Either works; `.map()` is consistent with existing code but `for` loop is clearer about mutation. Recommend keeping `.map()` since it already mutates `backlog.items` in the closure. |

### Recommended Approaches

#### High-Water Mark Implementation

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `next_item_id: u32` with `#[serde(default)]` | Simple, backward-compatible, matches existing patterns | Field name slightly misleading (stores last-assigned, named "next") | Always — this is the right approach for this use case |

**Initial recommendation:** Follow the PRD exactly. The approach is well-aligned with external best practices and internal codebase patterns. No alternative approaches are worth considering.

#### Implementation Plan

1. Add `next_item_id: u32` field to `BacklogFile` with `#[serde(default)]`
2. Update `generate_next_id()` to return `(String, u32)` and incorporate `backlog.next_item_id` in the max computation
3. Update `add_item()` to destructure tuple and set `backlog.next_item_id`
4. Update `ingest_follow_ups()` to destructure tuple and set `backlog.next_item_id` in the `.map()` closure
5. Update 6 test call sites to destructure tuple
6. Add 4 new unit tests per PRD success criteria
7. Add 1 integration test for backward compatibility (load old YAML without field)
8. Optionally add validation warning on load if `next_item_id < max(items)`

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [High-Water Mark - Martin Fowler](https://martinfowler.com/articles/patterns-of-distributed-systems/high-watermark.html) | Article | Foundational pattern description |
| [Files are hard - Dan Luu](https://danluu.com/file-consistency/) | Article | Crash consistency pitfalls |
| [Serde Field Attributes](https://serde.rs/field-attrs.html) | Docs | `#[serde(default)]` reference |
| [Rust Serde Versioning](https://siedentop.dev/posts/rust-serde-versioning/) | Article | Schema evolution patterns in Rust |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | Initial landscape + codebase research (medium mode) | Confirmed PRD approach aligns with HWM pattern; identified 2 callers + 6 test sites; no new dependencies needed |
