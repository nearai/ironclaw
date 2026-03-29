---
name: commitment-digest
version: 0.1.0
description: Compose and deliver summaries of open commitments, deadlines, pending signals, and recent resolutions.
activation:
  keywords:
    - show commitments
    - commitment digest
    - commitment summary
    - commitment report
    - open commitments
    - my obligations
    - what do I owe
    - pending tasks
    - what's overdue
    - what's due
    - commitment status
  patterns:
    - "(?i)(show|list|summarize|review) (my )?(commitments|obligations|deadlines|tasks)"
    - "(?i)what('s| is| are) (pending|overdue|due|open)"
    - "(?i)commitment (digest|report|status|summary)"
  tags:
    - commitments
    - digest
    - reporting
  max_context_tokens: 1500
---

# Commitment Digest

Compose a summary of the user's current commitments. This skill is used both in-conversation (user asks "show commitments") and by the `commitment-digest` routine for scheduled delivery.

## Gathering data

1. `memory_tree("commitments/open/", depth=1)` — list all open commitment files (skip README.md)
2. `memory_read` each file to extract frontmatter: status, urgency, due, delegated_to, tags
3. `memory_tree("commitments/signals/pending/", depth=1)` — count pending signals (skip README.md)
4. `memory_tree("commitments/resolved/", depth=1)` — count recently resolved (optional: read to check dates)

## Composing the digest

Group commitments by urgency and present in this order:

```
## Commitments — <today's date>

### Overdue / Critical
- **<title>** (due <date>) — owner: <owner>
  <one-line status note if relevant>

### Due This Week
- **<title>** (due <date>) — owner: <owner>, delegated to: <person>

### Open (no deadline)
- **<title>** — owner: <owner>

### Waiting / Blocked
- **<title>** — waiting on <person> since <date>

### Pending Signals (<count>)
<count> unprocessed signals in queue. Say "review signals" to triage them.

### Recently Resolved
- <title> (resolved <date>)
```

**Rules:**
- Omit empty sections entirely — don't show "Overdue" if nothing is overdue
- Keep each item to one line plus optional status note
- If there are zero open commitments and zero pending signals, say: "No open commitments. You're clear."
- Flag commitments not updated in 7+ days as "(stale — still relevant?)"
- For delegated items past 3 days without update, note: "(follow-up suggested)"

## Delivery

- **In conversation:** Display the digest inline as your response
- **From routine:** Send via `message` tool to the user's preferred channel
- **Tone:** Concise and scannable. This is a dashboard, not a narrative.
