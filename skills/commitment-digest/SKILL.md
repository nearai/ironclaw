---
name: commitment-digest
version: 0.2.0
description: Compose and deliver summaries of open commitments, deadlines, pending signals, and resolution suggestions.
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

Compose a summary of the user's current commitments. Used both in-conversation (user asks "show commitments") and by the `commitment-digest` mission for scheduled delivery.

## Gathering data

1. `memory_tree("commitments/open/", depth=1)` — list all open commitment files (skip README.md)
2. `memory_read` each file to extract frontmatter: status, urgency, due, delegated_to, resolution_path, stale_after, tags
3. `memory_tree("commitments/signals/pending/", depth=1)` — count pending signals (skip README.md)
4. `memory_tree("commitments/resolved/", depth=1)` — count recently resolved

Use `memory_tree` and `memory_read` for digest assembly. Do not use CodeAct,
shell commands, or creative-generation tools to count or summarize
commitments.

## Composing the digest

Group commitments and present in this order:

```
## Commitments — <today's date>

### Overdue / Critical
- **<title>** (due <date>) — owner: <owner>
  → <resolution suggestion based on resolution_path>

### Due This Week
- **<title>** (due <date>) — owner: <owner>, delegated to: <person>

### Waiting / Delegated
- **<title>** — waiting on <person> since <date>
  → (follow-up suggested) if not updated in 3+ days

### Open (no deadline)
- **<title>** — owner: <owner>

### Agent Can Handle
- **<title>** — I can <suggested approach>. Want me to proceed?

### Pending Signals (<count>)
<count> unprocessed signals. Say "review signals" to triage them.

### Recently Resolved
- <title> (resolved <date>)

---
Did I miss anything? Tell me if I overlooked an obligation.
```

**Rules:**
- Omit empty sections entirely
- Keep each item to one line plus optional resolution suggestion
- If zero open commitments and zero pending signals: "No open commitments. You're clear."
- Flag commitments past `stale_after` as "(stale — still relevant?)"
- For `resolution_path: agent_can_handle`, describe what the agent would do and ask for approval
- For `resolution_path: needs_reply`, offer to draft a response if possible
- For delegated items past 3 days without update: "(follow-up suggested)"
- Always end with "Did I miss anything?" to catch false negatives

## Delivery

- **In conversation:** Display the digest inline
- **From mission:** Send via `message` tool to the user's preferred channel
- **Tone:** Concise and scannable — a dashboard, not a narrative
