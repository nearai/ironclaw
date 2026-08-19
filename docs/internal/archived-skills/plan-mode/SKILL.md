---
name: plan-mode
version: 0.1.0
description: Structured planning mode for autonomous task execution. Creates plans as MemoryDocs, executes via Missions, tracks progress with live checklist.
activation:
  keywords:
    - "[PLAN MODE]"
    - plan mode
    - create a plan
    - make a plan
    - execution plan
    - step by step plan
  patterns:
    - "\\[PLAN MODE\\]"
    - "plan (out|how to|before|for)"
  tags:
    - planning
    - autonomous
    - task-management
  max_context_tokens: 2500
---

# Plan Mode

You are in plan mode. Follow these protocols precisely.

## Creating a Plan

When asked to create a plan (message contains [PLAN MODE] Create):

1. **Gather context**: Use `memory_search` for relevant prior work and decisions.
2. **Analyze**: Determine what tools and steps are needed. Consider dependencies and risks.
3. **Write the plan**: Use `memory_write` to save the plan at `plans/<slug>.md` where slug is a short kebab-case name derived from the goal.
4. **Emit checklist**: Call `plan_update` with status "draft" and all steps as "pending".
5. **Present**: Tell the user the plan is ready. Show the steps and say: "Use `/plan approve` to start autonomous execution, or `/plan revise <slug> <feedback>` to adjust."

### Plan Document Format

Write plans to workspace memory in this format:

```
plan_id: <slug>
status: draft

## Goal
<clear statement of what needs to be accomplished>

## Success Criteria
<how to know the plan is complete>

## Steps
1. [ ] Step title -- tools: [tool1, tool2] -- risk: low -- est: 5min
2. [ ] Step title -- tools: [tool3] -- risk: medium -- est: 10min
3. [ ] Step title -- tools: [tool4, tool5] -- risk: low -- est: 5min

## Risks
- Risk description and mitigation strategy

## Progress Log
(updated during execution)
```

### Plan Rules

- Each step MUST specify which tools it needs
- Steps should be independently verifiable
- Include risk assessment (low/medium/high) per step
- Include time estimates per step
- Keep plans under 20 steps; decompose larger work into sub-plans
- Steps should be ordered by dependency (earlier steps enable later ones)

## Approving and Executing a Plan

When asked to approve a plan (message contains [PLAN MODE] Approve):

1. Read the plan from memory using `memory_search` or `memory_read`.
2. Call `mission_create` with:
   - name: `plan:<slug>`
   - goal: The full plan content (goal, steps, success criteria)
   - cadence: `manual`
3. Call `mission_fire` with the mission ID to start execution.
4. Call `plan_update` with status "executing" and the mission_id.
5. Update the plan document status to "executing" via `memory_write`.
6. Tell the user: "Plan execution started. Mission ID: <id>. Check progress with `/plan status <slug>`."

## During Mission Thread Execution

When you are executing as part of a mission thread (your context includes "# Mission:" header with a plan):

1. The plan MemoryDoc is in your project knowledge. Read the steps carefully.
2. Check `current_focus` -- if set, this tells you which step to work on next.
3. Execute the current step using the specified tools.
4. Call `plan_update` to update the checklist:
   - Mark the current step as "completed" with a result summary
   - Mark the next step as "in_progress"
5. Report what you accomplished and what's next.
6. If a step fails:
   - Call `plan_update` marking the step as "failed" with the error
   - Try ONE alternative approach
   - If still failing, call `plan_update` with overall status "failed" and stop

## Checking Plan Status

When asked for plan status (message contains [PLAN MODE] Show status):

1. Search for the plan: `memory_search` for the plan slug or "plan:".
2. If a mission exists, use `mission_list` to check mission state.
3. Summarize: X of Y steps completed, current step, any blockers.
4. Call `plan_update` to refresh the UI checklist.

## Listing Plans

When asked to list plans (message contains [PLAN MODE] List all plans):

1. Use `memory_search` with query "plan" to find plan documents.
2. List each plan with: slug, status, step count, created date.
3. If no plans found, say "No plans found. Use `/plan <description>` to create one."

## Revising a Plan

When asked to revise (message contains [PLAN MODE] Revise):

1. Read the existing plan from memory.
2. Apply the user's feedback to update the steps.
3. Reset any failed/in-progress steps back to pending.
4. Rewrite the plan via `memory_write` (append: false).
5. Call `plan_update` with status "draft" and updated steps.
6. Present the revised plan and suggest `/plan approve` to re-execute.
