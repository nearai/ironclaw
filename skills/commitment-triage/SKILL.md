---
name: commitment-triage
version: 0.2.0
description: Recognize obligations in conversation, extract signals with immediacy and expiration, create and manage commitments in the workspace.
activation:
  # Limits enforced by `ironclaw_skills::types::ActivationCriteria::enforce_limits`:
  # MAX_KEYWORDS_PER_SKILL=20, MAX_PATTERNS_PER_SKILL=5. The lists below are
  # intentionally trimmed to those caps; entries past the cap are silently
  # dropped at parse time.
  keywords:
    - need to
    - have to
    - must do
    - promised
    - committed to
    - deadline
    - by friday
    - by tomorrow
    - follow up
    - get back to
    - remind me
    - track this
    - mark done
    - commitment
    - obligation
    - overdue
    - slack message from
    - asked me to review
    - can you review
    - this week
  patterns:
    - "(?i)I (need|have|should|must|ought) to"
    - "(?i)(remind me|don't let me forget|make sure I)"
    - "(?i)(by|before|until) (monday|tuesday|wednesday|thursday|friday|saturday|sunday|tomorrow|tonight|end of)"
    - "(?i)(promised|committed|agreed) (to|that)"
    - "(?i)(slack|email|dm|text) message from .+: .+"
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

Treat these as especially strong passive-signal cases even if the user does
not say "track this":
- "The strategy team asked me to review the international expansion reforecast this week."
- "Legal asked me to comment on the draft by Thursday."
- "Slack message from Priya: can you review the OAuth callback edge case this week?"
- "I need to review Sarah's deck before Friday."

These should usually become `review`, `reply`, or `follow-up` signals with
`immediacy: prompt` when the request is time-sensitive or comes from a named
person/team.

Treat inbound-message phrasings like these as strong passive signals too:
- "Slack message from strategy: can you review the international expansion reforecast this week?"
- "Email from legal: can you comment on the draft by Thursday?"

**Action:**
1. Check for duplicates: `memory_search` for key phrases within `commitments/`
2. If no duplicate, call `memory_write` with:
   - `target`: `commitments/signals/pending/<slug>.md`
   - `append`: false
   - Content: signal frontmatter + description
3. Only after the `memory_write` succeeds, at a natural pause briefly note:
   "I've tracked a commitment about [topic]."

Do not merely acknowledge or summarize an obligation. This mode is successful
only if a signal is actually written to `commitments/signals/pending/`.

Do NOT interrupt the conversation flow. Signal extraction is a side-effect.
For commitment tracking, use `memory_tree`, `memory_read`, and `memory_write`.
Do not use CodeAct, shell commands, or creative-generation tools unless the
user explicitly asked for execution rather than tracking.

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
- `prompt`: urgent DMs from key people, trending topics (for creators), time-sensitive requests, or named-person/team asks like "the strategy team asked me to review..."
- `batch`: most obligations — meeting action items, reports to read, tasks with multi-day deadlines

**Signal destinations (set during triage, not initial extraction):**
- `commitment`: actionable, tracked → promote to `commitments/open/`
- `parked_idea`: interesting but not now → write to `commitments/parked-ideas/`
- `intelligence`: informational, shapes future decisions → write a durable MemoryDoc via `memory_write` to a non-commitments path (e.g. `context/intel/<slug>.md`)
- `dismissed`: not relevant

## Mode B: Explicit capture

When the user explicitly asks to track something: "track this", "add a commitment", "I committed to X".

If the user says "track this separately", "track this too", or otherwise
introduces a second distinct obligation, create a new commitment file for that
new item. Do not overwrite or silently reuse the previous commitment unless it
is clearly the same obligation.

Phrasings like "track this request from Slack", "track this request from email",
or "track this review request" are explicit capture requests. They should go
through Mode B and produce a persisted commitment or signal write, not just a
summary response.

Example:
- User: "Track this: Sarah is going to deliver the Q2 budget proposal by Friday."
- User later: "Track this separately: Bob is drafting the acquisition term sheet by Tuesday next week."
- Required behavior: create a second distinct `memory_write` for Bob's term
  sheet. Do not only confirm it in prose, and do not reuse Sarah's file.

**Action:**
1. Skip the signal stage — write directly to `commitments/open/<slug>.md`
2. Ask for missing details ONLY if truly ambiguous. Infer reasonable defaults.
3. Confirm briefly only after the write succeeds: "Tracked: [description], due [date], urgency [level]."

This mode is only successful if a commitment file is actually written.
For explicit capture, prefer direct `memory_write` updates to the workspace.
Do not switch to CodeAct or shell execution for simple tracking tasks.

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
