---
name: decision-capture
version: 0.2.0
description: Detect decisions in conversation and record them with rationale, alternatives, and outcome tracking.
activation:
  keywords:
    - decided
    - decision
    - chose
    - going with
    - settled on
    - picked
    - landed on
    - went with
    - finalized
    - agreed on
    - opted for
    - concluded
    - confirmed
    - final answer
    - made the call
    - ruling
    - verdict
    - record decision
  exclude_keywords:
    - undecided
    - considering
    - thinking about
    - tentative
    - maybe
  patterns:
    - "(?i)(we|I|team) (decided|chose|went with|picked|settled on|landed on|opted for)"
    - "(?i)let's go with"
    - "(?i)the (decision|call|verdict) is"
    - "(?i)record (this|that) decision"
    - "(?i)(instead of|replaced|switched to|migrated to)"
  tags:
    - commitments
    - decision-making
  max_context_tokens: 1200
---

# Decision Capture

When a decision is detected in conversation, record it in the commitments workspace. Decisions are durable knowledge — they explain _why_ a path was chosen and enable outcome tracking over time.

## Detection

Look for explicit decision language:
- "We decided to..." / "I'm going with..." / "Let's do X"
- "The call is..." / "We settled on..."
- "I chose X over Y because..."

**Do NOT capture:**
- Brainstorming or hypotheticals: "maybe we should...", "what if we..."
- Preferences without commitment: "I think X is better"
- Questions: "should we go with X?"

When uncertain, ask: "Was that a decision, or still thinking it through?"

## Recording

This skill is only successful if the decision is actually persisted. Do not
just summarize or acknowledge the decision.

Execution order is mandatory:
1. Call `memory_write` for `commitments/decisions/<date>-<slug>.md`
2. If applicable, call `memory_write` for a follow-on commitment in `commitments/open/`
3. If applicable, call `memory_write` for `context/intel/<slug>.md`
4. Only then confirm to the user what was recorded

Never say a decision was "captured", "recorded", or "saved" unless the
corresponding `memory_write` call succeeded.

Write to `commitments/decisions/<date>-<slug>.md` via `memory_write`:

```
---
type: decision
decided_at: <today YYYY-MM-DD>
context: <topic-slug>
participants: [<who was involved>]
confidence: <high if explicit, medium if inferred>
reversible: <true|false>
outcome: null
outcome_positive: null
tags: [<relevant tags>]
---
# <What was decided>

## Context
<Why this decision was needed — 1-2 sentences.>

## Options considered
1. **<Option A>** — <brief pros/cons>
2. **<Option B>** — <brief pros/cons>

## Rationale
<Why this option was chosen.>

## Outcome
<To be filled in later when outcome is known.>
```

## Follow-through

1. If the decision creates an obligation (e.g., "we decided to migrate by Q2"), also create a commitment in `commitments/open/` following the commitment schema.
2. Write an intelligence MemoryDoc to `context/intel/<slug>.md` with a brief summary: "Decided X on <date>. Rationale: <reason>." This makes the decision searchable as durable knowledge.

For explicit requests like "record this decision", "log this decision", or
"note the decision", default to doing all required writes immediately rather
than asking a follow-up question unless the content is genuinely ambiguous.

## Outcome tracking

The triage mission checks for decisions older than 7 days without an outcome. It prompts: "You decided <X> <N> days ago. How did it turn out?" When the user provides an outcome, update the decision file's `outcome` and `outcome_positive` fields.

## Confirmation

After the write(s) succeed, briefly confirm:
- where the decision was written
- whether any follow-on commitment was created
- one-line summary of the rationale
