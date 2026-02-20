# Tech Research: Add --auto-advance flag for multi-target runs

**ID:** WRK-054
**Status:** Complete
**Created:** 2026-02-19
**PRD:** ./WRK-054_feature_PRD.md
**Mode:** Medium

## Overview

Researching patterns for skip-on-block behavior in sequential task orchestrators, and verifying how the existing codebase supports adding an `--auto-advance` flag to the `run` subcommand. The flag makes the scheduler skip blocked targets and continue to the next instead of halting. Key questions: what patterns exist for this in the industry, what exit code conventions apply, and how does the existing code structure accommodate this change.

## Research Questions

- [x] What patterns exist for skip-on-failure in sequential batch/task orchestrators?
- [x] What are the best practices for exit codes when a batch run has partial success?
- [x] What does the existing scheduler loop look like, and where does auto-advance logic fit?
- [x] What existing state tracking can be reused (circuit breaker, items_blocked, items_completed)?
- [x] What conventions does the codebase follow for adding CLI flags and threading them?

---

## External Research

### Landscape Overview

The problem of "continue processing remaining items when one blocks/fails" is common across batch processing, CI/CD pipelines, workflow engines, and build systems. The core tension: how to preserve signal about what failed, allow maximum forward progress, and return an exit code that automation can act on unambiguously.

The `--auto-advance` use case is a sequential (not parallel) task list where each item has discrete blocking states and intermediate state must be durably committed before moving on. This is closest to Spring Batch's skip-with-checkpoint model.

### Common Patterns & Approaches

#### Pattern: Fail-Fast (Default Halt)

**How it works:** First item failure stops the entire run. No further items processed.

**When to use:** When items have hard dependencies, or any failure invalidates subsequent work.

**Tradeoffs:**
- Pro: Simple semantics, no partial state
- Con: One stuck item prevents progress on unrelated items

**Common technologies:** GNU Make (default), go-task (default), most CLI tools

**References:**
- [GNU Make Parallel execution](https://www.gnu.org/software/make/manual/html_node/Parallel.html)

#### Pattern: Continue-on-Error / Keep-Going

**How it works:** Each item attempted regardless of prior failures. Failures accumulated. Exit code reflects aggregate outcome. This is the pattern `--auto-advance` implements.

**When to use:** When items are independent and maximum throughput matters more than early stopping.

**Tradeoffs:**
- Pro: Maximizes forward progress; unblocked items complete
- Pro: Full batch summary in a single run
- Con: Circuit breakers and retry counters must be explicitly reset between items
- Con: Exit code semantics for partial success require careful design

**Common technologies:** GNU Make `--keep-going`/`-k`, GitHub Actions `continue-on-error`, Argo Workflows `continueOn: failed: true`, Ansible `ignore_errors`

**References:**
- [go-task --keep-going feature request](https://github.com/go-task/task/issues/1318)
- [Argo Workflows continueOn failed issue](https://github.com/argoproj/argo-workflows/issues/11163)
- [GitHub Actions continue-on-error discussion](https://github.com/orgs/community/discussions/77915)

#### Pattern: Skip-with-Checkpoint / Durable State Before Advancing

**How it works:** Before skipping a blocked item, its terminal state is committed durably. Only after commit succeeds does the orchestrator advance. Makes partial runs recoverable.

**When to use:** When audit trails or mid-run crash recovery matter.

**Tradeoffs:**
- Pro: Durable intermediate state enables recovery and debugging
- Con: Adds latency between items (checkpoint I/O)
- Con: Checkpoint failure becomes its own failure mode

**Common technologies:** Spring Batch (step-level checkpointing), AWS Step Functions, Apache Airflow

**References:**
- [Spring Batch SkipPolicy](https://docs.spring.vmware.com/spring-batch/docs/5.0.7/api/org/springframework/batch/core/step/skip/SkipPolicy.html)
- [Spring Batch skip logic](https://www.baeldung.com/spring-batch-skip-logic)

#### Pattern: Threshold-Based Halt

**How it works:** Continue past failures up to a configured threshold (count or percentage). Prevents runaway processing when failures indicate systemic problems.

**When to use:** Large batches where some failures are expected but widespread failure is not.

**Tradeoffs:**
- Pro: Limits blast radius of systemic problems
- Con: Threshold hard to tune; adds configuration complexity

**Common technologies:** GNU Parallel `--halt`, Ansible `max_fail_percentage`, Kubernetes Jobs `backoffLimit`

**References:**
- [GNU Parallel tutorial](https://www.gnu.org/software/parallel/parallel_tutorial.html)

### Standards & Best Practices

**Exit code conventions for partial success:**

| Tool | 0 | Non-zero codes | Approach |
|------|---|----------------|----------|
| pytest | All passed | 1=some failed, 2=interrupted, 3=internal error | Distinct codes per failure class |
| cargo-nextest | OK | 4=no tests, 100=test failures, 101=build failed | Distinct codes |
| Ansible | All success | 2=task failures, 4=unreachable hosts | Partial success encoded |
| GNU Parallel | All succeeded | 1-253=count of failed jobs | Count encoding (breaks CI) |
| Robocopy | Success variants 0-7 | 8+=errors | Bitmask (breaks CI) |

**Industry consensus:** The [Better CLI guide](https://bettercli.org/design/exit-codes/) recommends sticking to binary 0/non-zero and using stderr for nuanced reporting. The PRD's chosen semantics (exit 0 if at least one target completed, non-zero if all blocked) is the most CI-compatible option.

### Common Pitfalls

| Pitfall | Why It's a Problem | How to Avoid |
|---------|-------------------|--------------|
| Circuit breaker state bleeding across targets | Failures from target A accumulate and trip breaker for unrelated target B | Explicitly reset counter when auto-advancing |
| Committing state after the fact | Crash between targets leaves previous target's state uncommitted | Commit blocked state to git before advancing (PRD requires this) |
| `continueOn` combined with retries | Known bugs in Argo where skip + retry interact incorrectly | Exhaust retries at item level before making skip decision |
| `ignore_error` hiding real failures | Skipped items don't contribute to exit code, masking failures | Blocked items must still affect final exit code |
| Ambiguous exit codes | exit 2 means different things in different tools | Use simple 0/non-zero; report details via stdout/stderr |

### Key Learnings

- The continue-on-error + checkpoint pattern (Patterns 2+3 combined) is exactly what the PRD describes
- Every implementation that shares circuit breaker state across independent items treats it as a bug
- Simple 0/non-zero exit codes are more robust than encoding counts or bitmasks
- The opt-in flag pattern (`--keep-going`, `--auto-advance`) is the industry standard for backward-compatible skip behavior

---

## Internal Research

### Existing Codebase State

**Relevant files/modules:**

- `src/scheduler.rs` — Main scheduler loop, RunParams, SchedulerState, HaltReason, block detection, circuit breaker, target advancement
- `src/main.rs` — CLI argument definitions (clap), handle_run function that validates and constructs RunParams
- `src/filter.rs` — FilterCriterion implementation, shows pattern for optional feature flags
- `tests/scheduler_test.rs` — Comprehensive scheduler tests including multi-target scenarios

**Existing patterns in use:**

- **CLI flag threading:** Parse from clap → validate at startup → construct RunParams → pass to scheduler
- **Block detection (lines 608-631):** Checks `state.items_blocked.contains(target_id)`, drains join set, commits, returns `HaltReason::TargetBlocked`
- **Target advancement (lines 468-510):** Pure function `advance_to_next_active_target()` skips Done/Blocked/archived targets based on snapshot
- **Circuit breaker (line 572):** `state.is_circuit_breaker_tripped()` checks `consecutive_exhaustions >= CIRCUIT_BREAKER_THRESHOLD` at top of loop
- **Logging:** `log_info!()`, `log_warn!()` macros with `[target]` prefix and position context `(index+1 / total.len())`

**Key structs:**

- `RunParams` (lines 49-59): `targets: Vec<String>`, `filter: Option<FilterCriterion>`, `cap: u32`, `root: PathBuf`, `config_base: PathBuf`
- `SchedulerState` (lines 1755-1774): `current_target_index`, `items_blocked`, `items_completed`, `consecutive_exhaustions`, `follow_ups_created`, `items_merged`, `phases_executed`, `cap`
- `HaltReason` (lines 37-47): `AllDoneOrBlocked`, `CapReached`, `CircuitBreakerTripped`, `ShutdownRequested`, `TargetCompleted`, `TargetBlocked`, `FilterExhausted`, `NoMatchingItems`

### Reusable Components

- `items_blocked: Vec<String>` in SchedulerState — already tracks runtime-blocked targets
- `items_completed: Vec<String>` — already tracks completed targets
- `consecutive_exhaustions` field — already exists, just needs reset logic
- `drain_join_set()` — already used at block detection site
- `coordinator.batch_commit()` — already called at block site for durability
- `advance_to_next_active_target()` — pure function, already handles skipping Done/Blocked/archived
- `build_summary()` — constructs RunSummary from state + halt reason

### Constraints from Existing Code

- **Circuit breaker check order:** The circuit breaker fires at line 572, before block detection at line 609. The reset must happen in the auto-advance branch on the same iteration, before the next iteration's circuit breaker check.
- **`max_wip=1` in target mode:** No in-flight tasks when block is detected, so no complex drain logic needed.
- **`advance_to_next_active_target()` handles pre-existing Blocked items only** (checks snapshot status). Runtime blocks (items blocked during this run) are tracked in `state.items_blocked`. Auto-advance operates on runtime blocks only.
- **HaltReason is a simple enum** — no payload. The run summary provides details via `items_completed` and `items_blocked` lists.

---

## PRD Concerns

| PRD Assumption | Research Finding | Implication |
|----------------|------------------|-------------|
| Simple boolean flag on RunParams | Matches industry pattern (GNU Make `-k`, Argo `failFast: false`). No concern. | Straightforward implementation |
| Exit 0 for partial success, non-zero for all blocked | Aligns with Better CLI guide and pytest/nextest conventions. Well-supported. | PRD's choice is correct and defensible |
| Circuit breaker reset when auto-advancing | Every industry implementation treats shared breaker state across independent items as a bug. PRD is correct. | Must verify reset timing relative to next iteration's check |
| Git commit before advancing | Matches Spring Batch checkpoint-before-skip model. Critical correctness property. | Already done at the existing block site (`coordinator.batch_commit()` at line 629) |
| `advance_to_next_active_target()` not modified | PRD correctly distinguishes pre-existing Blocked (handled by this function) from runtime Blocked (handled by `items_blocked` tracking). No concern. | Clean separation of concerns |

**No significant divergences found.** The PRD's design aligns well with industry patterns and the existing codebase structure.

---

## Critical Areas

### Circuit Breaker Reset Timing

**Why it's critical:** If the reset happens after the loop iterates (rather than immediately in the auto-advance branch), the next iteration's circuit breaker check at line 572 will fire before the reset takes effect.

**Why it's easy to miss:** The circuit breaker check and the block detection are separated by ~35 lines of code. The temporal ordering isn't obvious from reading either site in isolation.

**What to watch for:** The reset (`state.consecutive_exhaustions = 0`) must happen inside the auto-advance branch at the block detection site (lines 608-631), not at the top of the next loop iteration. A test with two consecutively blocked targets under `--auto-advance` should verify the circuit breaker does not trip.

### Exit Code When All Targets Blocked

**Why it's critical:** The run terminates via `HaltReason::TargetCompleted` (target list exhausted) even when all targets were blocked. Without distinct messaging, the user may misinterpret this as success.

**Why it's easy to miss:** `TargetCompleted` is the natural halt reason when the cursor reaches the end of the target list, regardless of whether items completed or blocked.

**What to watch for:** The `build_summary()` function and any exit-code logic in `main.rs` must check whether `items_completed` is empty when `items_blocked` is non-empty. The PRD's "Should Have" criterion covers distinct messaging for this case.

---

## Deep Dives

No interactive deep dives (autonomous mode). Key areas verified through direct code inspection.

---

## Synthesis

### Open Questions

| Question | Why It Matters | Possible Answers |
|----------|----------------|------------------|
| Should duplicate target IDs be deduplicated? | User may see fewer summary entries than expected | Deduplicate at parse time (simple), or document the behavior |
| Should `items_blocked` in RunSummary be deduplicated? | Multiple code paths can push same ID | Use a HashSet or deduplicate before summary output |
| Should a distinct non-zero exit code (e.g., 2) distinguish "all blocked" from "execution error"? | Would allow scripts to distinguish failure types | Simple non-zero (1) is sufficient per Better CLI guide; can add later if needed |

### Recommended Approaches

#### Skip Mechanism

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Conditional branch at existing block site (lines 608-631) | Minimal code change, follows existing structure, single decision point | None significant | Always — this is the clear choice |

**Initial recommendation:** Add `if params.auto_advance { skip-and-continue } else { halt }` at the existing block detection site. This is where the decision already happens; adding the conditional is a ~15-line change.

#### Exit Code Strategy

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Binary 0/1 (PRD's choice) | CI-compatible, simple, matches Better CLI guide | Can't distinguish "all blocked" from other errors | Default for most uses |
| Multi-code (0/1/2) like pytest | Scriptable, distinguishes failure classes | More complex, must document | If automation needs to distinguish failure types |

**Initial recommendation:** Start with binary 0/1 per the PRD. The run summary stdout provides the details. A distinct exit code for "all blocked" can be added later as a follow-up if users need it.

#### Circuit Breaker Reset

| Approach | Pros | Cons | Best When |
|----------|------|------|-----------|
| Reset in auto-advance branch | Simple, correct timing, single point of change | None | Always — this is the only correct approach |

**Initial recommendation:** `state.consecutive_exhaustions = 0` inside the auto-advance conditional, immediately before incrementing the target index.

### Key References

| Reference | Type | Why It's Useful |
|-----------|------|-----------------|
| [Better CLI - Exit Codes](https://bettercli.org/design/exit-codes/) | Guide | Validates binary 0/1 exit code choice |
| [pytest exit codes](https://docs.pytest.org/en/stable/reference/exit-codes.html) | Docs | Example of well-designed multi-code convention |
| [cargo-nextest exit codes](https://docs.rs/nextest-metadata/latest/nextest_metadata/enum.NextestExitCode.html) | Docs | Rust-ecosystem precedent for distinct exit codes |
| [Spring Batch SkipPolicy](https://docs.spring.vmware.com/spring-batch/docs/5.0.7/api/org/springframework/batch/core/step/skip/SkipPolicy.html) | API | Reference for checkpoint-before-skip pattern |
| [Argo continueOn + retry issues](https://github.com/argoproj/argo-workflows/issues/11163) | Issue | Documents pitfalls of skip + retry interaction |
| [GNU Make --keep-going](https://www.gnu.org/software/make/manual/html_node/Parallel.html) | Docs | Classic keep-going pattern reference |

---

## Research Log

| Date | Activity | Outcome |
|------|----------|---------|
| 2026-02-19 | External research: skip-on-failure patterns, exit codes, batch orchestration | 4 patterns documented, exit code conventions surveyed, 6 pitfalls identified |
| 2026-02-19 | Internal research: scheduler loop, RunParams, SchedulerState, CLI conventions | All integration points mapped, no blockers found, implementation path clear |
| 2026-02-19 | PRD analysis against findings | No significant divergences; PRD aligns with industry patterns |
