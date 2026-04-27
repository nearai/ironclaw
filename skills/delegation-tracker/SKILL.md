---
name: delegation-tracker
version: 0.1.0
description: Track delegated commitments, set follow-up timers, and generate nudge reminders when updates are overdue.
activation:
  keywords:
    - delegated
    - assigned to
    - waiting on
    - follow up with
    - check with
    - ask them
    - handed off
    - they said
    - blocked on
    - no response
    - nudge
    - chase up
  patterns:
    - "(?i)(delegated|assigned|handed off) (to|this to)"
    - "(?i)(waiting|blocked) on .+ (to|for)"
    - "(?i)follow up with .+ (about|on|regarding)"
    - "(?i)check (with|in with) .+ (about|on)"
    - "(?i)(tell|ask) .+ to .+"
  tags:
    - commitments
    - delegation
  max_context_tokens: 1200
---

# Delegation Tracker

Track commitments that depend on someone else. The triage routine will flag stale delegations for follow-up.

## When delegation is detected

User says: "I asked Sarah to handle the deployment" or "tell ops to investigate staging" or "waiting on legal for the contract review."

This skill is only successful if the delegation is persisted in workspace
memory. Do not just acknowledge or summarize a delegation.

Execution order is mandatory:
1. Check for an existing matching commitment
2. If needed, write or update the commitment in `projects/commitments/open/`
3. Only then confirm what was tracked

Never say "tracked", "saved", or "I'll flag it" unless the corresponding
`memory_write` succeeded.

**Action:**
1. Check if a matching commitment already exists in `projects/commitments/open/` via `memory_search`
2. If it exists: update the frontmatter — set `delegated_to: <person/team>`, `status: waiting`
3. If it doesn't exist: create a new commitment in `projects/commitments/open/` with:

```
---
type: commitment
status: waiting
urgency: <infer from context>
due: <if mentioned, else null>
created_at: <today>
owner: user
delegated_to: <person or team name>
source_signal: null
tags: [delegation]
---
# <What was delegated>
<Description of what was delegated and to whom.>

## Follow-up
- Delegated on: <today>
- Expected response by: <date if given, else 3 days from now>
- Last checked: never
```

4. After the write succeeds, confirm: "Tracked — waiting on <person> for <topic>. I'll flag it if there's no update by <date>."

## When an update is received

User says: "Sarah got back to me about the deployment" or "legal approved the contract."

**Action:**
1. Find the matching commitment via `memory_search` or `memory_tree("projects/commitments/open/")`
2. If the delegation is resolved: update status and move to `projects/commitments/resolved/`
3. If partially resolved: update the Progress section, keep status as `open` or `waiting`
4. Confirm the update

## Follow-up generation

The triage routine handles automated follow-up detection. It reads open commitments where `delegated_to` is set and `status: waiting`, checks the "Expected response by" date, and flags overdue items in the triage alert.

When the user asks to follow up explicitly ("nudge Sarah about the deployment"), draft a brief follow-up message and offer to send it if a channel to that person is available.
