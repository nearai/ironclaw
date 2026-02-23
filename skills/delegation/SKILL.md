---
name: delegation
version: 0.1.0
description: Helps users delegate tasks, break them into steps, set deadlines, and track progress via routines and memory.
activation:
  keywords:
    - delegate
    - hand off
    - assign task
    - help me with
    - take care of
    - remind me to
    - schedule
    - plan my
    - manage my
    - track this
  patterns:
    - "can you.*handle"
    - "I need (help|someone) to"
    - "take over"
    - "set up a reminder"
    - "follow up on"
  tags:
    - personal-assistant
    - task-management
    - delegation
  max_context_tokens: 1500
---

# Task Delegation Assistant

When the user wants to delegate a task or get help managing something, follow this process:

## 1. Clarify the Task

Ask what needs to be done, by when, and any constraints. Get enough detail to act independently but don't over-interrogate. If the request is clear, skip straight to planning.

## 2. Break It Down

Decompose the task into concrete, actionable steps. Use `memory_write` to persist the task plan to a path like `tasks/{task-name}.md` with:
- Clear description
- Steps with checkboxes
- Due date (if any)
- Status: pending/in-progress/done

## 3. Set Up Tracking

If the task is recurring or has a deadline:
- Create a routine using `routine_create` for scheduled check-ins
- Add a heartbeat item if it needs daily monitoring
- Set up an event-triggered routine if it depends on external input

## 4. Use Profile Context

Check `USER.md` for the user's preferences:
- **Proactivity level**: High = check in frequently. Low = only report on completion.
- **Communication style**: Match their preferred tone and detail level.
- **Focus areas**: Prioritize tasks that align with their stated goals.

## 5. Execute or Queue

- If you can do it now (search, draft, organize, calculate), do it immediately.
- If it requires waiting, external action, or follow-up, create a reminder routine.
- If it requires tools you don't have, explain what's needed and suggest alternatives.

## 6. Report Back

Always confirm the plan with the user before starting execution. After completing, update the task file in memory and notify the user with a concise summary.

## Communication Guidelines

- Be direct and action-oriented
- Confirm understanding before acting on ambiguous requests
- When in doubt about autonomy level, ask once then remember the answer
- Use `memory_write` to track delegation preferences for future reference
