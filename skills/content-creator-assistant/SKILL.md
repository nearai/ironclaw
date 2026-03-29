---
name: content-creator-assistant
version: 0.1.0
description: Commitment tracking tuned for content creators — content pipeline stages, trend expiration, cross-platform cascades, heavy idea parking.
activation:
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
  max_context_tokens: 2000
---

# Content Creator — Commitment System Setup

You are configuring the commitments system for a content creator. Their day involves:
- Morning: scanning trends and planning
- Midday: creating (writing, filming, recording)
- Afternoon: editing, thumbnails, publishing
- Evening: distribution across platforms, audience engagement
- Ideas arrive constantly and most won't be executed immediately

## Step 1: Ask configuration questions

1. **Timezone and channel**: What timezone? Which channel for digests?
2. **Platforms**: Which platforms do you publish to? (YouTube, TikTok, Instagram, Twitter, blog, podcast, etc.)
3. **Content cadence**: How often do you publish? (daily, 2-3x/week, weekly)
4. **Sponsored content**: Do you have sponsored/partner content with hard deadlines?
5. **Trend sensitivity**: How quickly do trends expire for your niche? (hours, days)

## Step 2: Create workspace structure

Check if `commitments/README.md` exists. If not, create the full workspace structure (same as commitment-setup). Additionally create:

```
memory_write(target="commitments/content-pipeline/README.md", content="# Content Pipeline\n\nEach content piece gets its own file tracking its lifecycle:\nidea → research → script → create → edit → thumbnail → publish → distribute → engage\n\nFiles: commitments/content-pipeline/<slug>.md", append=false)
```

## Step 3: Create tuned routines

### Triage routine — trend-aware, fast expiration

```
routine_create(
  name: "commitment-triage",
  description: "Creator triage — expire stale trends, track content pipeline, surface sponsored deadlines",
  prompt: "You are triaging commitments for a content creator. Read commitments/README.md for the schema. Priority order: (1) Sponsored content with hard deadlines — flag anything due within 3 days. (2) Content pipeline items in commitments/content-pipeline/ — check for stalled stages (not updated in 2+ days). (3) Trend-related signals — expire after 6 hours if not promoted (trends move fast). Non-trend signals expire after 48 hours as normal. (4) Check parked-ideas/ for ideas that might be timely now. (5) Append triage summary to commitments/triage-log.md. (6) Alert if any sponsored deadlines are approaching.",
  request: { kind: "cron", schedule: "0 8,14,20 * * *", timezone: "<user_timezone>" },
  execution: { mode: "lightweight", use_tools: true, max_tool_rounds: 6, context_paths: ["commitments/README.md"] }
)
```

Three runs: morning (planning), afternoon (mid-create check), evening (distribution check).

### Digest routine — pipeline-focused

```
routine_create(
  name: "commitment-digest",
  description: "Creator digest — content pipeline status, upcoming deadlines, fresh ideas",
  prompt: "Compose a content creator digest. Read commitments/README.md for schema. Sections: (1) CONTENT IN PROGRESS — list items from commitments/content-pipeline/ with their current stage and days since last update. (2) SPONSORED DEADLINES — any commitments tagged 'sponsored' with due dates. (3) PUBLISHING QUEUE — items in 'publish' or 'distribute' stage. (4) FRESH IDEAS — count of parked ideas, highlight any high-relevance ones parked in the last week. (5) ENGAGEMENT TASKS — any commitments about responding to comments, collaborations, etc. Keep it visual and scannable. Send via message tool.",
  request: { kind: "cron", schedule: "0 8 * * *", timezone: "<user_timezone>" },
  execution: { mode: "lightweight", use_tools: true, max_tool_rounds: 6, context_paths: ["commitments/README.md"] }
)
```

### Idea capture routine — weekly resurface

```
routine_create(
  name: "creator-idea-resurface",
  description: "Weekly resurfacing of parked content ideas",
  prompt: "Review parked ideas for the content creator. Read all files in commitments/parked-ideas/ via memory_tree and memory_read. For ideas parked more than 2 weeks ago, compose a brief list asking if they are still interesting. For high-relevance ideas, suggest promoting them to the content pipeline. Send the list via message tool. If no parked ideas exist, skip silently.",
  request: { kind: "cron", schedule: "0 10 * * MON", timezone: "<user_timezone>" },
  execution: { mode: "lightweight", use_tools: true, max_tool_rounds: 5, context_paths: ["commitments/README.md"] }
)
```

## Step 4: Write calibration memories

```
memory_write(
  target: "commitments/calibration.md",
  content: "# Content Creator Calibration\n\n- Content pieces are tracked as pipeline items in commitments/content-pipeline/, not as plain commitments\n- Pipeline stages: idea → research → script → create → edit �� thumbnail → publish → distribute → engage\n- When user publishes on one platform, automatically create commitments for distribution to other platforms: <platforms list>\n- Trend-related signals expire after 6 hours — if it is not acted on quickly, it is stale\n- Sponsored content is always urgency=critical when due within 3 days\n- Ideas flow constantly — park liberally, promote selectively\n- Parked ideas are resurfaced weekly on Monday mornings\n- When a new content piece starts, create a pipeline file with all stages as unchecked items",
  append: false
)
```

Replace `<platforms list>` with the platforms the user listed in Step 1.

## Step 5: Explain cross-platform cascades

Tell the user: "When you publish a piece, tell me and I'll automatically create distribution commitments for your other platforms. For example, 'published the API video on YouTube' will create commitments for TikTok clip, Instagram reel, Twitter thread, etc."

## Step 6: Confirm

> Your content creator system is ready:
> - **Triage** runs 3x daily (8am, 2pm, 8pm) — trend signals expire in 6h, sponsored deadlines flagged at 3 days
> - **Morning digest** at 8am — pipeline status, deadlines, publishing queue, fresh ideas
> - **Idea resurface** every Monday morning — reviews parked ideas older than 2 weeks
> - Pipeline tracking in `commitments/content-pipeline/` — each piece tracks idea through engagement
> - Say **"new content piece: [title]"** to start a pipeline, or **"park this idea"** to save for later
