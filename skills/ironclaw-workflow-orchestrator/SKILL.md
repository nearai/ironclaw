---
name: ironclaw-workflow-orchestrator
description: "Install and operate a full GitHub issue-to-merge workflow in IronClaw using event-driven and cron routines. Use when setting up or tuning autonomous project orchestration: issue intake, planning, maintainer feedback handling, branch/PR execution, CI/comment follow-up, batched staging review every 8 hours, and memory updates from merge outcomes."
---

# IronClaw Workflow Orchestrator

## Overview
Use this skill to install and maintain a complete project workflow as routines, not core code changes. It maps GitHub webhook events plus scheduled checks into plan/update/implement/review/merge loops with explicit staging-batch analysis.

## Workflow
1. Gather workflow parameters.
2. Verify runtime prerequisites.
3. Install or update routine set from templates.
4. Run a dry test with `event_emit`.
5. Monitor outcomes and tune prompts/filters.

## Parameters
Collect these values before creating routines:
- `repository`: `owner/repo` (required)
- `maintainers`: GitHub handles allowed to trigger implement/replan actions
- `staging_branch`: default `staging`
- `main_branch`: default `main`
- `batch_interval_hours`: default `8`
- `implementation_label`: default `autonomous-impl`

## Prerequisites
Before installing routines, verify:
- Routines system enabled.
- GitHub tool authenticated (for issue/PR/comment/status operations).
- GitHub webhook delivery configured to `POST /webhook/tools/github`.
- Webhook HMAC secret configured in the secrets store as `github_webhook_secret` (required for GitHub webhook delivery).
- Events can also be emitted via `event_emit` tool calls for testing or when webhook ingestion is not yet configured.

## Install Procedure
1. Open [`workflow-routines.md`](references/workflow-routines.md).
2. For each template block:
- replace placeholders (`{{repository}}`, `{{maintainers}}`, branch names)
- call `routine_create`
3. If a routine already exists:
- use `routine_update` instead of creating duplicates
- keep names stable so long-lived metrics/history stay intact
4. Confirm install with `routine_list` and `routine_history`.

## Routine Set
Install these routines:
- `wf-issue-plan`: on `issue.opened` or `issue.reopened`, generate implementation plan comment/checklist.
- `wf-maintainer-comment-gate`: on maintainer comments, decide update-plan vs start implementation.
- `wf-pr-monitor-loop`: on PR open/sync/review-comment/review, address feedback and refresh branch.
- `wf-ci-fix-loop`: on CI status/check failures, apply fixes and push updates.
- `wf-staging-batch-review`: every 8h, review ready PRs, merge into staging, run deep batch correctness analysis, fix findings, then merge staging -> main.
- `wf-learning-memory`: on merged PRs, extract mistakes/lessons and write to shared memory.

## Event Filters
Prefer top-level filters for stability:
- `repository_name` (string, e.g. `owner/repo`)
- `sender_login` (string)
- `issue_number` / `pr_number`
- `ci_status`, `ci_conclusion`
- `review_state`, `comment_author`

Use narrow filters to avoid accidental triggers across repos.

## Operating Rules
- All implementation work must occur on non-main branches.
- PR loop must resolve both human and AI review comments.
- On conflicts with `origin/main`, refresh branch before continuing.
- Staging-batch routine is the only path for bulk correctness verification before mainline merge.
- Memory update routine runs only after successful merge.

## Validation
After install, run:
1. `event_emit` with a synthetic `issue.opened` payload for the target repo.
2. Confirm at least one routine fired.
3. Check corresponding `routine_history` entries.
4. Confirm no unrelated routines fired.

## When To Update Templates
Update this skill when:
- GitHub event names/payload fields change.
- Team review policy changes (e.g., staging cadence, maintainer gates).
- New CI policy requires different failure routing.
