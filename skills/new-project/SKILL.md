---
name: new-project
version: 0.1.0
description: Create and structure a new autonomous project — "/new-project <what project does>"
activation:
  keywords:
    - project
    - create project
    - new project
    - set up project
    - project goals
    - project metrics
    - organize work
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
  max_context_tokens: 2500
---

# Project Management

A **project** is an autonomous workspace — a domain of work with its own goals, metrics, missions, knowledge base, and agent instructions. Examples: a company, a development project, a marketing campaign, a research initiative.

## Creating a Project

Use `project_create` to create a new project:

```
project_create(name: "Acme Corp", description: "Managing Acme Corp operations", goals: ["Hit $1M ARR by Q3", "Launch mobile app by June"])
```

After creation, set up the workspace:

1. **Write AGENTS.md** — project-specific agent instructions:
   ```
   memory_write(target: "projects/acme-corp/AGENTS.md", content: "# Acme Corp\n\nYou are managing operations for Acme Corp...\n\n## Key context\n- B2B SaaS company\n- 50 employees\n- Main product: ...")
   ```

2. **Add initial knowledge** — store relevant files under `projects/{slug}/`:
   ```
   memory_write(target: "projects/acme-corp/context.md", content: "## Current state\n...")
   ```

3. **Define metrics** with evaluation instructions:
   ```
   project_update(id: "<project-id>", metrics: [
     {"name": "Monthly Revenue", "unit": "USD", "target": 83333, "evaluation": "Check projects/acme-corp/revenue.md for latest figures"},
     {"name": "Active Users", "unit": "users", "target": 10000, "evaluation": "Query the analytics dashboard"}
   ])
   ```

4. **Create missions** scoped to the project — always pass `project_id`:
   ```
   mission_create(name: "Revenue tracking", goal: "Check and update revenue metrics weekly", cadence: "weekly", project_id: "<project-id>")
   mission_create(name: "Customer outreach", goal: "Review and respond to support tickets", cadence: "daily", project_id: "<project-id>")
   ```

## Project Structure Convention

```
projects/
  {slug}/
    AGENTS.md          # Project-specific agent instructions (loaded into system prompt)
    context.md         # Background knowledge, current state
    goals.md           # Detailed goal breakdown (optional)
    metrics/           # Metric tracking files (optional)
    research/          # Research and analysis outputs
    reports/           # Generated reports
```

## Key Rules

- **Always pass `project_id`** when creating missions. Without it, missions land in the Default project.
- **AGENTS.md is critical** — it gives the agent project context for every mission run. Include: what the project is, key stakeholders, current priorities, tools/APIs to use, constraints.
- **Metrics need evaluation instructions** — tell the agent *how* to measure each metric (API call, file to read, command to run). Without this, the agent can't track progress.
- **Use `project_list`** to see all projects and their IDs before creating missions.

## Updating a Project

Use `project_update` to modify goals, metrics, name, or description:

```
project_update(id: "<id>", goals: ["Updated goal 1", "New goal 2"])
project_update(id: "<id>", metrics: [{"name": "Revenue", "unit": "USD", "target": 100000, "current": 42000, "evaluation": "..."}])
```

## When to Create a Project vs. Just a Mission

- **Create a project** when: there are multiple related goals, ongoing work that accumulates knowledge, or a distinct domain that needs its own agent instructions.
- **Just create a mission** when: it's a single recurring task that doesn't need its own context (e.g., "check disk space daily").
