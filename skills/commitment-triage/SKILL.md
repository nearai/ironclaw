---
name: commitment-triage
version: 0.1.0
description: Recognize obligations in conversation, extract signals, create and manage commitments in the workspace.
activation:
  keywords:
    - need to
    - have to
    - must do
    - promise
    - promised
    - committed to
    - deadline
    - by friday
    - by tomorrow
    - by end of
    - follow up
    - get back to
    - remind me
    - track this
    - mark done
    - done with
    - commitment
    - obligation
    - overdue
    - owe them
  patterns:
    - "(?i)I (need|have|should|must|ought) to"
    - "(?i)(remind me|don't let me forget|make sure I)"
    - "(?i)(by|before|until) (monday|tuesday|wednesday|thursday|friday|saturday|sunday|tomorrow|tonight|end of)"
    - "(?i)(promised|committed|agreed) (to|that)"
    - "(?i)(track|add|log) (this|that|a) (commitment|task|obligation)"
  exclude_keywords:
    - setup commitments
    - install commitments
  tags:
    - commitments
    - task-management
    - personal-assistant
  max_context_tokens: 2000
---

# Commitment Triage

You have a commitments tracking system in the workspace under `commitments/`. Read `commitments/README.md` for the full schema if you need field details.

## Mode A: Passive signal detection

When the user says something that implies an obligation, promise, or deadline — but is NOT explicitly asking you to track it — silently extract a signal.

**Triggers:** "I need to...", "I promised Sarah...", "I should get back to...", "The report is due Friday", "They asked me to review..."

**Action:**
1. Check for duplicates first: `memory_search` for key phrases from the obligation within `commitments/`
2. If no duplicate, call `memory_write` with:
   - `target`: `commitments/signals/pending/<slug>.md`
   - `append`: false
   - Content: signal frontmatter + description (see schema below)
3. At a natural pause, briefly note: "I've tracked a commitment about [topic]."

Do NOT interrupt the conversation flow. The signal extraction is a side-effect, not the main response.

**Signal template:**
```
---
type: signal
source_channel: <current channel>
source_message: "<brief quote>"
detected_at: <today YYYY-MM-DD>
confidence: <high if explicit obligation, medium if implied, low if ambiguous>
obligation_type: <reply|deliver|attend|review|decide|follow-up>
mentions: [<people mentioned>]
promoted_to: null
---
<1-2 sentence description of the detected obligation.>
```

## Mode B: Explicit capture

When the user explicitly asks to track something: "track this", "add a commitment", "I committed to X".

**Action:**
1. Skip the signal stage — write directly to `commitments/open/<slug>.md`
2. Ask for missing details ONLY if truly ambiguous (due date, urgency). If the user gave enough context, infer reasonable defaults.
3. Confirm briefly: "Tracked: [description], due [date], urgency [level]."

**Commitment template:**
```
---
type: commitment
status: open
urgency: <critical|high|medium|low>
due: <YYYY-MM-DD or null>
created_at: <today>
owner: user
delegated_to: null
source_signal: null
tags: [<inferred tags>]
---
# <Title>
<Description.>

## Resolution path
- [ ] <Step 1>
- [ ] <Step 2>
```

**Urgency rules:**
- `critical`: due today or overdue
- `high`: due within 3 days
- `medium`: due within 2 weeks or soon but no hard deadline
- `low`: no deadline, whenever

## Mode C: Resolution

When the user says they finished something: "done with X", "finished the review", "sent the reply to Sarah".

**Action:**
1. `memory_tree("commitments/open/", depth=1)` to find the matching commitment
2. `memory_read` the likely match to confirm
3. Write the updated file (status: resolved, checked-off steps) to `commitments/resolved/<same-slug>.md`
4. Overwrite the original with empty content: `memory_write(target="commitments/open/<slug>.md", content="", append=false)`
5. Confirm: "Resolved: [title]. Nice work."

## Mode D: Signal promotion (used by triage routine)

When reviewing pending signals (either manually via "review signals" or during a triage routine run):
1. `memory_tree("commitments/signals/pending/", depth=1)` to list signals
2. For each, `memory_read` and assess: does this warrant a commitment?
3. If yes: create commitment in `commitments/open/`, update the signal's `promoted_to` field
4. If no: move to `commitments/signals/expired/`

## Filename conventions

Slugify: lowercase, hyphens, no special chars, max 50 chars. Examples:
- "Review Sarah's deck" → `review-sarah-deck.md`
- "Submit Q1 tax filing" → `submit-q1-tax-filing.md`
