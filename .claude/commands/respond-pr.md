---
description: Respond to PR review comments — triage, plan fixes, implement after confirmation, push, and reply to reviewers
disable-model-invocation: true
allowed-tools: Bash(gh pr list:*), Bash(gh pr comment:*), Bash(gh api:*), Bash(gh repo view:*), Bash(git branch:*), Bash(git status:*), Bash(git add:*), Bash(git commit:*), Bash(git push:*), Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*), Read, Edit, Write, Grep, Glob
argument-hint: "[pr-number (optional, auto-detects from branch)]"
---

# Review and Address PR Comments

## Step 1: Find the PR

If `$ARGUMENTS` is provided, use that as the PR number. Otherwise, detect the PR for the current branch:

```
gh pr list --head $(git branch --show-current) --json number,title,url --jq '.[0]'
```

If no PR is found, tell the user and stop.

## Step 2: Fetch all review comments

Resolve the repo owner and name:

```
gh repo view --json owner,name --jq '"\(.owner.login)/\(.name)"'
```

Fetch the full set of review comments (not issue-level comments):

```
gh api --paginate repos/{owner}/{repo}/pulls/{number}/comments
```

Also fetch the review summaries:

```
gh api --paginate repos/{owner}/{repo}/pulls/{number}/reviews
```

Deduplicate comments that appear multiple times (bots sometimes post the same finding under different IDs). Group by the actual issue being raised, not by comment ID.

## Step 3: Triage and plan

For each unique issue raised in the comments:

1. **Check if already addressed** - Read the current code at the referenced location. If a prior commit already fixed it, note it as "already resolved".
2. **Assess validity** - Determine if the comment identifies a real problem or is a false positive. Be honest about false positives but explain why.
3. **Classify severity** - Critical (security/data loss), High (bugs/broken behavior), Medium (correctness/robustness), Low (style/naming/nits).
4. **Plan the fix** - For each valid unresolved issue, describe the specific code change needed.

Present the plan as a table to the user:

| # | Issue | File:Line | Severity | Status | Planned Fix |
|---|-------|-----------|----------|--------|-------------|

Wait for user confirmation before proceeding to implementation.

## Step 4: Implement fixes

After user confirms:

1. Implement each fix in the plan.
2. Run IronClaw's quality gate to verify nothing breaks:
   - `cargo fmt`
   - `cargo clippy --all --benches --tests --examples --all-features`
   - `cargo test --lib`
3. Commit with a descriptive message referencing the PR review.
4. Push to the branch.

## Step 5: Reply to comments

For each comment addressed, reply on the PR with a short message stating what was fixed and the commit SHA. For false positives or already-resolved items, reply explaining why no change was needed.

## Rules

- Never guess at code you haven't read. Always read the referenced file and line before assessing a comment.
- Group duplicate comments (same issue reported by multiple bots) and reply to all of them.
- Do not make changes beyond what the review comments ask for. Stay focused.
- If a comment suggests a change you disagree with, present your reasoning to the user during the planning phase rather than silently ignoring it.
- Follow IronClaw conventions: no `.unwrap()` in production code, use `crate::` imports, `thiserror` errors.
- If changes touch persistence, verify both database backends are updated.
