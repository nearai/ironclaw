# Workflow Mission Templates

Replace `{{...}}` placeholders before use. Use `mission_create` with `name`, `goal`, and `cadence`.

## 1) Issue → Plan

```json
{
  "name": "wf-issue-plan-{{slug}}",
  "goal": "For issue #{{issue_number}} in {{repository}}, produce a concrete implementation plan with milestones, edge cases, and tests. Post/update an issue comment with the plan.",
  "cadence": "event:issue.opened"
}
```

Filter: `repository_name: "{{repository}}"`

## 2) Maintainer Comment Gate

Create one per maintainer handle, or use a shared convention.

```json
{
  "name": "wf-maintainer-gate-{{slug}}-{{maintainer}}",
  "goal": "Read the maintainer comment on {{repository}} and decide: update plan or start/continue implementation. If plan changes are requested, edit the plan artifact first. If implementation is requested, continue on the feature branch and update PR status/comment.",
  "cadence": "event:pr.comment.created"
}
```

Filter: `repository_name: "{{repository}}"`, `comment_author: "{{maintainer}}"`

## 3) PR Monitor Loop

```json
{
  "name": "wf-pr-monitor-{{slug}}",
  "goal": "For PRs in {{repository}}, collect open review comments and unresolved threads, apply fixes, push branch updates, and summarize remaining blockers. If conflict with {{main_branch}}, rebase/merge from origin/{{main_branch}} and resolve safely.",
  "cadence": "event:pr.synchronize"
}
```

Filter: `repository_name: "{{repository}}"`

## 4) CI Failure Fix Loop

```json
{
  "name": "wf-ci-fix-{{slug}}",
  "goal": "Find failing check details for PRs in {{repository}}, implement minimal safe fixes, rerun or await CI, and post concise status updates. Prioritize deterministic and test-backed fixes.",
  "cadence": "event:ci.check_run.completed"
}
```

Filter: `repository_name: "{{repository}}"`, `ci_conclusion: "failure"`

## 5) Staging Batch Review

Skip if no staging branch is configured.

```json
{
  "name": "wf-staging-review-{{slug}}",
  "goal": "Every cycle: list ready PRs in {{repository}}, merge ready ones into {{staging_branch}}, run deep correctness analysis in batch, fix discovered issues on affected branches, ensure CI green, then merge {{staging_branch}} into {{main_branch}} if clean.",
  "cadence": "0 */{{batch_interval_hours}} * * *"
}
```

## 6) Post-Merge Learning → Memory

```json
{
  "name": "wf-learning-{{slug}}",
  "goal": "From merged PRs in {{repository}}, extract preventable mistakes, reviewer themes, CI failure causes, and successful patterns. Write/update a shared memory doc at context/intel/{{slug}}-learnings.md with actionable rules to reduce cycle time and regressions.",
  "cadence": "event:pr.closed"
}
```

Filter: `repository_name: "{{repository}}"`, `pr_merged: "true"`

## Synthetic Event Test

Use with `event_emit` after mission install:

```json
{
  "event_source": "github",
  "event_type": "issue.opened",
  "payload": {
    "repository_name": "{{repository}}",
    "issue_number": 99999,
    "sender_login": "test-bot"
  }
}
```
