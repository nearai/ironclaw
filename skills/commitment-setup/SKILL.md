---
name: commitment-setup
version: 0.1.0
description: One-time setup for the commitments tracking system. Creates workspace structure, schema docs, and installs triage and digest routines.
activation:
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
---

# Commitment System Setup

You are installing the commitments tracking system. This creates a workspace structure for tracking obligations, signals, decisions, and parked ideas, plus two routines for automated triage and digest delivery.

## Step 1: Check existing setup

Call `memory_read(path="commitments/README.md")`. If it exists, tell the user: "The commitments system is already set up. Want me to reinstall from scratch?" Stop unless they confirm.

## Step 2: Gather user context

Read `USER.md` to find the user's timezone and preferred communication channel. If not found, ask:
1. What timezone are you in? (default: UTC)
2. Which channel should I send digests to? (default: the current channel)

## Step 3: Write the schema README

Call `memory_write` with `target="commitments/README.md"`, `append=false`, and this content:

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
    confidence: high | medium | low
    obligation_type: reply | deliver | attend | review | decide | follow-up
    mentions: [<names>]
    promoted_to: null | <commitment filename>
    ---
    <Human-readable description of what was detected.>

## Commitment Schema (open/<slug>.md or resolved/<slug>.md)

    ---
    type: commitment
    status: open | blocked | waiting | resolved
    urgency: critical | high | medium | low
    due: <YYYY-MM-DD> | null
    created_at: <YYYY-MM-DD>
    owner: user | agent
    delegated_to: null | <person or team>
    source_signal: <relative path> | null
    tags: [<freeform>]
    ---
    # <Title>
    <Description of the obligation.>

    ## Resolution path
    - [ ] Step 1
    - [ ] Step 2

    ## Progress
    <Updates appended over time.>

## Decision Schema (decisions/<date>-<slug>.md)

    ---
    type: decision
    decided_at: <YYYY-MM-DD>
    context: <topic slug>
    participants: [<names>]
    confidence: high | medium | low
    reversible: true | false
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
```

## Step 4: Create directory placeholders

Write a one-line README in each subdirectory to establish the structure:

- `memory_write(target="commitments/open/README.md", content="Active commitments.", append=false)`
- `memory_write(target="commitments/resolved/README.md", content="Completed commitments archive.", append=false)`
- `memory_write(target="commitments/signals/pending/README.md", content="Signals awaiting triage.", append=false)`
- `memory_write(target="commitments/signals/expired/README.md", content="Expired signals.", append=false)`
- `memory_write(target="commitments/decisions/README.md", content="Captured decisions.", append=false)`
- `memory_write(target="commitments/parked-ideas/README.md", content="Ideas for later.", append=false)`

## Step 5: Check for existing routines

Call `routine_list`. If routines named `commitment-triage` or `commitment-digest` already exist, skip creating them (mention they are already installed).

## Step 6: Create the triage routine

```
routine_create(
  name: "commitment-triage",
  description: "Review pending signals, expire stale ones, check for overdue commitments",
  prompt: "You are running a commitments triage. Read commitments/README.md for the schema. Then: (1) memory_tree('commitments/signals/pending/', depth=1) to list pending signals. For any signal with detected_at older than 48 hours and promoted_to=null, move it to signals/expired/ by writing a copy there and overwriting the pending file with empty content. (2) memory_tree('commitments/open/', depth=1) to list open commitments. For each, memory_read it and check: if due date is past, flag as overdue; if status=waiting and not updated in 3+ days, flag for follow-up. (3) Append a brief triage summary with today's date to commitments/triage-log.md. (4) If any items are overdue or need follow-up, send a message alerting the user.",
  request: { kind: "cron", schedule: "0 9,18 * * *", timezone: "<user_timezone>" },
  execution: { mode: "lightweight", use_tools: true, max_tool_rounds: 5, context_paths: ["commitments/README.md"] }
)
```

Replace `<user_timezone>` with the timezone from Step 2.

## Step 7: Create the digest routine

```
routine_create(
  name: "commitment-digest",
  description: "Morning summary of open commitments, deadlines, and pending signals",
  prompt: "You are composing a commitments digest. Read commitments/README.md for the schema. Then: (1) memory_tree('commitments/open/', depth=1) and memory_read each file. Extract status, urgency, due date, delegated_to from frontmatter. (2) memory_tree('commitments/signals/pending/', depth=1) to count pending signals. (3) Compose a digest grouped by urgency: Critical/Overdue first, then Due This Week, then Open (no deadline), then count of Pending Signals. Keep it concise. (4) Send the digest via message tool.",
  request: { kind: "cron", schedule: "0 8 * * MON-FRI", timezone: "<user_timezone>" },
  execution: { mode: "lightweight", use_tools: true, max_tool_rounds: 5, context_paths: ["commitments/README.md"] }
)
```

## Step 8: Confirm

Tell the user:

> Commitments system is ready. Here is what I set up:
> - Workspace structure under `commitments/` with schema docs
> - **Triage routine** runs twice daily (9am and 6pm) — expires stale signals, flags overdue items
> - **Digest routine** runs weekday mornings at 8am — summarizes your open commitments
>
> I will automatically track obligations from our conversations. Say **"show commitments"** anytime to see your current status. You can adjust the schedule by saying something like "give me digests at 7am instead."
