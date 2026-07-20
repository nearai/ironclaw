---
name: routine-advisor
version: 0.1.0
description: Suggests relevant cron routines based on user context, goals, and observed patterns
activation:
  keywords:
    - every day
    - every morning
    - every week
    - routine
    - automate
    - remind me
    - check daily
    - monitor
    - recurring
    - schedule
    - habit
    - workflow
    - keep forgetting
    - always have to
    - repetitive
    - notifications
    - digest
    - review daily
    - weekly review
  patterns:
    - "I (always|usually|often|regularly) (check|do|look at|review)"
    - "every (morning|evening|week|day|monday|friday)"
    - "I (wish|want) (I|it) (could|would) (automatically|auto)"
    - "is there a way to (auto|schedule|set up)"
    - "can you (check|monitor|watch|track).*for me"
    - "I keep (forgetting|missing|having to)"
  tags:
    - automation
    - scheduling
    - personal-assistant
    - productivity
  max_context_tokens: 1500
---

# Routine Advisor

When the conversation suggests the user has a repeatable task or could benefit from automation, consider suggesting a routine.

## When to Suggest

Suggest a routine when you notice:
- The user describes doing something repeatedly ("I check my PRs every morning")
- The user mentions forgetting recurring tasks ("I keep forgetting to...")
- The user asks you to do something that sounds periodic
- You've learned enough about the user to propose a relevant automation
- The user has installed extensions that enable new monitoring capabilities

Do not suggest or create a routine when the user asks for a one-time answer or says to do something now, right now, immediately, or ASAP without also asking for scheduling or recurrence.

## How to Suggest

Be specific and concrete. Not "Want me to set up a routine?" but rather: "I noticed you review PRs every morning. Want me to create a daily 9am routine that checks your open PRs and sends you a summary?"

Always include:
1. What the routine would do (specific action)
2. When it would run (specific schedule in plain language)
3. How it would notify them (which channel they're on)

Wait for the user to confirm before creating.

## Pacing

- First 1-3 conversations: Do NOT suggest routines. Focus on helping and learning.
- After learning 2-3 user patterns: Suggest your first routine. Keep it simple.
- After 5+ conversations: Suggest more routines as patterns emerge.
- Never suggest more than 1 routine per conversation unless the user is clearly interested.
- If the user declines, wait at least 3 conversations before suggesting again.

## Creating Routines

Use the `builtin__trigger_create` capability. Before creating, check `builtin__trigger_list` to avoid duplicates.

Parameters:
- `name`: Short human-readable routine name
- `prompt`: Clear, specific instruction for the full task each fire performs. Never tell the prompt to send results back to the requesting user — each fire's final reply is delivered automatically. "Send me the result" style asks are delivery routing, not a task step: satisfy them with `delivery_target_id` alone, and never write a send-to-requester step into the prompt, even with the requester's own conversation ID pinned. When the task itself is to message someone else or post somewhere (for example "send Firat a joke every morning"), that belongs in the prompt: resolve the exact recipient conversation ID while creating the routine (while the user is present to confirm) and pin that ID in the prompt, so a fire never has to look a recipient up by name.
- `delivery_target_id`: Optional. Routes THIS routine's results to a specific outbound delivery target (an id from `builtin__outbound_delivery_targets_list`). Set it whenever the user names a destination for the routine's results; when omitted, results go to the user's default outbound delivery target at fire time.
- `schedule`: An object — `{"kind": "cron", "expression": "0 9 * * *", "timezone": "America/New_York"}` for recurring, or `{"kind": "once", "at": "2026-07-01T09:00:00", "timezone": "America/New_York"}` for one-time. The timezone is required (IANA name). Common cron schedules:
  - Daily 9am: `0 9 * * *`
  - Weekday mornings: `0 9 * * MON-FRI`
  - Weekly Monday: `0 9 * * MON`
  - Every 2 hours during work: `0 9-17/2 * * MON-FRI`
  - Sunday evening: `0 18 * * SUN`

## Delivering Results

Routine results are delivered automatically — never re-send a routine result to the requesting user with a messaging capability (it would arrive twice: once from the host, once from you). Messaging a third party is different: when that is the routine's task, the fire performs it with the messaging capability as instructed by the pinned prompt.

If the user asks for a routine's results on a specific product or channel (a Slack DM, a Slack channel, ...): call `builtin__outbound_delivery_targets_list`, then pass the chosen id as `delivery_target_id` on `builtin__trigger_create` — that routes only this routine. Use `builtin__outbound_delivery_target_set` only when the user wants to change their user-wide default (it re-routes replies and every routine without its own `delivery_target_id`). If neither a per-routine target nor a default is set, routine results are not delivered anywhere.

## Routine Ideas by User Type

**Developer:**
- Daily PR review digest (check open PRs, summarize what needs attention)
- CI/CD failure alerts (monitor build status)
- Weekly dependency update check
- Daily standup prep (summarize yesterday's work from daily logs)

**Professional:**
- Morning briefing (today's priorities from memory + any pending tasks)
- End-of-day summary (what was accomplished, what's pending)
- Weekly goal review (check progress against stated goals)
- Meeting prep reminders

**Health/Personal:**
- Daily exercise or habit check-in
- Weekly meal planning prompt
- Monthly budget review reminder

**General:**
- Daily news digest on topics of interest
- Weekly reflection prompt (what went well, what to improve)
- Periodic task/reminder check-in
- Regular cleanup of stale tasks or notes
- Weekly profile evolution (if the user has a profile in `context/profile.json`, suggest a Monday routine that reads the profile via `memory_read`, searches recent conversations for new patterns with `memory_search`, and updates the profile via `memory_write` if any fields should change with confidence > 0.6 — be conservative, only update with clear evidence)

## Awareness

Before suggesting, consider what tools and extensions are currently available. Only suggest routines the agent can actually execute. If a routine would need a tool that isn't installed, mention that too: "If you connect your calendar, I could also send you a morning briefing with today's meetings."
