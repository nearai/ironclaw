# Workflow Routine Templates

Replace `{{...}}` placeholders before use.

## 1) Issue -> Plan

```json
{
  "name": "wf-issue-plan",
  "description": "Create implementation plan when a new issue arrives",
  "trigger_type": "system_event",
  "event_source": "github",
  "event_type": "issue.opened",
  "event_filters": {
    "repository_name": "{{repository}}"
  },
  "action_type": "full_job",
  "prompt": "For issue #{{issue_number}} in {{repository}}, produce a concrete implementation plan with milestones, edge cases, and tests. Post/update an issue comment with the plan.",
  "cooldown_secs": 30
}
```

## 2) Maintainer Comment Gate (Update Plan vs Implement)

Trigger per-maintainer by creating one routine per handle, or maintain a shared author convention.

```json
{
  "name": "wf-maintainer-comment-gate-{{maintainer}}",
  "description": "React to maintainer guidance comments on issues/PRs",
  "trigger_type": "system_event",
  "event_source": "github",
  "event_type": "pr.comment.created",
  "event_filters": {
    "repository_name": "{{repository}}",
    "comment_author": "{{maintainer}}"
  },
  "action_type": "full_job",
  "prompt": "Read the maintainer comment and decide: update plan or start/continue implementation. If plan changes are requested, edit the plan artifact first. If implementation is requested, continue on the feature branch and update PR status/comment.",
  "cooldown_secs": 20
}
```

## 3) PR Monitor Loop

```json
{
  "name": "wf-pr-monitor-loop",
  "description": "Keep PR healthy: address review comments and refresh branch",
  "trigger_type": "system_event",
  "event_source": "github",
  "event_type": "pr.synchronize",
  "event_filters": {
    "repository_name": "{{repository}}"
  },
  "action_type": "full_job",
  "prompt": "For PR #{{pr_number}}, collect open review comments and unresolved threads, apply fixes, push branch updates, and summarize remaining blockers. If conflict with {{main_branch}}, rebase/merge from origin/{{main_branch}} and resolve safely.",
  "cooldown_secs": 20
}
```

## 4) CI Failure Fix Loop

```json
{
  "name": "wf-ci-fix-loop",
  "description": "Fix failing CI checks on active PRs",
  "trigger_type": "system_event",
  "event_source": "github",
  "event_type": "ci.check_run.completed",
  "event_filters": {
    "repository_name": "{{repository}}",
    "ci_conclusion": "failure"
  },
  "action_type": "full_job",
  "prompt": "Find failing check details for PR #{{pr_number}}, implement minimal safe fixes, rerun or await CI, and post concise status updates. Prioritize deterministic and test-backed fixes.",
  "cooldown_secs": 20
}
```

## 5) Staging Batch Review (Every 8h)

```json
{
  "name": "wf-staging-batch-review",
  "description": "Batch correctness review through staging, then merge to main",
  "trigger_type": "cron",
  "schedule": "0 0 */{{batch_interval_hours}} * * *",
  "action_type": "full_job",
  "prompt": "Every cycle: list ready PRs, merge ready ones into {{staging_branch}}, run deep correctness analysis in batch, fix discovered issues on affected branches, ensure CI green, then merge {{staging_branch}} into {{main_branch}} if clean.",
  "cooldown_secs": 120
}
```

## 6) Post-Merge Learning -> Common Memory

```json
{
  "name": "wf-learning-memory",
  "description": "Capture merge learnings into shared memory",
  "trigger_type": "system_event",
  "event_source": "github",
  "event_type": "pr.closed",
  "event_filters": {
    "repository_name": "{{repository}}",
    "pr_merged": "true"
  },
  "action_type": "full_job",
  "prompt": "From merged PR #{{pr_number}}, extract preventable mistakes, reviewer themes, CI failure causes, and successful patterns. Write/update a shared memory doc with actionable rules to reduce cycle time and regressions.",
  "cooldown_secs": 30
}
```

## Optional: Synthetic Event Test

```json
{
  "source": "github",
  "event_type": "issue.opened",
  "payload": {
    "repository_name": "{{repository}}",
    "issue_number": 99999,
    "sender_login": "test-bot"
  }
}
```

Use with `event_emit` after routine install.
