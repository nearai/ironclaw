---
name: tech-debt-tracker
version: 0.1.0
description: Detect and track technical debt from conversation and PR review comments. Resurface in weekly retros, promote to commitments when ready to fix.
activation:
  keywords:
    - tech debt
    - technical debt
    - hack
    - hacky
    - refactor later
    - fixme
    - workaround
    - shortcut
    - should refactor
    - bandaid
    - temporary fix
    - show tech debt
    - debt backlog
  patterns:
    - "(?i)(this|that) is (a |kind of )?(hack|workaround|bandaid|band-aid|shortcut|kludge)"
    - "(?i)(we |I )should (refactor|clean up|rewrite|fix) (this|that) (later|eventually|someday)"
    - "(?i)(show|list|review) (tech )?debt"
    - "(?i)add.*tech ?debt"
  tags:
    - commitments
    - developer
    - tech-debt
  max_context_tokens: 1200
---

# Tech Debt Tracker

Track technical debt from conversation and PR review comments. Debt items live in `commitments/tech-debt/` and are resurfaced weekly.

## Mode A: Passive detection

When the user says something implying tech debt ("this is a hack but it works", "we should really refactor the auth module", "adding another TODO"), silently extract it.

**Action:**
1. Check for duplicates via `memory_search` in `commitments/tech-debt/`.
2. Write to `commitments/tech-debt/<slug>.md`:

```
---
type: tech-debt
detected_at: <today YYYY-MM-DD>
repo: <current repo context if known, else null>
severity: <high|medium|low>
category: <refactor|performance|security|testing|documentation|architecture>
source: conversation
source_pr: null
---
# <Brief title>
<What the debt is and why it exists.>

## Context
<What prompted the shortcut — deadline, complexity, missing knowledge.>

## Proposed fix
<What a proper fix would look like, if discussed.>
```

**Severity rules:**
- `high`: security shortcuts, data integrity risks, architectural violations
- `medium`: code quality, missing tests, poor abstractions
- `low`: style issues, minor workarounds, documentation gaps

3. At a natural pause: "Tracked tech debt: <brief description>."

Do NOT interrupt conversation flow.

## Mode B: PR review scan (used by triage mission)

When the triage mission processes recently merged PRs, it should scan review comments for tech-debt patterns:
- "can be addressed in follow-up work"
- "not blocking but should fix later"
- "TODO: address in next PR"
- "leaving for now", "good enough for now"
- "nit: ... (not blocking)"
- "tech debt", "hack", "workaround" in review comments

For each match, create a tech-debt item with `source: pr-review` and `source_pr: owner/repo#123`.

## Mode C: Explicit capture

User says: "add tech debt: the caching layer needs TTL eviction."

Write directly to `commitments/tech-debt/`, confirm briefly.

## Mode D: Listing

User says: "show tech debt" or "debt backlog"

**Action:**
1. `memory_tree("commitments/tech-debt/", depth=1)` — list files (skip README.md)
2. `memory_read` each for title, severity, detected_at, repo, source
3. Display grouped by severity, then age:

```
## Tech Debt Backlog

### High Severity
- **<title>** (repo: <repo>, <N> days old, source: <conversation|PR #X>) — <category>

### Medium Severity
- ...

### Low Severity
- ...

---
<count> total. <count> chronic (30+ days).
Say "resolve <title>" to mark fixed, or "promote <title>" to create a commitment.
For large items, use `/plan <description>` to create a structured fix plan.
```

## Mode E: Resolution

User says: "resolved the caching TTL debt"

Move to `commitments/resolved/` with original type preserved. Confirm.

## Mode F: Promotion to commitment

User says: "let's fix the auth refactor" or "promote the caching debt"

Create commitment in `commitments/open/` with `tags: [tech-debt]` and `resolution_path: agent_can_handle` if it's a code task. For complex items, suggest: "This looks like a multi-step refactor. Use `/plan <description>` to create a structured fix plan."

## Filename conventions

Slugify: lowercase, hyphens, max 50 chars. Prefix with repo slug if known:
- "Auth module needs refactor" in nearai/ironclaw → `nearai-ironclaw-auth-refactor.md`
- Generic debt → `caching-ttl-eviction.md`
