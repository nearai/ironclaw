---
name: ceo-assistant
version: 0.1.0
description: Commitment tracking tuned for executives and managers — delegation-heavy, meeting prep, decision capture, morning and evening digests.
activation:
  keywords:
    - ceo assistant
    - executive assistant
    - manager assistant
    - delegation setup
    - meeting prep
    - action items
    - leadership workflow
  patterns:
    - "(?i)I'm a (CEO|manager|executive|director|VP|founder)"
    - "(?i)set ?up.*(executive|manager|leadership|delegation)"
    - "(?i)help me manage my (day|schedule|team|obligations)"
  tags:
    - commitments
    - executive
    - delegation
    - setup
  max_context_tokens: 2000
---

# CEO / Manager — Commitment System Setup

You are configuring the commitments system for an executive or manager. Their day is dominated by:
- Back-to-back meetings where decisions are made verbally
- Constant delegation — most commitments are "make sure someone else does X"
- Information flowing in both directions: team → executive (synthesis needed) and executive → team (tracking needed)

## Step 1: Ask configuration questions

Before creating anything, ask the user:

1. **Timezone and channel**: What timezone are you in? Which channel should I send digests to?
2. **Digest schedule**: Morning + evening works for most executives (8am + 5pm). Want different times?
3. **Delegation follow-up style**: When I follow up on delegated items, should I draft a polite check-in or a direct status request? (default: polite check-in)
4. **Channels to watch**: Which communication channels carry actionable messages? (Slack channels, email, etc.)
5. **Exclusions**: Any channels or message types to ignore?

Use reasonable defaults if the user says "just set it up."

## Step 2: Create workspace structure

Check if `commitments/README.md` exists. If not, create the full commitments workspace structure:

1. Write `commitments/README.md` with the standard schema (same as commitment-setup skill)
2. Create placeholder READMEs in subdirectories: `open/`, `resolved/`, `signals/pending/`, `signals/expired/`, `decisions/`, `parked-ideas/`

If it already exists, skip this step and move to configuring routines.

## Step 3: Create tuned routines

### Triage routine — faster scan for executives

Executives generate obligations rapidly. Scan more frequently than the default.

```
routine_create(
  name: "commitment-triage",
  description: "Executive triage — scan for obligations, delegation follow-ups, and stale items",
  prompt: "You are triaging commitments for an executive. Read commitments/README.md for the schema. Priority order: (1) Check delegated items (status=waiting, delegated_to set) — if not updated in 2 days, flag for follow-up. Draft a polite check-in message. (2) Check overdue items. (3) Expire signals older than 24 hours (executives move fast, stale signals are noise). (4) Append triage summary to commitments/triage-log.md. (5) If anything needs attention, send a concise alert — executives scan, not read.",
  request: { kind: "cron", schedule: "0 9,13,18 * * *", timezone: "<user_timezone>" },
  execution: { mode: "lightweight", use_tools: true, max_tool_rounds: 6, context_paths: ["commitments/README.md"] }
)
```

Three runs daily: morning, midday, evening. Signal expiration shortened to 24 hours.

### Digest routine — morning and evening, grouped by responsibility

```
routine_create(
  name: "commitment-digest",
  description: "Executive digest — commitments grouped by responsibility type",
  prompt: "Compose an executive commitments digest. Read commitments/README.md for schema. Gather all open commitments via memory_tree and memory_read. Group by responsibility: (1) DELEGATED — items waiting on others, with days since delegation and follow-up status. (2) OWNED — items you need to act on personally, sorted by urgency. (3) DECISIONS PENDING — items needing a decision from you. (4) RECENT DECISIONS — decisions captured in the last 7 days (from commitments/decisions/). Keep each item to one line. End with pending signal count. Send via message tool.",
  request: { kind: "cron", schedule: "0 8,17 * * MON-FRI", timezone: "<user_timezone>" },
  execution: { mode: "lightweight", use_tools: true, max_tool_rounds: 6, context_paths: ["commitments/README.md"] }
)
```

## Step 4: Write calibration memories

Write these to the workspace as behavioral guidance that persists across sessions:

```
memory_write(
  target: "commitments/calibration.md",
  content: "# Executive Commitment Calibration\n\n- Group commitments by responsibility type in digests — delegated items shown separately from owned items\n- For delegation follow-ups, draft a polite check-in rather than a blunt status request\n- Only capture explicit decisions, not brainstorming or hypotheticals ('yeah let's do X' = decision; 'maybe we should' = not a decision)\n- Signal expiration is 24 hours — executives move fast, stale signals are noise\n- Most CEO commitments are delegations, not personal tasks — default responsibility to DelegatedTo when someone else is mentioned\n- When capturing decisions, note who was present and what it affects — executives revisit decisions frequently\n- Keep all communications scannable: bullet points, one-liners, no paragraphs",
  append: false
)
```

## Step 5: Confirm

Tell the user:

> Your executive commitment system is ready:
> - **Triage** runs 3x daily (9am, 1pm, 6pm) — delegation follow-ups after 2 days, signals expire after 24h
> - **Digest** runs morning (8am) and evening (5pm) on weekdays — grouped by delegated vs owned vs decisions pending
> - I'll capture decisions from our conversations and track delegations automatically
> - Say **"show commitments"** anytime, or **"who owes me what?"** for delegation status
> - Adjust anything by telling me: "make digests more frequent", "follow up after 1 day instead of 2", etc.
