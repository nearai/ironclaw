---
description: Triage open GitHub issues — split into bugs vs features, rank by severity/opportunity, and flag under-specified issues
disable-model-invocation: true
allowed-tools: Bash(gh issue list:*), Bash(gh issue view:*), Bash(gh api:*), Bash(git log:*), Read, Grep, Glob, Task
argument-hint: "[--label=<filter>] [--milestone=<filter>]"
---

# Issue Triage

You are triaging all open issues on this repository. Your job is to split them into **bugs** and **feature requests**, rank each group, assess how well-specified each issue is, and produce an actionable triage report.

## Step 1: Fetch all open issues

Fetch every open issue with metadata:

```
gh issue list --state open --limit 200 --json number,title,author,labels,assignees,createdAt,updatedAt,body,commentsCount,reactionGroups,milestone
```

If `$ARGUMENTS` contains `--label=<X>`, append `--label '<X>'` to the command. If it contains `--milestone=<X>`, append `--milestone '<X>'` to the command.

Also fetch recently closed issues (last 14 days) to detect duplicates and already-resolved work:

```
gh issue list --state closed --search "closed:>=$(date -v-14d +%Y-%m-%d)" --limit 100 --json number,title,body,labels,closedAt
```

**Exclude pull requests** — `gh issue list` may include PRs. Fetch open PR numbers to filter them out:

```
gh pr list --state open --json number --jq '.[].number'
```

Remove any issue whose number appears in this list.

## Step 2: Classify each issue as Bug or Feature

Read each issue's title, body, and labels to classify it into one of these categories:

### Bugs
Issues that describe **broken existing behavior** — something that worked or should work but doesn't. Signals:
- Labels: `bug`, `defect`, `regression`, `crash`, `error`
- Title/body keywords: "broken", "fails", "crash", "panic", "error", "regression", "doesn't work", "unexpected behavior"
- Includes reproduction steps or error output
- References existing functionality not working as documented

### Feature Requests
Issues that describe **new or enhanced behavior** — something that doesn't exist yet. Signals:
- Labels: `enhancement`, `feature`, `feature-request`, `improvement`, `proposal`
- Title/body keywords: "add", "support", "implement", "would be nice", "proposal", "RFC", "new"
- Describes a capability the project doesn't have
- Proposes a design or API change

### Ambiguous
If an issue doesn't clearly fit either category (e.g., "improve X performance" could be a bug or a feature), classify it as **Ambiguous** and note why.

## Step 3: Rate issue detail level

For each issue, assess how well-specified it is on a 3-tier scale:

| Detail Level | Criteria |
|-------------|----------|
| **Well-specified** | Has clear description of what/why, reproduction steps (bugs) or user story (features), acceptance criteria or expected behavior, and enough context to start working immediately |
| **Adequate** | Describes the problem or request clearly, but missing some detail — no repro steps, vague acceptance criteria, or unclear scope. Needs 1-2 clarifying questions before work can start |
| **Under-specified** | Vague title-only or single-sentence body, no context on why it matters, no clear definition of done. Needs significant discussion before it's actionable |

Indicators of good specification:
- Code snippets, error logs, or screenshots
- Steps to reproduce (bugs)
- Proposed API/behavior (features)
- Links to related issues or discussions
- Clear "done when" criteria

## Step 4: Rank bugs by severity

Score each bug on these dimensions and compute an overall severity rank:

### Impact (1-4)
| Score | Level | Description |
|-------|-------|-------------|
| 4 | **Critical** | Data loss, security vulnerability, complete feature broken, crash in common path |
| 3 | **High** | Major feature degraded, workaround exists but painful, affects many users |
| 2 | **Medium** | Minor feature broken, easy workaround, affects subset of users |
| 1 | **Low** | Cosmetic, edge case, documentation error, minor inconvenience |

### Urgency (1-3)
| Score | Level | Description |
|-------|-------|-------------|
| 3 | **Urgent** | Security issue, regression in recent release, blocking other work |
| 2 | **Normal** | Should be fixed in next release cycle |
| 1 | **Low** | Fix when convenient, backlog-worthy |

### Scope (1-3)
| Score | Level | Description |
|-------|-------|-------------|
| 3 | **Broad** | Affects core path, multiple modules, or all users |
| 2 | **Moderate** | Affects one module or a specific configuration |
| 1 | **Narrow** | Affects edge case or single obscure path |

**Bug severity score** = Impact × 2 + Urgency + Scope (base max 14)

Apply a one-time +2 boost if any of the following are true (max 16):
- Has a linked PR already (someone is working on it — fast-track review)
- Is labeled `security`
- Is a regression (worked before, broken now)

## Step 5: Rank features by opportunity

Score each feature request on these dimensions:

### Value (1-4)
| Score | Level | Description |
|-------|-------|-------------|
| 4 | **High** | Unlocks new use cases, frequently requested, strategic alignment |
| 3 | **Medium-High** | Significant quality-of-life improvement, good user demand signals |
| 2 | **Medium** | Nice to have, modest improvement to existing workflow |
| 1 | **Low** | Marginal value, niche use case, unclear demand |

Look for value signals in the issue:
- Number of thumbs-up reactions or "+1" comments
- Multiple people asking for the same thing
- Alignment with project roadmap (check CLAUDE.md TODOs)
- Unblocks other features or simplifies architecture

### Effort estimate (1-3, inverted — lower effort = higher score)
| Score | Level | Description |
|-------|-------|-------------|
| 3 | **Small** | <1 day, isolated change, clear implementation path |
| 2 | **Medium** | 1-3 days, touches a few modules, some design needed |
| 1 | **Large** | 3+ days, cross-cutting, needs RFC or architectural discussion |

### Readiness (1-3)
| Score | Level | Description |
|-------|-------|-------------|
| 3 | **Ready** | Well-specified, implementation path clear, no blockers |
| 2 | **Almost ready** | Needs minor clarification, but scope is understood |
| 1 | **Not ready** | Needs design discussion, has open questions, blocked by other work |

**Opportunity score** = Value × 2 + Effort + Readiness (base max 14)

Apply a one-time +2 boost if any of the following are true (max 16):
- A community member offered to implement it
- It has a linked draft PR
- It closes a gap listed in the project's "Current Limitations / TODOs"

## Step 6: Detect duplicates and relationships

Check for:
- **Duplicates** — Issues describing the same bug or requesting the same feature (compare titles and bodies)
- **Related clusters** — Groups of issues around the same area (e.g., multiple workspace issues, multiple CLI issues)
- **Already fixed** — Open issues that may have been resolved by recently closed issues or merged PRs
- **Blockers** — Issues that reference other issues as prerequisites ("depends on #N", "blocked by #N")
- **Epic candidates** — Multiple small issues that could be grouped under a single tracking issue

## Step 7: Produce the triage report

Present the output in this format:

### Quick Stats

```
Open: N | Bugs: N | Features: N | Ambiguous: N
Well-specified: N | Adequate: N | Under-specified: N
Unassigned: N | Stale (>30d): N
```

---

### Critical Bugs (Severity 12+)

Bugs that need immediate attention. For each:

| # | Title | Severity | Impact | Detail | Age | Assignee |
|---|-------|----------|--------|--------|-----|----------|

Include a 1-line summary of the root cause if discernible from the issue.

### High-Priority Bugs (Severity 8-12)

Same table format. These should be addressed in the next release cycle.

### Medium/Low Bugs (Severity <8)

Compact table, sorted by severity descending.

---

### Quick Wins (Opportunity 12+ AND Effort = Small)

Features that are high-value and low-effort — do these first. For each:

| # | Title | Opportunity | Value | Effort | Detail | Age |
|---|-------|-------------|-------|--------|--------|-----|

### High-Opportunity Features (Opportunity 10+)

Same table format. Worth investing in.

### Backlog Features (Opportunity <10)

Compact table, sorted by opportunity descending.

---

### Under-Specified Issues (Need Clarification)

Issues rated "Under-specified" that can't be triaged effectively. For each, suggest 1-2 specific questions to ask the author to make it actionable.

| # | Title | Type | What's missing |
|---|-------|------|---------------|

### Ambiguous Issues (Bug or Feature?)

Issues that couldn't be clearly classified. For each, explain the ambiguity and suggest which category it likely belongs in.

---

### Duplicates & Overlaps

Groups of issues that appear to be duplicates or closely related. Recommend which to keep and which to close.

### Already Fixed?

Open issues that may have been resolved by recently closed issues or merged PRs.

### Stale Issues (>30 days, no activity)

Issues with no updates in 30+ days. Recommend: close, ping author, or keep.

---

### By Area

Group all issues by the area of the codebase they affect (infer from title/body/labels):

| Area | Bugs | Features | Top Priority |
|------|------|----------|-------------|

### Suggested Next Actions

Based on the triage, provide 3-5 concrete recommendations:
1. Which bugs to fix first and why
2. Which quick-win features to pick up
3. Which under-specified issues to clarify
4. Which stale issues to close
5. Any clusters that suggest a larger initiative

## Rules

- Use `gh` CLI for all GitHub operations. Never guess issue state — always check.
- For large issue lists (>20), use the Task tool to parallelize fetching issue details and comments.
- Be concise in summaries. One line per issue in tables.
- When scoring, be honest about uncertainty. If you can't tell severity from the description, say so and rate it conservatively.
- Factor in issue age — older unresolved bugs may indicate they're less critical than they seem, or that they're hard to fix. Note this in your assessment.
- Check comment threads for additional context that the original body may lack. An under-specified issue with rich discussion may actually be well-understood.
- Do NOT post comments, close issues, or take any action. This skill is read-only analysis.
- If the repo has >100 open issues, focus the detailed analysis on the top 30 by recency and engagement (comments + reactions), and provide a summary table for the rest.
