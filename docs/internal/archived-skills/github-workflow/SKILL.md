---
name: github-workflow
version: 0.1.0
description: Install and operate a full GitHub issue-to-merge workflow for any repository using event-driven and cron missions. Handles issue planning, PR monitoring, CI fixing, staging review, and post-merge learning.
activation:
  keywords:
    - github workflow
    - CI automation
    - PR automation
    - automate repo
    - issue to merge
    - workflow setup
    - github automation
  patterns:
    - "(?i)(set ?up|install|enable).*(workflow|automation) for .+/.+"
    - "(?i)automate (this |my )?(repo|repository|PRs|CI)"
    - "(?i)github (workflow|automation|pipeline)"
  tags:
    - developer
    - github
    - automation
    - workflow
  max_context_tokens: 2000
requires:
  skills:
    - github
---

# GitHub Workflow Automation

Install and maintain a complete issue-to-merge workflow for any GitHub repository. Maps GitHub webhook events and cron schedules into automated missions for planning, implementation, review, CI fixing, staging validation, and post-merge learning.

## Workflow

1. Gather project parameters from `projects/<owner>-<repo>/project.md` or ask the user.
2. Verify runtime prerequisites.
3. Install or update mission set from templates.
4. Run a dry test with `event_emit`.
5. Monitor outcomes and tune prompts/filters.

## Parameters

Read these from the project file in workspace, or collect from the user:
- `repository`: `owner/repo` (required)
- `maintainers`: GitHub handles allowed to trigger implement/replan actions
- `staging_branch`: default `staging` (or null if no staging workflow)
- `main_branch`: default `main`
- `batch_interval_hours`: default `8`

## Prerequisites

Before installing missions, verify:
- GitHub skill authenticated (for issue/PR/comment/status operations).
- GitHub webhook delivery configured to `POST /webhook/tools/github`.
- Webhook HMAC secret configured in the secrets store as `github_webhook_secret`.
- Events can also be emitted via `event_emit` tool for testing.

## Install Procedure

1. Open [`workflow-routines.md`](references/workflow-routines.md).
2. For each template block:
   - Replace placeholders (`{{repository}}`, `{{maintainers}}`, branch names)
   - Namespace mission names with the repo slug (e.g. `wf-issue-plan-nearai-ironclaw`)
   - Call `mission_create` with `name`, `goal` (the prompt), and `cadence` (cron expression or `event:<pattern>`)
3. If a mission already exists (check `mission_list`), update rather than duplicate.
4. If `staging_branch` is null, skip `wf-staging-batch-review`.
5. Write installation status to `projects/<owner>-<repo>/project.md`.
6. Confirm install with `mission_list`.

## Mission Set

Install these missions per repository:
- `wf-issue-plan-<slug>`: on `issue.opened` or `issue.reopened`, generate implementation plan comment.
- `wf-maintainer-gate-<slug>`: on maintainer comments, decide update-plan vs start implementation.
- `wf-pr-monitor-<slug>`: on PR open/sync/review-comment/review, address feedback and refresh branch.
- `wf-ci-fix-<slug>`: on CI status/check failures, apply fixes and push updates.
- `wf-staging-review-<slug>`: every N hours, review ready PRs, merge into staging, run batch correctness analysis, fix findings, then merge staging to main.
- `wf-learning-<slug>`: on merged PRs, extract mistakes/lessons and write to shared memory.

## Event Filters

Use top-level filters for stability:
- `repository_name` (e.g., `owner/repo`)
- `sender_login`, `comment_author`
- `issue_number`, `pr_number`
- `ci_status`, `ci_conclusion`
- `review_state`, `pr_merged`

## Operating Rules

- All implementation work on non-main branches.
- PR loop must resolve both human and AI review comments.
- On conflicts with origin/main, refresh branch before continuing.
- Staging-batch is the only path for bulk correctness verification before mainline merge.
- Memory update runs only after successful merge.

## Validation

After install, run:
1. `event_emit` with a synthetic `issue.opened` payload for the target repo.
2. Confirm at least one mission fired.
3. Check corresponding mission status.
4. Confirm no unrelated missions fired.
