---
description: Sync latest changes from the upstream fork (nearai/ironclaw) into a new branch and open a PR into staging
allowed-tools: Bash(git fetch:*), Bash(git checkout:*), Bash(git merge:*), Bash(git branch:*), Bash(git push:*), Bash(git log:*), Bash(git diff:*), Bash(git status:*), Bash(git show:*), Bash(git rev-parse:*), Bash(gh pr create:*), Bash(gh repo view:*), Bash(date:*), Read, Grep, Glob
---

# Sync Upstream

Propagate changes from `upstream` (nearai/ironclaw) into a dated branch and open a PR into `staging`.

If `$ARGUMENTS` names a specific upstream branch (e.g. `main`, `develop`), use it. Otherwise default to `main`.

## Step 1: Determine upstream branch

The upstream remote is `upstream` pointing at `nearai/ironclaw.git`. The branch to sync from is `$ARGUMENTS` if provided, otherwise `main`.

Verify the upstream remote exists:

```
git remote get-url upstream
```

If it doesn't exist, stop and tell the user to add it:
```
git remote add upstream https://github.com/nearai/ironclaw.git
```

## Step 2: Fetch upstream

```
git fetch upstream
```

Record the result — how many new commits arrived.

## Step 3: Check for new commits

Compare our `staging` against `upstream/{branch}`:

```
git log staging..upstream/{branch} --oneline
```

If there are no new commits, report that staging is already up to date and stop — there is nothing to do.

Otherwise, report the number of commits and a one-line summary of each.

## Step 4: Create a sync branch

Create a dated branch from the current `staging` HEAD:

```
git checkout -b sync/upstream-$(date +%Y-%m-%d) staging
```

If a branch with that name already exists (rare but possible if run twice in a day), append `-2`, `-3`, etc.

## Step 5: Attempt the merge

Merge upstream changes into the sync branch without auto-committing, so you can inspect the result first:

```
git merge --no-edit upstream/{branch}
```

### If the merge succeeds cleanly

Report success and proceed to Step 7.

### If there are merge conflicts

```
git status
```

List every conflicted file. For each one:

1. Show the conflict markers with surrounding context:
   ```
   git diff --diff-filter=U
   ```
2. Read the full file to understand the context on both sides.
3. Summarise the conflict: what does the upstream change, what does our fork change, and what is the tension between them?

Present a clear summary table to the user:

| File | Upstream change | Our change | Conflict nature |
|------|----------------|------------|-----------------|

Ask the user how to resolve each conflict. Options to offer:
- **Take upstream** — accept the upstream version entirely for this file
- **Keep ours** — keep the fork's version entirely for this file
- **Manual** — the user will resolve it themselves; pause and wait
- **Discuss** — talk through the specific lines together before deciding

Once all conflicts are resolved (either by applying user decisions or waiting for manual resolution):

```
git add {resolved files}
git merge --continue
```

If the user resolves manually, wait for them to confirm before running `git add` and `git merge --continue`.

## Step 6: Verify the result

After the merge completes, run a sanity check:

```
git log staging..HEAD --oneline
git diff staging..HEAD --stat
```

Report: how many commits were merged, how many files changed, and the net line delta. Note any files in our fork's custom directories (anything not present in the upstream repo) that were touched unexpectedly.

## Step 7: Push and open a PR

Push the sync branch:

```
git push -u origin sync/upstream-{date}
```

Get the repo owner/name:

```
gh repo view --json owner,name --jq '"\(.owner.login)/\(.name)"'
```

Get the list of merged commits for the PR body:

```
git log staging..HEAD --oneline
```

Open a PR into `staging`:

```
gh pr create \
  --base staging \
  --title "sync: upstream nearai/ironclaw {date}" \
  --body "$(cat <<'EOF'
## Upstream sync

Merges changes from [nearai/ironclaw](https://github.com/nearai/ironclaw) `{upstream_branch}` into our fork.

### Commits included

{commit_list}

### Conflict resolution

{conflict_summary_or_"No conflicts — clean merge."}

---
🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Report the PR URL to the user.

## Rules

- Never force-push or rebase upstream commits onto `staging` directly — always go via a sync branch and a PR.
- If the user asks to skip the PR and merge directly, remind them that a PR gives the team visibility and revert capability, then ask again.
- Do not resolve conflicts silently. Any conflict must be presented to the user before it is committed.
- Do not modify files outside of conflict resolution. This command syncs; it does not refactor.
- If any step fails unexpectedly, stop, show the error output, and ask the user how to proceed.
