---
name: commitment-setup
version: 0.3.0
description: One-time setup for the commitments tracking system. Creates workspace structure, schema docs, and installs triage and digest missions. Excluded from activation once `projects/commitments/README.md` exists in the workspace (the file this skill writes as its first step).
activation:
  # commitment-setup writes projects/commitments/README.md as its first step, so
  # the marker is automatically set after a successful first run. To
  # re-trigger (e.g. migrate to a new schema), delete README.md first.
  setup_marker: projects/commitments/README.md
  keywords:
    - setup commitments
    - install commitments
    - enable commitments
    - commitment system
    - initialize commitments
    - commitment tracking
  patterns:
    - "(?i)set ?up.*(commitment|obligation|tracking)"
    - "(?i)install.*(commitment|obligation)"
    - "(?i)enable.*(commitment|tracking)"
  tags:
    - commitments
    - setup
    - personal-assistant
  max_context_tokens: 2000
requires:
  skills:
    - commitment-triage
    - commitment-digest
---

# Commitment System Setup

You are installing the commitments tracking system. This creates a workspace structure for tracking obligations, signals, decisions, and parked ideas, plus two missions for automated triage and digest delivery.

## Step 1: Check existing setup

Call `memory_read(path="projects/commitments/README.md")`. If it exists, tell the user: "The commitments system is already set up. Want me to reinstall from scratch?" Stop unless they confirm.

## Step 2: Gather user context

The user's timezone is provided by the channel automatically — do not ask for it. Ask only:
1. Which channel should I send digests to? (default: the current channel)

## Step 3: Write the schema README

Call `memory_write` with `target="projects/commitments/README.md"`, `append=false`, and this content:

```
# Commitments System

Tracks obligations, decisions, and ideas via structured markdown files.

## Directory Layout

- `open/` — Active commitments (one file each)
- `resolved/` — Completed commitments (archived)
- `signals/pending/` — Raw extracted signals awaiting triage
- `signals/expired/` — Signals that were not promoted in time
- `decisions/` — Captured decisions with rationale
- `parked-ideas/` — Ideas saved for later consideration

## Signal Schema (signals/pending/<slug>.md)

    ---
    type: signal
    source_channel: <channel name>
    source_message: "<brief quote or paraphrase>"
    detected_at: <YYYY-MM-DD>
    immediacy: realtime | prompt | batch
    expires_at: <YYYY-MM-DD> | null
    confidence: high | medium | low
    obligation_type: reply | deliver | attend | review | decide | follow-up | informational
    mentions: [<names>]
    destination: null | commitment | parked_idea | intelligence | dismissed
    promoted_to: null | <commitment filename>
    ---
    <Human-readable description of what was detected.>

### Immediacy levels
- realtime: push-notify immediately (production incident, market alert, security)
- prompt: surface within the hour (urgent DM, trending topic)
- batch: next digest is fine (meeting action item, report to read)

### Signal destinations
- commitment: actionable, tracked, has a resolution path
- parked_idea: interesting, not committed, revisit later
- intelligence: no action needed, but informs future decisions → write a MemoryDoc via memory_write
- dismissed: not relevant

## Commitment Schema (open/<slug>.md or resolved/<slug>.md)

    ---
    type: commitment
    status: open | in_progress | blocked | waiting | resolved
    urgency: critical | high | medium | low
    due: <YYYY-MM-DD> | null
    created_at: <YYYY-MM-DD>
    stale_after: <YYYY-MM-DD> | null
    owner: user | agent
    delegated_to: null | <person or team>
    resolution_path: agent_can_handle | needs_reply | needs_decision | note_only
    decision_type: mechanical | taste | challenge
    effort_human: <estimate> | null
    effort_assisted: <estimate> | null
    source_signal: <relative path> | null
    resolved_by: null | agent | user | delegate | expired
    tags: [<freeform>]
    ---
    # <Title>
    <Description of the obligation.>

    ## Resolution path
    - [ ] Step 1
    - [ ] Step 2

    ## Progress
    <Updates appended over time.>

### Resolution path types
- agent_can_handle: the agent can do this autonomously (review PR, draft doc, research)
- needs_reply: user must send a response to someone
- needs_decision: user must choose between options
- note_only: informational, no action needed but tracked

### Decision types (when to ask the user)
- mechanical: auto-act silently (expire stale signal, update status, dismiss noise). Report in digest as "auto-handled."
- taste: auto-act but surface for awareness ("I auto-dismissed 3 FYI signals, auto-resolved 2 completed items"). User can override.
- challenge: always ask the user before acting (architecture decisions, sending messages, spending money, irreversible actions).

### Effort estimates
When known, include dual estimates:
- effort_human: time without AI assistance (e.g. "2h", "3d")
- effort_assisted: time with AI assistance (e.g. "15min", "2h")
This reframes decisions — when AI makes completeness cheap, there is no excuse for shortcuts.

### Autonomous resolution
When a commitment has resolution_path=agent_can_handle and decision_type=mechanical or taste:
1. Agent handles it and reports in the next digest
When decision_type=challenge:
1. Agent asks for explicit approval before acting
2. On approval, spawns a mission for complex work or handles inline
3. Status transitions to in_progress, then resolved with resolved_by=agent

## Decision Schema (decisions/<date>-<slug>.md)

    ---
    type: decision
    decided_at: <YYYY-MM-DD>
    context: <topic slug>
    participants: [<names>]
    confidence: high | medium | low
    reversible: true | false
    outcome: null | <brief outcome description>
    outcome_positive: null | true | false
    tags: [<freeform>]
    ---
    # <What was decided>

    ## Context
    <Why this decision was needed.>

    ## Options considered
    1. **Option A** — pros/cons
    2. **Option B** — pros/cons

    ## Rationale
    <Why this option was chosen.>

    ## Outcome
    <Filled in later: what happened as a result of this decision.>

## Parked Idea Schema (parked-ideas/<slug>.md)

    ---
    type: parked-idea
    parked_at: <YYYY-MM-DD>
    source: conversation | triage | research
    relevance: high | medium | low
    tags: [<freeform>]
    ---
    # <Idea title>
    <Description and why it is interesting.>

    ## Activation trigger
    <What would make this worth pursuing.>

## Conventions

- Filenames use lowercase kebab-case: `review-sarah-deck.md`
- Dates are ISO-8601: `YYYY-MM-DD`
- Moving a commitment from open/ to resolved/: write the updated file to resolved/, then overwrite the open/ file with empty content
- One file per entity — never batch multiple commitments into one file

## Trust calibration

Start conservative:
- All signals surfaced in digest, none auto-promoted to commitments
- All agent_can_handle commitments require explicit user approval before dispatch
- Realtime immediacy disabled initially (everything batched)
- Track user feedback patterns in projects/commitments/calibration.md to gradually increase autonomy
```

## Step 4: Create directory placeholders

Write a one-line README in each subdirectory to establish the structure:

- `memory_write(target="projects/commitments/open/README.md", content="Active commitments.", append=false)`
- `memory_write(target="projects/commitments/resolved/README.md", content="Completed commitments archive.", append=false)`
- `memory_write(target="projects/commitments/signals/pending/README.md", content="Signals awaiting triage.", append=false)`
- `memory_write(target="projects/commitments/signals/expired/README.md", content="Expired signals.", append=false)`
- `memory_write(target="projects/commitments/decisions/README.md", content="Captured decisions.", append=false)`
- `memory_write(target="projects/commitments/parked-ideas/README.md", content="Ideas for later.", append=false)`

## Step 5: Check for existing missions

Call `mission_list`. If missions named `commitment-triage` or `commitment-digest` already exist, skip creating them.

## Step 6: Create the triage mission

```
mission_create(
  name: "commitment-triage",
  goal: "Review pending signals, expire stale ones, check for overdue commitments. Read projects/commitments/README.md for the schema. Then: (1) memory_tree('projects/commitments/signals/pending/', depth=1) to list pending signals. For any signal past its expires_at or older than 48 hours with destination=null, move it to signals/expired/. (2) memory_tree('projects/commitments/open/', depth=1) to list open commitments. For each, memory_read and check: if due date is past, flag as overdue; if status=waiting and not updated in 3+ days, flag for follow-up; if stale_after is past, re-surface with escalated urgency. (3) For signals with immediacy=realtime, broadcast immediately via message tool — do not wait for digest. (4) Append triage summary to projects/commitments/triage-log.md. (5) If any items are overdue or need follow-up, send a message alerting the user.",
  cadence: "0 9,18 * * *"
)
```

## Step 7: Create the digest mission

```
mission_create(
  name: "commitment-digest",
  goal: "Compose a morning commitments digest. Read projects/commitments/README.md for the schema. (1) memory_tree('projects/commitments/open/', depth=1) and memory_read each file. Extract status, urgency, due, delegated_to, resolution_path from frontmatter. (2) memory_tree('projects/commitments/signals/pending/', depth=1) to count pending signals. (3) Compose digest grouped by: Overdue/Critical first, then Due This Week, then Waiting/Delegated (with follow-up status), then Open (no deadline). For agent_can_handle items, note 'I can handle this — want me to proceed?' (4) End with: pending signal count and 'Did I miss anything? Tell me if I overlooked an obligation.' (5) Send via message tool.",
  cadence: "0 8 * * 1-5"
)
```

## Step 8: Confirm

Tell the user:

> Commitments system is ready. Here is what I set up:
> - Workspace structure under `projects/commitments/` with schema docs
> - **Triage mission** runs twice daily (9am and 6pm) — expires stale signals, flags overdue items, broadcasts realtime alerts
> - **Digest mission** runs weekday mornings at 8am — summarizes open commitments with resolution suggestions
>
> I will automatically track obligations from our conversations. Say **"show commitments"** anytime to see your current status. You can adjust the schedule by saying something like "give me digests at 7am instead."
>
> I start conservative — I'll surface everything but won't act without your approval. As you confirm or dismiss my suggestions, I'll learn your preferences.
