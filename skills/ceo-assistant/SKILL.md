---
name: ceo-assistant
version: 0.2.0
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
requires:
  skills:
    - commitment-triage
    - commitment-digest
    - decision-capture
    - delegation-tracker
    - idea-parking
---

# CEO / Manager — Commitment System Setup

You are configuring the commitments system for an executive or manager. Their day is dominated by:
- Back-to-back meetings where decisions are made verbally
- Constant delegation — most commitments are "make sure someone else does X"
- Information flowing in both directions: team → executive (synthesis needed) and executive → team (tracking needed)

## Companion skills

This bundle relies on these skills activating during conversation (they are keyword-triggered, no manual install needed):

| Skill | Activates when | What it does |
|---|---|---|
| `commitment-triage` | User mentions obligations, deadlines, promises | Extracts signals, creates/resolves commitments |
| `commitment-digest` | User asks "show commitments" or similar | Composes formatted summary |
| `decision-capture` | User makes a decision ("let's go with X") | Records decision with rationale |
| `delegation-tracker` | User delegates ("tell Sarah to...", "waiting on...") | Tracks delegation, flags stale follow-ups |
| `idea-parking` | User says "park this idea", "save for later" | Parks ideas for periodic resurfacing |

If any of these skills are missing from the `skills/` directory, tell the user which ones are needed and where to find them.

## Step 1: Ask configuration questions

Before creating anything, ask the user:

1. **Digest schedule**: Morning + evening works for most executives (8am + 5pm). Want different times?
2. **Delegation follow-up style**: When I follow up on delegated items, should I draft a polite check-in or a direct status request? (default: polite check-in)
3. **Channels to watch**: Which communication channels carry actionable messages? (Slack channels, email, etc.)
4. **Exclusions**: Any channels or message types to ignore?

Use reasonable defaults if the user says "just set it up."

## Step 2: Create workspace structure

Run the `commitment-setup` skill's workspace creation procedure. Specifically:

1. Check if `commitments/README.md` exists via `memory_read`. If it does, skip to Step 3.
2. Write `commitments/README.md` with the full schema — it must document frontmatter for signals (with immediacy/expires_at/destination), commitments (with resolution_path/stale_after/resolved_by), decisions (with outcome tracking), and parked ideas. See `commitment-setup` skill for the complete content.
3. Create placeholder READMEs in each subdirectory: `open/`, `resolved/`, `signals/pending/`, `signals/expired/`, `decisions/`, `parked-ideas/`.

## Step 3: Create tuned missions

### Triage mission — faster scan for executives

Executives generate obligations rapidly. Scan 3x daily. Signal expiration shortened to 24 hours.

```
mission_create(
  name: "commitment-triage",
  goal: "Executive triage. Read commitments/README.md for schema. Priority order: (1) Check delegated items (status=waiting, delegated_to set) — if not updated in 2 days, flag for follow-up and draft a polite check-in message. (2) Check overdue items — escalate urgency. (3) Expire signals older than 24 hours (executives move fast, stale signals are noise). (4) For signals with immediacy=realtime, broadcast immediately via message tool. (5) Promote high-confidence signals to commitments — default resolution_path to needs_decision for ambiguous items. (6) Route informational signals to intelligence (write MemoryDoc to context/intel/). (7) Append triage summary to commitments/triage-log.md. (8) If anything needs attention, send a concise alert — executives scan, not read.",
  cadence: "0 9,13,18 * * *"
)
```

### Digest mission — morning and evening, grouped by responsibility

```
mission_create(
  name: "commitment-digest",
  goal: "Executive commitments digest. Read commitments/README.md for schema. Gather all open commitments via memory_tree and memory_read. Group by responsibility: (1) DELEGATED — items where delegated_to is set, with days since delegation and follow-up status. (2) OWNED — items you need to act on personally, sorted by urgency. For agent_can_handle items, note what the agent would do and ask permission. (3) DECISIONS PENDING — items with resolution_path=needs_decision. (4) RECENT DECISIONS — decisions captured in the last 7 days (from commitments/decisions/), including any needing outcome assessment. Keep each item to one line. End with pending signal count and 'Did I miss anything?' Send via message tool.",
  cadence: "0 8,17 * * 1-5"
)
```

## Step 4: Write calibration memories

```
memory_write(
  target: "commitments/calibration.md",
  content: "# Executive Commitment Calibration\n\n- Group commitments by responsibility type in digests — delegated items shown separately from owned items\n- For delegation follow-ups, draft a polite check-in rather than a blunt status request\n- Only capture explicit decisions, not brainstorming or hypotheticals ('yeah let's do X' = decision; 'maybe we should' = not a decision)\n- Signal expiration is 24 hours — executives move fast, stale signals are noise\n- Most CEO commitments are delegations, not personal tasks — default delegated_to when someone else is mentioned\n- When capturing decisions, note who was present and what it affects — executives revisit decisions frequently\n- Keep all communications scannable: bullet points, one-liners, no paragraphs\n- Start conservative: surface everything, don't auto-promote signals or auto-dispatch agent_can_handle without approval",
  append: false
)
```

## Step 5: Confirm

Tell the user:

> Your executive commitment system is ready:
> - **Triage** runs 3x daily (9am, 1pm, 6pm) — delegation follow-ups after 2 days, signals expire after 24h
> - **Digest** runs morning (8am) and evening (5pm) on weekdays — grouped by delegated vs owned vs decisions pending
> - I'll capture decisions from our conversations and track delegations automatically
> - For items I can handle (PR reviews, drafts, research), I'll ask your permission first
> - Say **"show commitments"** anytime, or **"who owes me what?"** for delegation status
> - I start conservative — I'll learn your preferences over time as you confirm or override my suggestions
