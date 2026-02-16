# Tech Research: Eliminate BacklogSnapshot by Using BacklogFile Directly

**ID:** WRK-007
**Status:** Complete
**Created:** 2026-02-13
**PRD:** ./WRK-007_PRD.md
**Mode:** Light

## Overview

Researching whether eliminating `BacklogSnapshot` in favor of passing `&BacklogFile` directly is the correct approach, verifying all usage sites, and confirming there are no hidden constraints. The PRD proposes a mechanical refactoring — this research validates that the approach aligns with Rust idioms and that the codebase state matches PRD assumptions.

## Research Questions

- [x] Is passing `&BacklogFile` instead of a separate projection type idiomatic Rust?
- [x] Does `BacklogFile` already have the necessary derives/traits?
- [x] Are all `BacklogSnapshot` usage sites correctly identified in the PRD?
- [x] Are there any hidden constraints or gotchas in the refactoring?

---

## External Research

### Landscape Overview

The Rust ecosystem addresses the problem of duplicate/projection types through several well-established patterns centered on shared references, trait-based abstractions, and wrapper types. Rather than creating separate types for read-only projections, the idiomatic Rust approach favors using **immutable references** (`&T`) to provide read-only views. This aligns with Rust's ownership model — the borrow checker enforces read-only access at compile time, eliminating the need for separate "view" types.

### Common Patterns & Approaches

#### Pattern: Immutable References (`&T`)

**How it works:** Pass `&BacklogFile` (immutable reference) to consumers who only need read-only access instead of creating a separate snapshot type.

**When to use:** Default approach when consumers need read-only access to data. This is the fundamental Rust idiom.

**Tradeoffs:**
- Pro: Zero runtime overhead, enforced by compiler, simplest to understand
- Pro: No type duplication, no manual field mapping
- Con: Requires data to outlive the reference (lifetime constraints)

**References:**
- [References and Borrowing - The Rust Book](https://doc.rust-lang.org/book/ch04-02-references-and-borrowing.html) — foundational Rust ownership concepts

#### Pattern: Newtype Wrapper

**How it works:** Create `pub struct BacklogView<'a>(&'a BacklogFile)` with methods that expose only desired fields.

**When to use:** When you want to explicitly hide certain fields at the type level, not just via immutability.

**Tradeoffs:**
- Pro: Explicit control over exposed fields, zero runtime cost
- Con: Requires forwarding methods manually (boilerplate)
- Con: Overkill for hiding a single `u32` that readers can trivially ignore

**References:**
- [Newtype - Rust Design Patterns](https://rust-unofficial.github.io/patterns/patterns/behavioural/newtype.html) — official pattern documentation
- [Item 6: Embrace the newtype pattern - Effective Rust](https://www.lurklurk.org/effective-rust/newtype.html) — best practices

### Standards & Best Practices

- Rust's ownership model makes `&T` the standard for read-only views — the borrow checker enforces immutability at compile time with zero runtime cost.
- Projection types (separate structs with a subset of fields) are an anti-pattern when the "hidden" fields carry no side effects and readers can simply ignore them.
- The `Deref` trait approach was considered but rejected — it hides intent through implicit coercion and is generally considered an [anti-pattern for non-smart-pointer types](https://rust-unofficial.github.io/patterns/anti_patterns/deref.html).

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Lifetime propagation | Switching from owned `BacklogSnapshot` to `&BacklogFile` could introduce lifetime annotations | In this codebase, the snapshot is cloned and sent through a channel as an owned value — no lifetime changes needed |
| Loss of type distinction | Using `BacklogFile` everywhere blurs "source of truth" vs "read-only copy" | Semantic distinction maintained by passing `&BacklogFile` (immutable ref) to consumers; coordinator clones before sending through channel |

### Key Learnings

- The PRD's proposed approach (eliminate `BacklogSnapshot`, pass `&BacklogFile`) is the idiomatic Rust solution
- No need for newtype wrappers, traits, or other abstractions — a simple `&BacklogFile` reference is sufficient
- The existing channel-based architecture (coordinator clones and sends owned `BacklogFile`) means no lifetime annotation changes are needed

---

## Internal Research

### Existing Codebase State

Two types represent backlog state:
- **`BacklogFile`** (types.rs:231-240): `schema_version: u32`, `items: Vec<BacklogItem>`, `next_item_id: u32` — derives `Serialize, Deserialize, Clone, Debug, PartialEq`
- **`BacklogSnapshot`** (types.rs:265-268): `schema_version: u32`, `items: Vec<BacklogItem>` — derives `Serialize, Deserialize, Clone, Debug, PartialEq`

The snapshot is created by `handle_get_snapshot()` (coordinator.rs:354-358) which copies `items` and `schema_version`, omitting `next_item_id`.

**Relevant files/modules:**
- `orchestrator/src/types.rs` — defines both `BacklogFile` (lines 231-240) and `BacklogSnapshot` (lines 265-268)
- `orchestrator/src/coordinator.rs` — `CoordinatorCommand::GetSnapshot` (lines 14-16), `handle_get_snapshot()` (lines 354-358), `CoordinatorHandle::get_snapshot()` (lines 95-99)
- `orchestrator/src/scheduler.rs` — `select_actions()` (line 145), `advance_to_next_active_target()` (line 462), `select_targeted_actions()` (line 839) — all accept `&BacklogSnapshot`
- `orchestrator/src/filter.rs` — `apply_filter()` (lines 144-156) takes `&BacklogSnapshot`, returns `BacklogSnapshot`
- `orchestrator/src/main.rs` — creates temporary `BacklogSnapshot` (lines 343-346) for filter testing

**Existing patterns in use:**
- Pure functions taking `&BacklogSnapshot` — scheduler functions are pure, taking immutable references
- Helper function pattern — test files define local `make_snapshot()` helpers for construction
- Channel-based actor model — coordinator clones data before sending through `oneshot` channels

### Reusable Components

- `BacklogFile` struct already has all derives except `Default` — needs `Default` added for test construction convenience
- `matches_item()` function in filter.rs is independent and unaffected by this change
- Test helper pattern (`make_snapshot()`) can be trivially adapted to `make_backlog_file()` or similar

### Constraints from Existing Code

- `BacklogFile` lacks `Default` derive — must be added for `..Default::default()` pattern in tests (PRD criterion)
- `next_item_id` has `#[serde(default)]` — backward compatible with YAML fixtures missing the field
- `apply_filter()` constructs a new struct from filtered items — must carry forward `next_item_id` and `schema_version`
- Channel sends owned values — coordinator clones `BacklogFile` before sending, so consumers get an owned copy (no lifetime complications)

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| `BacklogFile` can be used directly everywhere | Confirmed — all necessary derives present except `Default` | Must add `Default` derive to `BacklogFile` before test construction sites work with `..Default::default()` |
| All usage sites identified | Confirmed — PRD correctly lists coordinator, scheduler, filter, main, and test files | No additional sites found |
| Change is mechanical | Confirmed — compiler will guide all changes since `BacklogSnapshot` will cease to exist | Low risk of missing any references |

No significant concerns. The PRD accurately describes the current state and the proposed change.

---

## Critical Areas

### `Default` derive addition

**Why it's critical:** The PRD mentions using `..Default::default()` in test construction, but `BacklogFile` currently lacks a `Default` derive.

**Why it's easy to miss:** The PRD's success criteria mention updating test sites to use `..Default::default()` but don't explicitly list adding `Default` as a prerequisite.

**What to watch for:** Add `#[derive(Default)]` to `BacklogFile` in the same change. Verify that the default values (0 for `u32`, empty `Vec` for `items`) are sensible.

### `apply_filter()` field carry-forward

**Why it's critical:** The filter function constructs a new struct from filtered items. When switching to `BacklogFile`, it must now include `next_item_id`.

**Why it's easy to miss:** The current `BacklogSnapshot` has only 2 fields, so the mapping is simple. `BacklogFile` has 3, and forgetting to carry forward `next_item_id` would cause a compile error (good — the compiler catches it).

**What to watch for:** Ensure `next_item_id` is copied from the input `BacklogFile`, not defaulted to 0.

---

## Deep Dives

No deep dives needed — the change is well-understood from initial research.

---

## Synthesis

### Open Questions

None — all questions resolved through research. The PRD's approach is validated.

### Recommended Approaches

#### Elimination Strategy

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Direct elimination (pass `&BacklogFile`) | Idiomatic Rust, zero overhead, compiler-enforced safety, no type duplication | Exposes `next_item_id` to readers (harmless) | The extra field has no side effects — this is our case |
| Newtype wrapper (`BacklogView<'a>(&'a BacklogFile)`) | Hides `next_item_id` at type level | Boilerplate for method forwarding, over-engineered for one `u32` | When hidden fields carry dangerous semantics — not our case |

**Initial recommendation:** Direct elimination. The PRD's proposed approach is correct. Pass `&BacklogFile` to scheduler/filter. The extra `next_item_id` field is a single `u32` that readers can trivially ignore, and immutability is enforced by the borrow checker.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [References and Borrowing](https://doc.rust-lang.org/book/ch04-02-references-and-borrowing.html) | Docs | Foundational Rust ownership concepts validating the `&T` approach |
| [Newtype Pattern](https://rust-unofficial.github.io/patterns/patterns/behavioural/newtype.html) | Docs | Alternative approach (rejected as over-engineered for this case) |
| [Deref Anti-pattern](https://rust-unofficial.github.io/patterns/anti_patterns/deref.html) | Docs | Why Deref-based approach was correctly excluded |

---

## Assumptions

- **Light mode sufficient** — This is a small, low-complexity refactoring with clear direction from the PRD. No heavy external research needed.
- **No Product Vision constraint** — No `PRODUCT_VISION.md` found in the project root, so no product-level constraints to consider.

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-13 | Initial internal codebase research | Confirmed all PRD assumptions; found `Default` derive gap |
| 2026-02-13 | External patterns research | Confirmed `&T` approach is idiomatic Rust; alternatives rejected as over-engineered |
| 2026-02-13 | PRD analysis | No significant concerns; PRD is accurate and well-scoped |
