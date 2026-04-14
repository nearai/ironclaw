---
name: content-creator-setup
version: 0.3.0
description: One-time onboarding for the content creator workflow — content pipeline stages, trend expiration, cross-platform cascades, heavy idea parking. After successful setup this skill is excluded from selection until the marker file is deleted.
activation:
  setup_marker: commitments/.content-creator-setup-complete
  keywords:
    - content creator
    - creator assistant
    - youtube workflow
    - content pipeline
    - publishing schedule
    - creator setup
    - content calendar
  patterns:
    - "(?i)I'm a (content creator|youtuber|creator|streamer|podcaster|blogger)"
    - "(?i)set ?up.*(content|creator|publishing|video)"
    - "(?i)help me manage my (content|videos|publications|posts)"
  tags:
    - commitments
    - content-creation
    - publishing
    - setup
  max_context_tokens: 2500
requires:
  skills:
    - commitment-triage
    - commitment-digest
    - decision-capture
    - idea-parking
---

# Content Creator — Commitment System Setup

You are configuring the commitments system for a content creator. Their day involves:
- Morning: scanning trends and planning
- Midday: creating (writing, filming, recording)
- Afternoon: editing, thumbnails, publishing
- Evening: distribution across platforms, audience engagement
- Ideas arrive constantly and most won't be executed immediately

## Persistence rules

For creator workflows, persistence comes before presentation.

- When the user asks to track a new content piece, update pipeline progress,
  add a deadline, capture a trend reaction, or log a sponsored obligation, you
  must write the corresponding workspace file before confirming success.
- Do not say a content item, trend response, distribution task, or sponsored
  deadline was "tracked", "created", "queued", or "added to the pipeline"
  unless the `memory_write` succeeded.
- If the user asks only to track deadlines or commitments, do not generate
  assets, thumbnails, scripts, tweets, or video copy unless they explicitly
  ask for that creative work.
- Trend reactions like "create a short take on React compiler tonight" should
  normally become active commitments in `commitments/open/`, not just prose or
  a suggestion.

Concrete examples:
- "Track this commitment: create a short take on the React compiler trend tonight."
  Required behavior: write an active commitment under `commitments/open/` before confirming.
- "Track this: the sponsored Figma workflow video has to ship by Friday."
  Required behavior: write a sponsored commitment under `commitments/open/`.
- "Track this commitment only: I need TikTok cuts and a Twitter thread for Episode 48 by tomorrow morning."
  Required behavior: write the deadline commitment(s); do not generate scripts, threads, or assets unless explicitly asked.

## Companion skills

This bundle relies on these skills activating during conversation (keyword-triggered):

| Skill | Activates when | What it does |
|---|---|---|
| `commitment-triage` | User mentions obligations, deadlines | Extracts signals, creates/resolves commitments |
| `commitment-digest` | User asks "show commitments" | Composes formatted summary |
| `decision-capture` | User makes a decision | Records decision with rationale |
| `idea-parking` | User says "park this idea" | Parks ideas for weekly resurfacing |

If any are missing from `skills/`, tell the user which ones are needed.

## Step 1: Ask configuration questions

1. **Platforms**: Which platforms do you publish to? (YouTube, TikTok, Instagram, Twitter, blog, podcast, etc.)
2. **Content cadence**: How often do you publish? (daily, 2-3x/week, weekly)
3. **Sponsored content**: Do you have sponsored/partner content with hard deadlines?
4. **Trend sensitivity**: How quickly do trends expire for your niche? (hours, days)

## Step 2: Create workspace structure

1. Check if `commitments/README.md` exists via `memory_read`. If it does, skip to creating the content-pipeline directory.
2. Write `commitments/README.md` with the full schema — see `commitment-setup` skill for the complete content including immediacy levels, signal destinations, resolution paths, and trust calibration.
3. Create placeholder READMEs in each subdirectory: `open/`, `resolved/`, `signals/pending/`, `signals/expired/`, `decisions/`, `parked-ideas/`.
4. Create the content-pipeline directory:

```
memory_write(target="commitments/content-pipeline/README.md", content="# Content Pipeline\n\nEach content piece gets its own file tracking its lifecycle:\nidea → research → script → create → edit → thumbnail → publish → distribute → engage\n\nFiles: commitments/content-pipeline/<slug>.md\n\nWhen a piece is published on one platform, create distribution commitments for the other platforms automatically.", append=false)
```

## Step 3: Create tuned missions

### Triage mission — trend-aware, fast expiration

```
mission_create(
  name: "commitment-triage",
  goal: "Creator triage. Read commitments/README.md for schema. Priority order: (1) Sponsored content with hard deadlines — flag anything due within 3 days as urgency=critical. (2) Content pipeline items in commitments/content-pipeline/ — check for stalled stages (not updated in 2+ days). (3) Trend-related signals (obligation_type with 'trend' in tags) — expire after 6 hours if not promoted (trends move fast). Non-trend signals expire after 48 hours. (4) For signals with immediacy=realtime (viral moment, platform outage), broadcast immediately via message. (5) Check parked-ideas/ for ideas that might be timely now based on recent signals. (6) Route informational signals (industry news, competitor moves) to intelligence via MemoryDoc. (7) Append triage summary to commitments/triage-log.md. (8) Alert if any sponsored deadlines are approaching.",
  cadence: "0 8,14,20 * * *"
)
```

### Digest mission — pipeline-focused

```
mission_create(
  name: "commitment-digest",
  goal: "Creator digest. Read commitments/README.md for schema. Sections: (1) CONTENT IN PROGRESS — list items from commitments/content-pipeline/ with their current stage and days since last update. Flag stalled items. (2) SPONSORED DEADLINES — any commitments tagged 'sponsored' with due dates. (3) PUBLISHING QUEUE — items in 'publish' or 'distribute' stage. For agent_can_handle items (scheduling posts, writing descriptions), offer to proceed. (4) FRESH IDEAS — count of parked ideas, highlight high-relevance ones from the last week. (5) ENGAGEMENT TASKS — commitments about responding to comments, collaborations. End with 'Did I miss anything?' Send via message tool.",
  cadence: "0 8 * * *"
)
```

### Idea resurface mission — weekly

```
mission_create(
  name: "creator-idea-resurface",
  goal: "Weekly parked ideas review for content creator. Read all files in commitments/parked-ideas/ via memory_tree and memory_read. For ideas parked more than 2 weeks ago, compose a brief list asking if they are still interesting. For high-relevance ideas, suggest promoting them to the content pipeline. If any parked ideas align with recent trending signals, highlight the match. Send the list via message tool. If no parked ideas exist, skip silently.",
  cadence: "0 10 * * 1"
)
```

## Step 4: Write calibration memories

```
memory_write(
  target: "commitments/calibration.md",
  content: "# Content Creator Calibration\n\n- Content pieces are tracked as pipeline items in commitments/content-pipeline/, not as plain commitments\n- Pipeline stages: idea → research → script → create → edit → thumbnail → publish → distribute → engage\n- When user publishes on one platform, automatically create commitments for distribution to other platforms: <platforms list>\n- Trend-related signals expire after 6 hours — if not acted on quickly, they are stale\n- Sponsored content is always urgency=critical when due within 3 days\n- Ideas flow constantly — park liberally, promote selectively\n- Parked ideas are resurfaced weekly on Monday mornings\n- When a new content piece starts, create a pipeline file with all stages as unchecked items\n- For agent_can_handle items (scheduling, descriptions, thumbnails), ask permission before proceeding\n- Start conservative: surface everything, learn preferences over time",
  append: false
)
```

Replace `<platforms list>` with the platforms the user listed in Step 1.

## Step 5: Confirm

> Your content creator system is ready:
> - **Triage** runs 3x daily (8am, 2pm, 8pm) — trend signals expire in 6h, sponsored deadlines flagged at 3 days
> - **Morning digest** at 8am — pipeline status, deadlines, publishing queue, fresh ideas
> - **Idea resurface** every Monday morning — reviews parked ideas older than 2 weeks
> - Pipeline tracking in `commitments/content-pipeline/` — each piece tracks idea through engagement
> - Cross-platform cascades: tell me when you publish and I'll create distribution commitments
> - Say **"new content piece: [title]"** to start a pipeline, or **"park this idea"** to save for later

## Step 6: Mark setup complete

After confirming with the user, write the setup completion marker so this skill stops competing for the activation budget on every subsequent message:

```
memory_write(
  path: "commitments/.content-creator-setup-complete",
  content: "# Content Creator Setup Complete\n\nCompleted: <today's UTC date>\n\nMissions installed: creator-triage, creator-digest, creator-idea-resurface"
)
```

To re-trigger setup, delete `commitments/.content-creator-setup-complete` first.
