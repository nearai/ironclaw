---
name: idea-parking
version: 0.1.0
description: Park interesting ideas for later consideration, resurface them periodically, and promote to commitments when ready.
activation:
  keywords:
    - park this
    - save for later
    - interesting idea
    - maybe someday
    - backburner
    - idea for later
    - might want to
    - parked ideas
    - idea pool
    - not now but
    - revisit later
    - shelve this
  patterns:
    - "(?i)(park|save|shelf|shelve|backburner) (this|that|the) (idea|thought|concept)"
    - "(?i)not (now|yet|ready) but"
    - "(?i)(show|list|review) parked (ideas|items)"
    - "(?i)activate (idea|parked)"
  tags:
    - commitments
    - ideas
  max_context_tokens: 1000
---

# Idea Parking

Manage ideas that are interesting but not yet actionable. Parked ideas live in `projects/commitments/parked-ideas/` and are periodically resurfaced by the digest.

## Parking an idea

User says: "park this idea: do a comparison video of AWS vs GCP" or "save for later: investigate Rust WASM performance."

This skill is only successful if the parked idea is actually persisted.
Do not just acknowledge or summarize an idea.

Execution order is mandatory:
1. Call `memory_write` for `projects/commitments/parked-ideas/<slug>.md`
2. Only then confirm what was parked

Never say an idea was "parked", "saved for later", or "resurfaced later"
unless the corresponding `memory_write` succeeded.

**Action:**
1. Write to `projects/commitments/parked-ideas/<slug>.md`:

```
---
type: parked-idea
parked_at: <today YYYY-MM-DD>
source: conversation
relevance: <high|medium|low — infer from user's enthusiasm>
tags: [<relevant tags>]
---
# <Idea title>
<Description and why it is interesting.>

## Activation trigger
<What would make this worth pursuing — a condition, event, or timeframe.>
```

2. Confirm only after the write succeeds: "Parked: <title>. I'll resurface it when the time seems right."

## Listing parked ideas

User says: "show parked ideas" or "what's on the backburner?"

**Action:**
1. `memory_tree("projects/commitments/parked-ideas/", depth=1)` — list all files (skip README.md)
2. `memory_read` each to get title and relevance
3. Display a brief list:
   ```
   Parked ideas:
   - **<title>** (parked <date>, relevance: <level>)
   - **<title>** (parked <date>, relevance: <level>)
   ```

## Promoting an idea

User says: "activate the comparison video idea" or "let's do the WASM performance investigation."

**Action:**
1. Find the matching parked idea
2. Create a commitment in `projects/commitments/open/` based on the idea content
3. Overwrite the parked idea file with empty content
4. Confirm: "Promoted to active commitment: <title>."

## Dismissing an idea

User says: "dismiss the comparison video idea" or "drop that parked idea."

**Action:**
1. Overwrite the file with empty content
2. Confirm: "Dismissed: <title>."
