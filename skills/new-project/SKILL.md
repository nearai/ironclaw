---
name: new-project
version: 0.2.0
description: Create and structure a new autonomous project — "/new-project <what project does>"
activation:
  keywords:
    - project
    - create project
    - new project
    - set up project
    - autonomous workspace
    - campaign
    - department
    - company project
  patterns:
    - "create a (new )?project"
    - "set up.*project"
    - "organize.*into.*project"
    - "new.project"
    - "/new.project"
  tags:
    - project-management
    - organization
    - goals
  max_context_tokens: 2000
---

# New Project

Create an autonomous project workspace using `memory_write` and `mission_create`.

## Step-by-step procedure

Given the user's description of what the project does, derive a short slug (lowercase, hyphens, e.g. `ai-research`). Then execute these steps **sequentially** (one tool call at a time — do NOT batch calls that depend on each other):

### 1. Write AGENTS.md

```
memory_write(target: "projects/{slug}/AGENTS.md", content: "# {Project Name}\n\n{What the agent should know about this project: domain, stakeholders, priorities, constraints, tools/APIs to use.}")
```

This file is loaded into the system prompt for every mission in this project. Make it specific and actionable.

### 2. Write context.md

```
memory_write(target: "projects/{slug}/context.md", content: "# {Project Name} — Context\n\n## Overview\n{What the project is and why it exists.}\n\n## Current State\n{What is known so far.}")
```

### 3. Write goals.md (if the project has clear goals)

```
memory_write(target: "projects/{slug}/goals.md", content: "# Goals\n\n- Goal 1\n- Goal 2\n...")
```

Include measurable targets when possible. If the project would benefit from tracked metrics, add a metrics section:

```
## Metrics

| Metric | Unit | Target | How to measure |
|--------|------|--------|----------------|
| {name} | {unit} | {target} | {evaluation instruction — tell the agent HOW to check this: API call, file to read, command to run} |
```

### 4. Create missions

Create recurring missions scoped to the project. Use the **slug** as `project_id`:

```
mission_create(name: "...", goal: "...", cadence: "daily", project_id: "{slug}")
```

Choose appropriate cadences: `hourly`, `daily`, `weekly`, `monthly`, or cron expressions like `0 9 * * 1-5`.

## Project structure convention

```
projects/
  {slug}/
    AGENTS.md      # Agent instructions (loaded into system prompt)
    context.md     # Background knowledge, current state
    goals.md       # Goal breakdown with optional metrics
    research/      # Research and analysis outputs
    reports/       # Generated reports
```

## Rules

- Execute tool calls **one at a time**, sequentially. Wait for each result before the next call.
- Always pass `project_id` when creating missions. Without it, missions land in the Default project.
- AGENTS.md must be written first — it gives the agent project context.
- Keep the response concise. After setup, summarize what was created in a short list.
