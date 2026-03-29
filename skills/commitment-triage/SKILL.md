---
name: commitment-triage
version: 0.2.0
description: Recognize obligations in conversation, extract signals with immediacy and expiration, create and manage commitments in the workspace.
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
1. Check for duplicates: `memory_search` for key phrases within `commitments/`
2. If no duplicate, call `memory_write` with:
   - `target`: `commitments/signals/pending/<slug>.md`
   - `append`: false
   - Content: signal frontmatter + description
3. At a natural pause, briefly note: "I've tracked a commitment about [topic]."

Do NOT interrupt the conversation flow. Signal extraction is a side-effect.

**Signal template:**
```
---
type: signal
source_channel: <current channel>
source_message: "<brief quote>"
detected_at: <today YYYY-MM-DD>
immediacy: <realtime|prompt|batch — see rules below>
expires_at: <YYYY-MM-DD or null>
confidence: <high if explicit obligation, medium if implied, low if ambiguous>
obligation_type: <reply|deliver|attend|review|decide|follow-up|informational>
mentions: [<people mentioned>]
destination: null
promoted_to: null
---
<1-2 sentence description of the detected obligation.>
```

**Immediacy rules:**
- `realtime`: production incidents, security alerts, stop-loss triggers, anything marked urgent by the user. If you detect a realtime signal, send a `message` immediately — do not wait for the next triage run.
- `prompt`: urgent DMs from key people, trending topics (for creators), time-sensitive requests
- `batch`: most obligations — meeting action items, reports to read, tasks with multi-day deadlines

**Signal destinations (set during triage, not initial extraction):**
- `commitment`: actionable, tracked → promote to `commitments/open/`
- `parked_idea`: interesting but not now → write to `commitments/parked-ideas/`
- `intelligence`: informational, shapes future decisions → write a durable MemoryDoc via `memory_write` to a non-commitments path (e.g. `context/intel/<slug>.md`)
- `dismissed`: not relevant

## Mode B: Explicit capture

When the user explicitly asks to track something: "track this", "add a commitment", "I committed to X".

**Action:**
1. Skip the signal stage — write directly to `commitments/open/<slug>.md`
2. Ask for missing details ONLY if truly ambiguous. Infer reasonable defaults.
3. Confirm briefly: "Tracked: [description], due [date], urgency [level]."

**Commitment template:**
```
---
type: commitment
status: open
urgency: <critical|high|medium|low>
due: <YYYY-MM-DD or null>
created_at: <today>
stale_after: <14 days from now, or sooner for urgent items>
owner: <user|agent>
delegated_to: null
resolution_path: <agent_can_handle|needs_reply|needs_decision|note_only>
source_signal: null
resolved_by: null
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

**Resolution path inference:**
- Agent can research, draft, review code, summarize → `agent_can_handle`
- User must reply to a person → `needs_reply`
- User must choose between options → `needs_decision`
- Just tracking awareness → `note_only`

For `agent_can_handle`, note in the commitment body what the agent would do. The agent must NOT act autonomously without user approval — add a note: "I can handle this. Want me to proceed?"

## Mode C: Resolution

When the user says they finished something: "done with X", "finished the review", "sent the reply to Sarah".

**Action:**
1. `memory_tree("commitments/open/", depth=1)` to find the matching commitment
2. `memory_read` the likely match to confirm
3. Write the updated file (status: resolved, resolved_by: user) to `commitments/resolved/<same-slug>.md`
4. Overwrite the original with empty content: `memory_write(target="commitments/open/<slug>.md", content="", append=false)`
5. Confirm: "Resolved: [title]."

## Mode D: Signal promotion (used by triage mission)

When reviewing pending signals (manually via "review signals" or during a triage mission run):
1. `memory_tree("commitments/signals/pending/", depth=1)` to list signals
2. For each, `memory_read` and route to destination:
   - Actionable → create commitment in `commitments/open/`, set signal `destination: commitment`
   - Interesting but not now → write to `commitments/parked-ideas/`, set `destination: parked_idea`
   - Informational → write a MemoryDoc to `context/intel/`, set `destination: intelligence`
   - Not relevant → move to `signals/expired/`, set `destination: dismissed`
3. Update the signal's `promoted_to` field for commitment destinations

## Filename conventions

Slugify: lowercase, hyphens, no special chars, max 50 chars. Examples:
- "Review Sarah's deck" → `review-sarah-deck.md`
- "Submit Q1 tax filing" → `submit-q1-tax-filing.md`
