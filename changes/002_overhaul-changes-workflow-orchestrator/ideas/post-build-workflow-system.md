# Post-Build Workflow System

## Overview

A system for maximizing AI agent utilization between human check-ins. Three components, each doing one thing:

1. **Changes Workflow** (existing) — the execution engine. Takes any work item through the software engineering lifecycle.
2. **Backlog** (new) — the work queue. Items at various stages of readiness, pushed forward by agents until ready to execute.
3. **Orchestration Script** (new) — the loop. Picks the next item, feeds it to the changes workflow, collects follow-ups, repeats.

## The Loop

```
                    ┌──────────────────────────────────┐
                    │                                  │
                    ▼                                  │
              ┌───────────┐                            │
              │  Backlog  │                            │
              │           │                            │
              │  Items at │                            │
              │  various  │                            │
              │  stages   │                            │
              └─────┬─────┘                            │
                    │                                  │
                    │ script picks next                │
                    ▼                                  │
              ┌───────────┐                            │
              │  Changes  │                            │
              │ Workflow  │                            │
              │           │                            │
              │ PRD →     │                            │
              │ Research →│                            │
              │ Design →  │                            │
              │ Spec →    │                            │
              │ Build     │                            │
              └─────┬─────┘                            │
                    │                                  │
                    │ build complete                   │
                    ▼                                  │
              ┌───────────┐                            │
              │  Review   │     follow-ups,            │
              │           │────  new items,  ──────────┘
              │  Triage   │     discoveries
              │  follow-  │
              │  ups      │
              └─────┬─────┘
                    │
                    ▼
              ┌───────────┐
              │ Work Log  │
              │ (append)  │
              └───────────┘
```

**When you're present:** You tell the script what to work on next. You plan the big stuff. You make decisions on items flagged `needs_human`.

**When you're away:** The script runs the same loop autonomously — picks the highest-priority ready item from the backlog, feeds it through the changes workflow, reviews, triages follow-ups back into the backlog, picks the next thing.

Same pipeline either way. The "autonomous" part isn't a separate mode — it's just the loop running without you.

## Core Principles

- **One pipeline**: Everything flows through the same changes workflow, whether it's a big feature or a small fix. The workflow scales to the task.
- **Pull model**: No notifications. You check in when ready and scan the state.
- **Issue-based**: Structured, parseable work items. Script makes decisions from metadata, agents make decisions from context.
- **Promotion pipeline**: Items earn detail as they mature. One-liner → idea file → changes workflow. Each step adds context only when warranted.
- **Low friction capture, easy promotion**: Most items start as a one-liner. Only graduate when they need it.

---

## Component 1: Backlog (`BACKLOG.yaml`)

The work queue. YAML file in the repo root. Every item is tracked here regardless of size or stage.

**YAML because** the script needs to reliably parse, filter, sort, and modify items. All three consumers handle it well:
- **Script**: Standard YAML parser, direct field access
- **Agent**: Naturally reads and produces YAML
- **Human**: Scannable with comment dividers, editable in any text editor

### Schema

```yaml
items:
  # ── Ready (can enter changes workflow) ──────────────────
  - id: fix-webhook-errors
    title: Fix error handling in webhook parser
    size: small
    status: ready
    risk: low
    created: 2026-02-10
    notes: Unhandled edge case when payload is missing required fields

  # ── Scoped (idea file complete, needs review or planning) ─
  - id: api-key-auth
    title: Refactor auth middleware to support API keys
    size: medium
    status: scoped
    risk: medium
    idea_file: _ideas/api-key-auth.md
    created: 2026-02-09

  # ── Researching (agent is actively investigating) ───────
  - id: rate-limiting
    title: Add rate limiting to public endpoints
    size: medium
    status: researching
    risk: low
    idea_file: _ideas/rate-limiting.md
    created: 2026-02-09

  # ── New (just captured, needs triage) ───────────────────
  - id: websocket-migration
    title: Migrate from polling to websockets
    size: large
    status: new
    risk: high
    created: 2026-02-08
    notes: Not urgent but would improve UX significantly

  # ── Blocked (needs human decision) ──────────────────────
  - id: auth-strategy
    title: "OAuth2 vs API keys: which direction?"
    size: medium
    status: blocked
    needs_human: true
    idea_file: _ideas/auth-strategy.md
    created: 2026-02-10
    notes: Affects API design broadly. Blocks api-key-auth.

  # ── In Progress (currently in changes workflow) ─────────
  - id: input-validation
    title: Add input validation to /create endpoint
    size: small
    status: in_progress
    phase: build          # prd | research | design | spec | build | review
    phase_attempts: 0     # retry counter, resets on phase advance
    risk: low
    created: 2026-02-08

  # ── Done ────────────────────────────────────────────────
  - id: queue-race-fix
    title: Fixed race condition in queue processor
    size: small
    status: done
    created: 2026-02-09
    done_date: 2026-02-10
```

### Fields

**Required:** `id`, `title`, `size`, `status`, `risk`, `created`

**Optional:** `idea_file`, `needs_human` (default false), `tags`, `done_date`, `notes`, `dependencies` (list of item IDs)

**In-progress fields:** `phase` (current changes workflow phase), `phase_attempts` (retry counter, resets on phase advance)

### Status Lifecycle

```
new → researching → scoped → ready → in_progress → done
                                 ↗
              (small items skip straight to ready)

Any status can also → blocked (needs human decision)
```

| Status | Meaning | Who acts |
|---|---|---|
| `new` | Just captured. Needs triage — is this worth pursuing? How big? | Script assigns to agent for research, or human triages |
| `researching` | Agent is investigating. Idea file being built. | Agent |
| `scoped` | Idea file complete. Has proposed approach, size estimate, risks. | Human reviews, or script auto-promotes if small + low risk |
| `ready` | Fully planned and ready to enter the changes workflow. | Script picks this up |
| `in_progress` | Currently being executed via changes workflow. | Agent (via changes workflow) |
| `done` | Complete. Logged in work log. | Archive periodically |
| `blocked` | Needs human decision. `needs_human: true`. | Human |

### Script Queries

```
get_next_task():       status == ready, sort by priority/created
get_autonomous_ready(): status == ready AND size in [small, medium] AND risk != high AND needs_human == false
get_needs_triage():    status == new
get_blocked():         status == blocked OR needs_human == true
get_in_progress():     status == in_progress
```

---

## Component 2: Changes Workflow (existing)

The execution engine. Takes a work item through:

**PRD → Research → Design → Spec → Build → Review**

Each phase runs in a **fresh agent** to avoid context overload on long-running changes. The script manages the phase transitions, spawning a new agent for each one and passing forward only the relevant context (item metadata, idea file, previous phase output).

The workflow scales to the task:
- A small fix might breeze through PRD/spec in seconds with a one-line description
- A medium scoped item uses the idea file as the starting point for the PRD
- A large feature gets the full treatment with thorough planning

**Input:** A backlog item (with its idea file if one exists)
**Output per phase:** A result with a status code, phase artifacts, and any follow-ups

### Phase Tracking

Items in `in_progress` carry a `phase` field:

```yaml
- id: add-user-auth
  status: in_progress
  phase: design        # prd | research | design | spec | build | review
  phase_attempts: 0    # retry counter for current phase
```

### Agent Return Protocol

Every agent returns a structured result so the script knows what to do next:

| Code | Meaning | Script Action |
|---|---|---|
| `PHASE_COMPLETE` | Phase finished successfully. Output saved. | Advance to next phase. Reset `phase_attempts` to 0. |
| `BLOCKED` | Can't proceed. Needs human decision or input. | Set `status: blocked`, `needs_human: true`. Log reason. Stop. |
| `FAILED` | Something went wrong. | Retry up to `max_retries` (default 2, so 3 total attempts). If exhausted → set `status: blocked`, log failure context. |
| `NEEDS_INPUT` | Phase requirements are ambiguous. Needs clarification on the task, not an architectural decision. | Set `status: blocked`. Log what's unclear. Different from `BLOCKED` in intent — this is "I don't understand the task" not "I need a design decision." |

Agent returns include context with every code:

```
{
    code: "PHASE_COMPLETE" | "BLOCKED" | "FAILED" | "NEEDS_INPUT",
    summary: "what happened this phase",
    output: { ... phase artifacts ... },
    follow_ups: [ ... items discovered during this phase ... ],
    context: "why this code — what went wrong, what's unclear, what's needed"
}
```

This context is critical — when something escalates to you, you can read the `context` field and know exactly what the agent tried, what it hit, and what it thinks is needed. You're reviewing, not investigating.

### Retry Logic

```
max_retries = 2  (3 total attempts)

on FAILED:
    item.phase_attempts += 1
    if item.phase_attempts > max_retries:
        # Exhausted retries — escalate to human
        item.status = blocked
        item.needs_human = true
        log("Failed after {max_retries + 1} attempts: {result.context}")
        stop
    else:
        # Retry with a fresh agent
        # Pass previous failure context so the new agent knows what didn't work
        retry_context = "Previous attempt failed: {result.context}. Try a different approach."
        spawn_fresh_agent(item.phase, extra_context=retry_context)
```

Fresh agent on retry is key — you're not asking the same confused agent to try again, you're giving a new agent the failure context so it can approach differently.

### Manual Phase Control

When you're driving the workflow yourself:

```bash
# Bump to next phase (script spawns agent for it)
./orchestrate.sh advance --target add-user-auth

# Skip to a specific phase (e.g. you wrote the PRD yourself)
./orchestrate.sh advance --target add-user-auth --phase spec

# Or just edit the YAML directly
# Set phase: spec, and the script picks up from there next run
```

This means you can do the high-judgment phases (PRD, design) yourself and let the script handle the more mechanical ones (spec, build) autonomously. Mixed mode, same pipeline.

---

## Component 3: Orchestration Script

The loop. Deterministic logic that manages the pipeline. Owns **state transitions** — both across the backlog lifecycle (new → researching → scoped → ready) and across change phases (prd → research → design → spec → build → review).

### Responsibilities

| Script (deterministic) | Agent (flexible) |
|---|---|
| What to work on next | How to implement it |
| Status and phase transitions | Triage judgment (size, risk assessment) |
| Guardrail checks | Research and scoping |
| Agent lifecycle (spawn, collect output) | Writing idea files and proposals |
| Retry logic and failure handling | Quality assessment |
| Work log appending | Discovering follow-ups |
| Interpreting return codes | Producing return codes with context |

### Main Loop

```
PHASE_ORDER = [prd, research, design, spec, build, review]

function run(mode, target=None):

    if mode == "directed":
        # Human said "work on this"
        item = get_item(target)
        execute(item)

    elif mode == "autonomous":
        # Run the loop until nothing actionable
        while has_work():
            # First: advance any in_progress items to their next phase
            for item in get_items(status="in_progress"):
                result = run_phase(item)
                handle_result(item, result)

            # Then: pick next ready item if nothing is in_progress
            if not get_items(status="in_progress"):
                item = get_next_autonomous_task()
                if item is None:
                    advance_backlog()
                    break
                execute(item)

function execute(item):
    update_backlog(item.id, status="in_progress", phase="prd", phase_attempts=0)
    result = run_phase(item)
    handle_result(item, result)

function run_phase(item):
    # Spawn a fresh agent for this phase
    # Only pass: item metadata, idea file, previous phase output
    agent = spawn_fresh_agent(role=item.phase)
    phase_input = get_phase_context(item)  # previous phase outputs, idea file, etc.
    return agent.run(item, phase_input)

function handle_result(item, result):
    # Save any follow-ups discovered during this phase
    for follow_up in result.follow_ups:
        add_to_backlog(follow_up)

    switch result.code:
        case PHASE_COMPLETE:
            save_phase_output(item, result.output)
            item.phase_attempts = 0

            if item.phase == "review":
                # All phases done
                update_backlog(item.id, status="done", done_date=today())
                append_worklog(item, result.summary)
            else:
                # Advance to next phase
                next = PHASE_ORDER[PHASE_ORDER.index(item.phase) + 1]
                update_backlog(item.id, phase=next)

        case BLOCKED:
            update_backlog(item.id, status="blocked", needs_human=true)
            log("Blocked: {result.context}")

        case NEEDS_INPUT:
            update_backlog(item.id, status="blocked", needs_human=true)
            log("Needs clarification: {result.context}")

        case FAILED:
            item.phase_attempts += 1
            if item.phase_attempts > MAX_RETRIES:
                update_backlog(item.id, status="blocked", needs_human=true)
                log("Failed after {MAX_RETRIES + 1} attempts: {result.context}")
            else:
                # Retry: fresh agent with failure context
                update_backlog(item.id, phase_attempts=item.phase_attempts)
                retry_context = "Previous attempt failed: {result.context}"
                result = run_phase(item, extra_context=retry_context)
                handle_result(item, result)  # recursive, bounded by MAX_RETRIES

function advance_backlog():
    # Push items forward through their lifecycle
    for item in get_items(status="new"):
        agent = spawn_fresh_agent(role="triage")
        agent.run(item)

    for item in get_items(status="researching"):
        agent = spawn_fresh_agent(role="research")
        agent.run(item, item.idea_file)

    for item in get_items(status="scoped"):
        if item.size in ["small"] and item.risk == "low" and not item.needs_human:
            update_backlog(item.id, status="ready")
```

### Guardrails for Autonomous Execution

An item can be autonomously promoted to `ready` and executed if ALL are true:
- `size` is `small` (or `medium` if you loosen over time)
- `risk` is `low`
- `needs_human` is `false`
- No unresolved `dependencies`
- Has been scoped (idea file with proposed approach, if medium+)

If any fail → item stays at current status until human reviews.

### Usage

```bash
# Directed: "work on this specific thing"
./orchestrate.sh run --target fix-webhook-errors

# Autonomous: "work through the queue"
./orchestrate.sh run --auto

# Advance a specific item one phase
./orchestrate.sh advance --target add-user-auth

# Skip to a specific phase (e.g. you did the PRD yourself)
./orchestrate.sh advance --target add-user-auth --phase spec

# Check state: "what's going on?"
./orchestrate.sh status

# Triage: "process new items"
./orchestrate.sh triage
```

---

## Supporting Artifacts

### Ideas Folder (`_ideas/`)

Markdown files with YAML frontmatter for items that need research or scoping. Created when a backlog item needs more than a one-liner.

**Frontmatter** holds structured metadata the script can parse.
**Body** holds freeform research, proposals, and notes that agents build up over time.

```markdown
---
id: api-key-auth
title: "Refactor auth middleware to support API keys"
status: scoped       # draft | researching | scoped | ready
size: medium         # small | medium | large
risk: medium         # low | medium | high
needs_human: false
origin: "change review for auth refactor"
backlog_id: api-key-auth
created: 2026-02-10
last_updated: 2026-02-10
updated_by: agent-abc123
dependencies: []
tags: []
---

## Problem
<!-- What's the issue or opportunity? -->

## Context
<!-- How did this come up? What change surfaced it? -->

## Research
<!-- Dated entries. Agents append as they investigate. -->

### 2026-02-10 - agent-abc123
...

## Proposed Approach
<!-- Current best thinking. Updated as research evolves. -->

## Alternatives Considered
| Alternative | Pros | Cons | Why Not |
|---|---|---|---|

## Open Questions / Decisions Needed
- [ ] ...

## Scope Estimate
- **Effort:**
- **Dependencies:**
- **Touches:**
```

**When to create:** Item needs research, has complexity, or has open questions.
**When NOT to create:** Item is small and self-explanatory (stays as backlog one-liner).

### Work Log (`_worklog/YYYY-MM.md`)

Monthly-partitioned, append-only log of completed work. The script writes entries after each item completes. Human-facing audit trail.

```markdown
# Work Log - 2026-02

## 2026-02-10

### Fixed error handling in webhook parser [small]
- **Source:** Backlog (follow-up from auth refactor)
- **What:** Added validation for missing required fields. Returns 400 with field-level errors.
- **Files touched:** src/routes/webhooks.rs, src/validation/schemas.rs
- **Notes:** Found /update has same issue — added to backlog.

### Researched rate limiting approaches [research]
- **Source:** Backlog advance (rate-limiting)
- **What:** Investigated token bucket vs sliding window. Updated idea file.
- **Files touched:** _ideas/rate-limiting.md
- **Notes:** Leaning token bucket with per-user keying. Flagged scope question.
```

---

## State at a Glance

When you pull back in:

| Question | Where to Look |
|---|---|
| What got done? | `_worklog/YYYY-MM.md` → latest entries |
| What's in flight and what phase? | `BACKLOG.yaml` → `status: in_progress`, check `phase` |
| What needs my judgment? | `BACKLOG.yaml` → `status: blocked` — check `notes` for context on why |
| What failed? | `BACKLOG.yaml` → `status: blocked` with `phase_attempts > 0` |
| What's ready to go? | `BACKLOG.yaml` → `status: ready` |
| What's been researched? | `_ideas/` → check frontmatter `status` |
| Full picture of remaining work? | `BACKLOG.yaml` → all non-done items |
| What happened this month? | `_worklog/2026-02.md` |

---

## Example Flow

1. You run `./orchestrate.sh run --target add-user-auth` to kick off a big feature.
2. Script sets `status: in_progress`, `phase: prd`. Spawns fresh agent for PRD phase.
3. Agent returns `PHASE_COMPLETE`. Script saves PRD output, advances to `phase: research`.
4. Fresh agent for research phase. `PHASE_COMPLETE`. Advances to `phase: design`.
5. Fresh agent for design phase. `PHASE_COMPLETE`. Advances to `phase: spec`.
6. Fresh agent for spec phase. Returns `FAILED` — couldn't resolve a dependency conflict.
7. Script increments `phase_attempts: 1`. Spawns fresh agent with failure context: "Previous attempt failed: dependency conflict between auth middleware and session store."
8. Retry agent takes a different approach. `PHASE_COMPLETE`. Script resets `phase_attempts: 0`, advances to `phase: build`.
9. During build, agents discover follow-ups: inconsistent error messages, missing rate limiting, session store should use Redis, middleware needs tests.
10. Build complete. `phase: review`. Review agent triages follow-ups into the backlog:
    - `middleware-tests` → `status: ready` (small, low risk, clear scope)
    - `error-consistency` → `status: ready` (small, low risk)
    - `rate-limiting` → `status: new` (needs research)
    - `redis-sessions` → `status: new` (needs research, large)
11. Review complete. Script marks `add-user-auth` as done, appends to work log.
12. You leave. Script continues in autonomous mode.
13. Script picks `middleware-tests` (ready) → enters changes workflow → phases breeze through → done.
14. Script picks `error-consistency` → same → done.
15. Queue empty. Script runs `advance_backlog()`.
16. Agent triages `rate-limiting`: creates idea file, starts researching → `status: researching`.
17. Agent triages `redis-sessions`: creates idea file, flags as large/high-risk → `status: blocked`, `needs_human: true`.
18. Agent finishes rate limiting research, updates idea file → `status: scoped`.
19. Script checks: scoped, medium, low risk, no human needed → promotes to `ready`.
20. Script picks `rate-limiting` → changes workflow → done.
21. You come back. You scan:
    - **Done:** auth feature, middleware tests, error consistency, rate limiting ✓
    - **Blocked:** redis-sessions needs your call (large, high risk)
    - **Work log:** full trail of what happened and why
22. You decide to plan redis-sessions yourself. You write a PRD, update the YAML:
    ```bash
    ./orchestrate.sh advance --target redis-sessions --phase research
    ```
    Script picks it up from research phase and continues autonomously.

---

## File Structure

```
project/
├── BACKLOG.yaml              # The work queue
├── _ideas/                   # Detailed research/scoping docs
│   ├── _TEMPLATE.md
│   ├── api-key-auth.md
│   ├── rate-limiting.md
│   └── redis-sessions.md
├── _changes/                 # Active change phase outputs
│   └── add-user-auth/       # One folder per in-progress item
│       ├── prd.md
│       ├── research.md
│       ├── design.md
│       ├── spec.md
│       └── build-log.md
├── _worklog/                 # Monthly completion logs
│   ├── 2026-01.md
│   └── 2026-02.md
└── src/                      # Your actual code
    └── ...
```
