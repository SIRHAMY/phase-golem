# Tech Research: Add --target Priority List and --only Flag for Orchestrator Run

**ID:** WRK-030
**Status:** Complete
**Created:** 2026-02-12
**PRD:** ./WRK-030_add-target-priority-list-and-only-flag-for-orchestrator-run_PRD.md
**Mode:** Medium

## Overview

Researching how to add multi-target priority lists (`--target A --target B`) and attribute-based filtering (`--only key=value`) to the orchestrator's `run` command. Key questions: how do established CLI tools handle repeated flags and `key=value` filtering, what clap patterns apply, and how does the existing codebase constrain the implementation?

## Research Questions

- [x] How do popular CLI tools handle multi-target selection with repeated flags?
- [x] What are the conventions for `key=value` attribute-based filtering in CLI tools?
- [x] What clap patterns work for `Vec<String>` repeated args and `conflicts_with`?
- [x] How does the existing scheduler/runner architecture accommodate target iteration state?
- [x] What types and fields are available for filtering on `BacklogItem`?
- [x] Where are the integration points in the codebase?

---

## External Research

### Landscape Overview

Multi-target selection and attribute-based filtering are well-established patterns in CLI tools. Major tools (Cargo, Terraform, kubectl, Docker, pytest) each solve different facets of this problem. The core tension is between **simplicity** (one flag that "just works") and **expressiveness** (boolean logic, negation, composability). Most successful tools start simple and add expressiveness over time — which aligns with the MVP approach in the PRD.

For this use case — ordered targets processed sequentially plus attribute filtering on a backlog — the closest analogues are:
- **Terraform's `-target`** for repeated flag multi-target selection
- **Cargo's `-p`/`--package`** for multi-target with ordering
- **kubectl's `-l`/`--selector`** and **Docker's `--filter`** for `key=value` filtering

### Common Patterns & Approaches

#### Pattern: Repeated Flag Multi-Target Selection

**How it works:** Users specify a flag multiple times to build an ordered list. Each occurrence appends one value. The tool collects all values into an ordered `Vec`.

**When to use:** When users need to explicitly enumerate a small set (2-10) of targets and order matters.

**Tradeoffs:**
- Pro: Explicit — user knows exactly what will be processed
- Pro: Order is preserved in the argument list
- Pro: Backward compatible when changing from `Option<String>` to `Vec<String>`
- Con: Verbose for large target sets (10+ gets unwieldy)
- Con: Typo risk increases; validation at startup is essential

**Common technologies:** Terraform `-target`, Cargo `-p`, Docker Compose positional args

**References:**
- [Cargo build command](https://doc.rust-lang.org/cargo/commands/cargo-build.html) — multi-package selection via `-p`
- [Terraform Target Flag](https://spacelift.io/blog/terraform-target) — repeated `-target` for multi-resource targeting
- [Terraform Resource Targeting Tutorial](https://developer.hashicorp.com/terraform/tutorials/state/resource-targeting) — official targeting docs

#### Pattern: Key=Value Attribute-Based Filtering

**How it works:** Users supply filter criteria as `key=value` pairs. The tool parses key and value, validates both, and filters items before processing.

**When to use:** When users want to process a dynamically-determined subset based on item properties rather than explicit IDs.

**Tradeoffs:**
- Pro: Powerful for ad-hoc querying without knowing specific IDs
- Pro: Scales well — works the same on 5 items or 500
- Pro: `key=value` is universally understood syntax
- Con: Combining logic (AND/OR) needs careful design upfront
- Con: Case sensitivity decisions affect UX
- Con: Adding new filterable fields requires code changes

**Common technologies:** kubectl `-l` selectors, Docker `--filter`, pytest `-k`

**References:**
- [Kubernetes Labels and Selectors](https://kubernetes.io/docs/concepts/overview/working-with-objects/labels/) — canonical `key=value` filtering reference
- [Docker CLI Filter Documentation](https://docs.docker.com/engine/cli/filter/) — `--filter key=value` with AND/OR semantics
- [pytest -k filter options](https://pytest-with-eric.com/introduction/pytest-k-options/) — attribute-based test filtering

#### Pattern: Clap Derive with Vec + conflicts_with

**How it works:** In Rust's clap crate, a `Vec<String>` field automatically enables repeated flag occurrences. Mutual exclusivity uses `conflicts_with` attributes.

**When to use:** This is the standard Rust/clap pattern for exactly this scenario.

**Tradeoffs:**
- Pro: First-class clap support, well-documented
- Pro: Compile-time checked via derive macros
- Pro: Free error messages from clap
- Con: Custom validation (duplicate detection, format checking) must be done after parsing
- Con: `Vec<String>` defaults to empty vec (not `None`), which is fine for this use case

**References:**
- [Clap derive tutorial](https://docs.rs/clap/latest/clap/_derive/_tutorial/index.html) — official tutorial
- [Clap ArgGroup docs](https://docs.rs/clap/latest/clap/struct.ArgGroup.html) — mutual exclusivity
- [Repeat same argument - Clap book](https://rust.code-maven.com/clap/repeat-the-same-argument-several-times) — practical guide
- [Vec\<String\> in clap - discussion](https://github.com/clap-rs/clap/discussions/3788) — maintainer guidance

#### Pattern: Index Cursor for Ordered Queue Processing

**How it works:** A task processor maintains an ordered list and a `current_index` cursor. It processes items front-to-back. When an item reaches a terminal state (completed/blocked), the processor applies a policy: halt, skip to next, or fallback.

**When to use:** When you have an ordered list of work items and need deterministic behavior at each state transition.

**Tradeoffs:**
- Pro: Clear semantics — users understand "process in this order"
- Pro: Simple state machine: `current_index` + `item_state` -> `next_action`
- Pro: Naturally maps to `Vec<TargetId>` with `current_target_index: usize`
- Con: Must handle edge cases: all targets already done, target blocked mid-run

**References:**
- [Azure Priority Queue Pattern](https://learn.microsoft.com/en-us/azure/architecture/patterns/priority-queue) — ordered task processing architecture

### Technologies & Tools

| Technology | Purpose | Pros | Cons |
|---|---|---|---|
| `clap` `Vec<String>` + `conflicts_with` | Multi-target + mutual exclusivity | First-class derive support; free error messages; order preserved | Custom validation needed post-parse |
| `clap` `ArgGroup` | Alternative mutual exclusivity | Declarative | Derive support limited; `conflicts_with` is simpler |
| `FilterField` enum | Filter key parsing | Compiler exhaustiveness checking | Must update when adding fields |

### Standards & Best Practices

1. **Fail fast at startup** — Validate all target IDs and filter syntax before entering the scheduler loop. All surveyed tools (Terraform, kubectl, Docker) follow this pattern.
2. **Preserve existing single-target behavior** — `Vec<String>` with one element must behave identically to old `Option<String>`.
3. **Use standard `key=value` syntax** — Universal, no custom syntax needed.
4. **Filter on snapshot, not live state** — kubectl, Terraform, Docker all filter on point-in-time state. Consistent with existing scheduler design.
5. **Ordered processing via index cursor** — Simple `current_target_index` that advances sequentially.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Using `Option<Vec<String>>` instead of `Vec<String>` | Adds unnecessary Option layer; empty Vec is sufficient | Use `Vec<String>`, check `is_empty()` |
| Clap doesn't deduplicate repeated values | `--target WRK-005 --target WRK-005` produces duplicates | Validate with `HashSet` at startup |
| Case sensitivity mismatch in filter values | `impact=HIGH` silently misses `High` | Convert both to lowercase for enum fields |
| `conflicts_with` must reference field name, not flag name | Incorrect reference silently ignored | Keep field names and flag names consistent |
| String matching for filter field names | Missing match arm = silent filter failure | Parse into `FilterField` enum for exhaustiveness |
| Putting target iteration state in scheduler function | Breaks purity and testability | Keep cursor in runner/`SchedulerState`, scheduler stays pure |

### Key Learnings

- The `Vec<String>` + `conflicts_with` pattern is exactly the right clap approach — well-documented, backward compatible, order-preserving
- `key=value` is the universal CLI filtering syntax; start with single filter, extend to multiple with AND logic later
- Target cursor state (mutable) belongs in the runner loop, not the pure scheduler function — this aligns with the existing architecture

---

## Internal Research

### Existing Codebase State

The orchestrator is a Rust async application built around an actor-based coordinator pattern with clean separation between:
- **Pure scheduling logic** (`select_actions()`, `select_targeted_actions()`) — deterministic, no I/O, trivially testable
- **Stateful runner loop** (`run_scheduler()`) — manages `SchedulerState`, runs the main loop, calls pure functions
- **Coordinator actor** (`coordinator.rs`) — single-threaded owner of backlog state, communicates via `mpsc` channels
- **CLI entry point** (`main.rs`) — `clap` derive-based argument parsing

The scheduler loop works as: snapshot -> select actions -> execute promotions -> spawn phase tasks -> await completion -> process results -> batch commit -> repeat until halt condition.

**Relevant files/modules:**

| File | Purpose | Relevance |
|------|---------|-----------|
| `src/scheduler.rs` | Core scheduling logic, runner loop, `RunParams`, `HaltReason`, `SchedulerState` | Primary integration point — target/filter changes here |
| `src/main.rs` | CLI args (clap), `handle_run()`, validation | Add `--only` arg, multi-target, startup validation |
| `src/types.rs` | `BacklogItem`, `ItemStatus`, `SizeLevel`, `DimensionLevel` | Defines filterable fields and their types |
| `src/coordinator.rs` | Actor owning backlog state | No changes needed — provides snapshots |
| `src/config.rs` | Config loading, `validate()` | Pattern for error accumulation validation |
| `src/preflight.rs` | Preflight validation | Pattern for structured validation errors |
| `src/lib.rs` | Module declarations | Add `pub mod filter;` |
| `src/backlog.rs` | Backlog CRUD operations | No changes needed |
| `tests/scheduler_test.rs` | Scheduler unit + integration tests (1655 lines) | Add multi-target and filter tests |
| `Cargo.toml` | Dependencies | No new dependencies needed |

**Existing patterns in use:**

1. **Pure function + state management separation** — `select_actions()` and `select_targeted_actions()` are pure; `run_scheduler()` manages mutable state. PRD requires maintaining this.
2. **Snapshot-based design** — Fresh `BacklogSnapshot` each loop iteration. Filter naturally fits here — filter `snapshot.items` before passing to `select_actions()`.
3. **Error accumulation** — `config::validate()` and `preflight::run_preflight()` accumulate all errors into `Vec` before returning.
4. **Case-insensitive parsing** — `parse_size_level()` and `parse_dimension_level()` use `s.to_lowercase().as_str()` match with descriptive `Result<T, String>` errors.
5. **Display impls** — `SizeLevel` and `DimensionLevel` have lowercase `Display`. `ItemStatus` uses `format!("{:?}", status).to_lowercase()`.
6. **clap derive** — `#[arg(long)]` with `Option<T>` for optional flags.
7. **Separate test files** — Tests in `tests/scheduler_test.rs`, not inline `#[cfg(test)]`.
8. **HaltReason-based termination** — Scheduler always returns `RunSummary` with `HaltReason` variant.

### Reusable Components

- **`parse_size_level()` / `parse_dimension_level()`** — Adapt for `--only` filter value parsing for `size`, `complexity`, `risk`, `impact` fields
- **`make_item()` family of test helpers** — Extend to set tags/pipeline_type for filter tests
- **`select_targeted_actions()`** — Existing single-target implementation; multi-target just calls it with `targets[current_index]`
- **`SchedulerState.items_completed` / `items_blocked`** — Already track completed/blocked items; extend target completion check to use current index
- **All `BacklogItem` filterable fields** — Public, directly accessible for filter matching

### Constraints from Existing Code

- **`RunParams.target`** is currently `Option<String>` (scheduler.rs:46-51). Changing to `Vec<String>` is backward compatible but all call sites using `if let Some(ref target_id) = params.target` must be updated.
- **`HaltReason` derives `PartialEq`** — New variants are straightforward to add; tests compare with `==`.
- **`select_targeted_actions()` takes `target_id: &str`** — Keeps its current signature. Runner passes `targets[current_index]`.
- **Snapshot re-fetched each loop iteration** — Filter must be applied each cycle on the fresh snapshot.
- **`BacklogItem` implements `Clone`** — Filtering can produce a new `BacklogSnapshot` with filtered items.
- **`serde(rename_all = "snake_case")` on `ItemStatus`** — YAML values are `new`, `scoping`, `ready`, `in_progress`, `done`, `blocked`. Filter must match these representations case-insensitively.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| `--target` changes from `Option<String>` to multi-value | `Vec<String>` is the correct clap type; empty Vec = no targets specified. Backward compatible. | Straightforward. All `Some(ref target_id)` checks become `!targets.is_empty()` + index-based access. |
| Filter operates on snapshot before scheduler | Snapshot is re-fetched each cycle, so filter runs each cycle | Correct. Filter produces a narrowed `BacklogSnapshot`. Must be efficient but O(n) per cycle is fine per NFR. |
| `select_targeted_actions()` stays pure, target iteration in runner | Confirmed: this is the right split. `select_targeted_actions()` already takes `&str`, no changes needed to its signature. | Runner tracks `current_target_index` in `SchedulerState` or a new field on `RunParams`. |
| New halt reasons `FilterExhausted`, `NoMatchingItems` | HaltReason enum already has 6 variants, adding 2 is trivial. All derives (`Debug`, `PartialEq`) work automatically. | No concerns. |
| Filter field validation uses a closed set of 7 field names | All 7 fields exist on `BacklogItem`. Existing `parse_size_level()` / `parse_dimension_level()` provide patterns for value validation. | Use a `FilterField` enum parsed from the key string, matching existing patterns. |
| Tags field is `Vec<String>`, unused in scheduling | Confirmed: `tags: Vec<String>` exists but has no scheduling logic. Tag filter = "item tags contain value". | Simple `contains()` check. PRD correctly specifies case-sensitive for tags. |
| `pipeline_type` is `Option<String>` | Confirmed. Filter match = `Some(v) if v == filter_value`. PRD correctly specifies case-sensitive. | Must handle `None` case (item has no pipeline_type = no match). |

No significant concerns. The PRD assumptions align well with the codebase reality.

---

## Critical Areas

### Target State Transition Edge Cases

**Why it's critical:** The target iteration logic (advance on complete, halt on block, skip already-Done) has several edge cases that interact with the async scheduler loop.

**Why it's easy to miss:** The happy path (target completes, advance to next) is simple, but: what if a target transitions to Blocked mid-phase? What if multiple targets complete in the same cycle? What if a target that was "next in queue" becomes Done via a dependency before it's the active target?

**What to watch for:**
- Check target status against the **current** snapshot each cycle, not against a stale state
- Ensure the advance-to-next-target logic handles the case where the next target is already Done (should skip, not re-process)
- The existing single-target check at scheduler.rs:514-526 is the model — extend it carefully

### Filter Application Point in the Runner Loop

**Why it's critical:** The filter must be applied at exactly the right point in the scheduler loop — after fetching the snapshot but before calling `select_actions()`. Applying it too early or too late changes semantics.

**Why it's easy to miss:** The PRD says "filter applied once per scheduler cycle on the snapshot." But the runner loop has multiple steps. The filter must not interfere with the existing snapshot-based promotions or phase completion tracking.

**What to watch for:**
- Filter the snapshot items, then pass the filtered snapshot to `select_actions()`
- The coordinator's full snapshot is still needed for other purposes (e.g., checking total items for halt conditions)
- `FilterExhausted` must check all filtered items' statuses, not all backlog items

### ItemStatus Serialization Format Mismatch

**Why it's critical:** `ItemStatus` uses `serde(rename_all = "snake_case")`, so YAML stores `in_progress`, but `Debug` output is `InProgress`. The filter must match the YAML/serde representation since that's what users see.

**Why it's easy to miss:** If the filter parses `status=in_progress` but compares against `Debug` format (`InProgress`), it silently fails.

**What to watch for:**
- Parse filter value into `ItemStatus` enum using the serde representation (`in_progress`), not the Rust variant name
- Use `serde_plain::from_str()` or a custom parser matching the `snake_case` serde format
- Test with `in_progress` specifically since it's the only multi-word status variant

---

## Deep Dives

_No deep dives conducted — autonomous mode, medium research._

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| Should `current_target_index` live in `SchedulerState` or a new struct? | Affects where target iteration logic sits in the runner loop | `SchedulerState` (simple, one struct) vs. new `TargetQueue` struct (cleaner separation). Recommend: `SchedulerState` since it already tracks per-run mutable state. |
| How to parse `ItemStatus` filter values given `serde(rename_all = "snake_case")`? | `in_progress` must match, not `InProgress` | Use a custom `from_str` that matches snake_case format, or `serde_plain::from_str()`. Recommend: simple match on lowercase with snake_case entries. |
| Should filter produce a new `BacklogSnapshot` or modify items in-place? | Affects whether coordinator's full snapshot is still available for other checks | Produce a new filtered snapshot; keep original for halt condition checking. |

### Recommended Approaches

#### Multi-Target CLI Argument

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `Vec<String>` with `conflicts_with` | First-class clap; preserves order; backward compatible; free error messages | Custom validation after parse | Always — this is the standard pattern |
| `ArgGroup` with derive | Declarative exclusivity | Limited derive support; more complex | Only if more than 2 mutually exclusive flag groups |

**Initial recommendation:** `Vec<String>` with `conflicts_with = "only"`. This is exactly what Cargo and Terraform use.

#### Filter Implementation

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| `FilterField` enum + match | Exhaustive; compiler catches missing fields | Must update enum for new fields | ✓ Recommended — aligns with closed set of 7 fields |
| String-based field matching | No enum needed | Silent failures on typos; no exhaustiveness | Rapid prototyping only |

**Initial recommendation:** Parse filter key into a `FilterField` enum. Parse value using existing `parse_size_level()` / `parse_dimension_level()` patterns for enum fields, direct string comparison for tags/pipeline_type.

#### Target Iteration State

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Add `current_target_index` to `SchedulerState` | Simple; SchedulerState already tracks per-run mutable state | Slightly overloads SchedulerState's purpose | ✓ Recommended — minimal change |
| New `TargetQueue` struct | Clean separation of concerns | Extra type and plumbing | If target logic becomes complex |

**Initial recommendation:** Add `current_target_index: usize` to `SchedulerState`. It already tracks `phases_executed`, `items_completed`, etc. — target index is the same kind of per-run state.

#### Filter Application Architecture

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Filter snapshot before `select_actions()` | Clean; scheduler sees only matching items | Must keep original snapshot for halt checks | ✓ Recommended — matches PRD's snapshot-based design |
| Pass filter into `select_actions()` | Everything in one place | Breaks purity; adds filter concern to scheduler | Not recommended |

**Initial recommendation:** Filter in the runner loop. Create a filtered snapshot, pass to `select_actions()`. Keep the unfiltered snapshot for `FilterExhausted` halt condition checking.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Cargo build -p](https://doc.rust-lang.org/cargo/commands/cargo-build.html) | Docs | Multi-package selection pattern |
| [Terraform -target](https://developer.hashicorp.com/terraform/tutorials/state/resource-targeting) | Docs | Repeated `-target` flag |
| [Kubernetes Labels and Selectors](https://kubernetes.io/docs/concepts/overview/working-with-objects/labels/) | Docs | Canonical `key=value` filtering reference |
| [Docker CLI Filter](https://docs.docker.com/engine/cli/filter/) | Docs | `--filter key=value` AND/OR semantics |
| [Clap derive tutorial](https://docs.rs/clap/latest/clap/_derive/_tutorial/index.html) | Docs | Vec, Option, repeated args |
| [Clap ArgGroup](https://docs.rs/clap/latest/clap/struct.ArgGroup.html) | Docs | Mutual exclusivity |
| [Clap Vec discussion](https://github.com/clap-rs/clap/discussions/3788) | Discussion | Vec population guidance from maintainers |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-12 | External research: CLI multi-target and filter patterns | Identified 4 patterns (repeated flag, key=value filter, clap Vec+conflicts_with, index cursor queue). Strong convergence on approach. |
| 2026-02-12 | Internal research: Codebase exploration | Mapped all integration points. Confirmed PRD assumptions align with codebase. Identified 3 critical areas. |
| 2026-02-12 | PRD analysis | No significant concerns. PRD is well-aligned with both external patterns and internal architecture. |
