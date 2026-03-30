---
name: learn
version: 0.1.0
description: Manage the learning system — browse extracted skills, review lessons, prune stale knowledge, export learnings, view stats, and add manual entries.
activation:
  keywords:
    - learnings
    - what have you learned
    - show lessons
    - extracted skills
    - learning stats
    - prune learnings
    - export learnings
    - knowledge base
    - what do you know
  patterns:
    - "(?i)(show|list|browse|review) (learnings|lessons|skills|knowledge)"
    - "(?i)(prune|clean|remove) (stale|old) (learnings|knowledge)"
    - "(?i)export (learnings|knowledge|lessons)"
    - "(?i)learning (stats|statistics|status)"
    - "(?i)what have you (learned|extracted|remembered)"
  tags:
    - learning
    - knowledge-management
  max_context_tokens: 2000
---

# Learning Management

Manage the knowledge the agent has accumulated from completed work. The v2 engine automatically extracts skills, lessons, and insights from threads — this skill lets the user browse, curate, and export that knowledge.

## Naming conventions

The engine uses title prefixes to identify doc types:
- `skill:<name>` — extracted skills (e.g. `skill:github-issue-triage`)
- `pattern:<name>` — positive learning patterns (e.g. `pattern:search-before-write`)
- `insight:<category>:<topic>` — conversation insights (e.g. `insight:preference:concise-output`)
- Lessons and issues use plain titles without prefixes

## Confidence scoring

Every learning doc can have a `confidence` field in metadata (1-10 scale):
- **9-10**: User-stated or verified by reading specific code
- **7-8**: High confidence pattern match, confirmed by user feedback
- **5-6**: Moderate, observed once or twice
- **3-4**: Low confidence, may be false positive
- **1-2**: Speculation, likely to be pruned

Confidence affects retrieval ranking — lower confidence docs score lower. User-stated docs (`"source": "user_stated"`) never decay regardless of age.

## False positive history

Dismissed findings are logged to `context/fp-history.md`. Review skills should check this file before flagging issues to avoid re-flagging known FPs.

Use these prefixes when searching via `memory_search`.

## Commands

### Show learnings

User says: "what have you learned?", "show lessons", "browse learnings"

**Action:**
1. Search for each doc type separately to get clean results:
   - `memory_search("skill:")` — extracted skills (titles use `skill:<name>` convention)
   - `memory_search("lesson")` — lessons from error diagnosis
   - `memory_search("insight:")` — conversation insights (titles use `insight:<category>:<topic>`)
2. Also check `memory_tree("context/intel/", depth=2)` for intelligence docs written by commitments/retros
3. Group results by type:

```
## Knowledge Base

### Extracted Skills (<count>)
Skills automatically extracted from successful work sessions.
- **<skill title>** — <description> (extracted <date>, used <N> times)

### Lessons Learned (<count>)
Hard-won lessons from errors and corrections.
- **<lesson title>** — <summary> (learned <date>)

### Intelligence (<count>)
Durable knowledge from decisions, retros, and observations.
- **<intel title>** — <summary> (captured <date>)

### Insights (<count>)
Patterns detected from conversation analysis.
- **<insight title>** — <summary> (detected <date>)
```

### Stats

User says: "learning stats", "knowledge base stats"

**Action:**
1. `memory_search("lesson")` + `memory_search("skill:")` + `memory_search("insight:")` to count by type
2. Check `memory_tree("context/intel/")` for intelligence docs
3. Present:

```
## Learning Stats

- Extracted skills: <count>
- Lessons learned: <count>
- Intelligence docs: <count>
- Conversation insights: <count>
- Total knowledge entries: <count>

Oldest entry: <date>
Most recent: <date>

Top skill by usage: <name> (<N> uses, <success_rate>% success)
```

### Prune stale knowledge

User says: "prune learnings", "clean up stale knowledge"

**Action:**
1. Search for all learning-related docs
2. Identify stale candidates:
   - Skills with 0 usage and created > 60 days ago
   - Lessons that reference files/functions no longer in the codebase (if verifiable)
   - Intelligence docs older than 6 months
3. Present candidates and ask for confirmation before deleting:

```
## Stale Knowledge Candidates

These entries may no longer be relevant:

1. **<title>** — <reason it's stale> (created <date>)
   → [keep] [remove]

2. **<title>** — <reason> (created <date>)
   → [keep] [remove]

Remove selected items? (yes/no/remove 1,3,5)
```

### Export to workspace

User says: "export learnings", "save knowledge to file"

**Action:**
1. Gather all skills, lessons, insights
2. Format as structured markdown grouped by category
3. Write to `context/learnings-export.md` via `memory_write`
4. Confirm: "Exported <N> knowledge entries to context/learnings-export.md"

The export format:

```markdown
# Learnings Export — <date>

## Skills
### <skill name>
- **Description:** <desc>
- **Activation:** <keywords>
- **Extracted from:** thread <id> on <date>
- **Usage:** <N> times, <success_rate>% success

<skill content>

---

## Lessons
### <lesson title>
- **Source:** <thread/mission>
- **Learned:** <date>

<lesson content>

---

## Intelligence
### <intel title>
- **Captured:** <date>

<content>
```

### Add manual learning

User says: "remember that...", "add a lesson:", "I want you to know that..."

**Action:**
1. Classify: is this a lesson (mistake to avoid), a preference (how user likes things), or intelligence (fact about the domain)?
2. Write as appropriate doc type via `memory_write`:
   - Lesson → `memory_write(target="context/lessons/<slug>.md", ...)`
   - Preference → append to `MEMORY.md` or `USER.md`
   - Intelligence → `memory_write(target="context/intel/<slug>.md", ...)`
3. Set metadata: `"source": "user_stated"` for preferences (these never decay in retrieval), `"source": "observed"` for lessons
4. Set initial confidence: `"confidence": 9` for user-stated (high — user said it directly), `"confidence": 6` for observed
5. Confirm: "Noted: <summary>. Stored as <type>."

### Confirm or dismiss a learning (confidence recalibration)

User says: "that finding was correct", "that was a false positive", "good catch", "wrong about X"

**Action: Confirming (boosting confidence):**
1. Find the referenced learning via `memory_search`
2. Read its current confidence from metadata (default 10 if not set)
3. Boost: `new_confidence = min(current + 2, 10)`
4. Rewrite the doc with updated confidence via `memory_write` (append=false)
5. Confirm: "Boosted confidence on '<title>' to <new>/10."

**Action: Dismissing (lowering confidence):**
1. Find the referenced learning
2. Lower: `new_confidence = max(current - 3, 1)`
3. Rewrite with updated confidence
4. If confidence drops below 3, suggest removing: "Confidence is now <new>/10. Remove this learning?"
5. Log the dismissal to `context/fp-history.md`:
   ```
   - <date>: Dismissed "<title>" — reason: <user's reason>
   ```
6. Confirm: "Lowered confidence on '<title>' to <new>/10. Logged as potential false positive."

The FP history at `context/fp-history.md` is checked by review skills (security-review, qa-review) to avoid re-flagging dismissed patterns.

### Review learning quality

User says: "are the learnings helping?", "learning quality"

**Action:**
1. Search for skills with usage metrics in metadata
2. Calculate: how many extracted skills have been used? What's the average success rate?
3. Identify top performers and underperformers
4. Check for positive vs negative learnings (metadata `"positive": true`)
5. Present:

```
## Learning Quality

### High-value learnings (used and successful)
- <skill/lesson> — used <N> times, <success>% success, confidence: <N>/10

### Positive patterns captured
- <pattern> — confidence: <N>/10, observed in <N> threads

### Unused learnings (may be stale)
- <skill/lesson> — 0 uses since extraction <date>, confidence: <N>/10

### Failed learnings (may need correction)
- <skill> — <N> uses, <success>% success (below 50%), confidence: <N>/10
  Consider revising or removing.

### False positive history
- <count> findings dismissed. Check context/fp-history.md for details.
```
