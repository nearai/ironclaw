---
description: Full PR lifecycle — review, fix findings, address comments, quality gate, push, CI fix loop, merge
disable-model-invocation: true
allowed-tools: Bash(gh pr view:*), Bash(gh pr diff:*), Bash(gh pr comment:*), Bash(gh pr merge:*), Bash(gh pr checks:*), Bash(gh pr edit:*), Bash(gh pr list:*), Bash(gh pr checkout:*), Bash(gh api:*), Bash(gh repo view:*), Bash(gh run view:*), Bash(gh run watch:*), Bash(git diff:*), Bash(git log:*), Bash(git fetch:*), Bash(git checkout:*), Bash(git status:*), Bash(git branch:*), Bash(git add:*), Bash(git commit:*), Bash(git push:*), Bash(git merge:*), Bash(git rebase:*), Bash(cargo fmt:*), Bash(cargo clippy:*), Bash(cargo test:*), Bash(cargo check:*), Read, Edit, Write, Grep, Glob, Agent
argument-hint: "<pr-number or url> [--fix] [--merge] [--review-only]"
---

# PR Shepherd

Full PR lifecycle: review → fix → quality gate → push → CI → merge.

Parse `$ARGUMENTS`:
- Extract PR number from bare number or `https://github.com/owner/repo/pull/123` URL.
- Flags: `--fix` (auto-fix without asking), `--merge` (merge when CI green), `--review-only` (stop after review, don't fix).
- If no PR number, detect from current branch: `gh pr list --head $(git branch --show-current) --json number --jq '.[0].number'`
- If still nothing, stop and ask the user.

---

## Phase 1: Situational Awareness

Gather everything in parallel:

**PR metadata:**
```
gh pr view {number} --json number,title,body,author,baseRefName,headRefName,headRefOid,state,isDraft,mergeable,mergeStateStatus,files,additions,deletions,labels,reviewRequests
```

**Diff:**
```
gh pr diff {number}
gh pr diff {number} --name-only
```

**CI status:**
```
gh pr checks {number} --json name,status,conclusion,detailsUrl
```

**Review comments (human + bot):**
```
gh api --paginate repos/{owner}/{repo}/pulls/{number}/comments
gh api --paginate repos/{owner}/{repo}/pulls/{number}/reviews
```

Resolve `{owner}/{repo}`:
```
gh repo view --json owner,name --jq '"\(.owner.login)/\(.name)"'
```

Save `headRefOid` — needed for posting line comments later.

**Assess the situation and print a status card:**

```
PR #{number}: {title}
Author: {author}    Base: {base} ← {head}
Size: +{additions} -{deletions} across {file_count} files
CI: {PASS|FAIL|PENDING|NONE}    Mergeable: {yes|no|conflict}
Reviews: {N approved, N changes_requested, N comments-only, N bot-only}
Unresolved comments: {N}
Draft: {yes|no}
```

**Decide the mode** based on situation:
- **Has unresolved review comments** → Phase 2a (address comments first, then review remaining)
- **No reviews yet / bot-only reviews** → Phase 2b (full deep review)
- **CI failing, no review issues** → Phase 4 (jump to CI fix)
- **Everything green + approved** → Phase 6 (ready to merge)

---

## Phase 2a: Address Existing Review Comments

For each unresolved review comment or review with CHANGES_REQUESTED:

1. **Read the referenced code** at the file and line mentioned. Never assess without reading.
2. **Classify each comment:**
   - ✅ **Valid & unresolved** — needs a code fix
   - ✅ **Already fixed** — a later commit addressed it
   - ❌ **False positive** — explain why the code is correct
   - 🔧 **Nit** — optional improvement, not blocking

3. **Deduplicate** — bots (Copilot, Gemini) often post the same finding. Group by actual issue.

Present a table:

| # | Source | File:Line | Issue | Status | Planned Fix |
|---|--------|-----------|-------|--------|-------------|

Wait for user confirmation (unless `--fix` flag set), then proceed to Phase 3.

---

## Phase 2b: Deep Review (6 Lenses)

Read EVERY changed file in full (not just diff hunks). For PRs touching >20 files, prioritize: service logic > handlers > types > tests > docs. Batch reads in parallel via Agent tool.

### IronClaw-specific checks (always)
- No `.unwrap()` or `.expect()` in production code
- Prefer `crate::` for cross-module imports (`super::` OK in tests/intra-module)
- Error types use `thiserror`
- If persistence touched, both backends updated (postgres.rs AND libsql/)
- New tools implement `Tool` trait correctly and registered
- External tool output passes through safety layer
- Tool parameters redacted before logging/SSE
- No byte-index slicing on external strings
- Case-insensitive comparisons where needed

### Correctness
Off-by-one, wrong operators, inverted conditions, unreachable code, type confusion, error propagation, broken invariants, TOCTOU races.

### Edge cases & failure handling
Empty/None/zero-length input, external service failures, integer boundaries, malformed/adversarial input, partial failure handling.

### Security (assume adversarial actors)
Auth/authz bypass, IDOR, injection (SQL/command/log/header), data leakage in logs/errors/API responses, resource exhaustion, replay/race conditions.

### Test coverage
New public functions tested? Error paths tested? Edge cases covered? Existing tests still valid?

### Architecture
Follows existing patterns? Unnecessary abstractions? Duplicated logic? Clean module dependencies?

**Present findings as a table:**

| # | Severity | Category | File:Line | Finding | Suggested Fix |
|---|----------|----------|-----------|---------|---------------|

Severity: Critical > High > Medium > Low > Nit

If `--review-only` flag is set, post findings as GitHub comments (see Phase 2c) and STOP.

Otherwise, ask which findings to fix (default: all Critical + High + Medium). Then proceed to Phase 3.

---

## Phase 2c: Post Review Comments on GitHub

For each finding the user approved (or all Critical/High/Medium if `--fix`):

**Line-specific findings** — post as PR review comments:
```
gh api repos/{owner}/{repo}/pulls/{number}/comments \
  -f body="**{Severity}**: {finding}\n\n{explanation}\n\n**Suggested fix:** {suggestion}" \
  -f path="{file}" \
  -f commit_id="{headRefOid}" \
  -F line={line} \
  -f side="RIGHT"
```

**Cross-cutting/architectural findings** — post as regular PR comment:
```
gh pr comment {number} --body "..."
```

---

## Phase 3: Fix

Checkout the PR branch if not already on it (handles fork PRs automatically):
```
gh pr checkout {number}
```

**Implement fixes** for:
1. All approved review comment fixes (from Phase 2a)
2. All approved review findings (from Phase 2b)

Follow IronClaw conventions:
- `thiserror` for errors
- `crate::` imports
- No `.unwrap()` in production
- Both DB backends if persistence touched
- Regression test for every bug fix (enforced by commit-msg hook; bypass only with `[skip-regression-check]` if genuinely not feasible)

After all fixes implemented, proceed to Phase 4.

---

## Phase 4: Quality Gate

Run the full IronClaw shipping checklist:

```bash
cargo fmt
```

```bash
cargo clippy --all --benches --tests --examples --all-features
```

```bash
cargo test --lib
```

If persistence changes are present, also verify feature isolation:
```bash
cargo check --no-default-features --features libsql
cargo check --all-features
```

**If any step fails:** fix the issue and re-run. Do NOT proceed past a failing step. Loop up to 3 times per step. If still failing after 3 attempts, report the failure and stop.

---

## Phase 5: Commit & Push

Stage changed files by name (never `git add -A` — it can include unintended files):
```bash
git add path/to/changed/file1 path/to/changed/file2
git commit -m "{message}"
```

Commit message format:
- For review fixes: `fix: address review findings on PR #{number}`
- For comment responses: `fix: address review comments on PR #{number}`
- For CI fixes: `fix: resolve CI failures on PR #{number}`
- Include specifics in the body (which findings/comments were addressed)

Push:
```bash
git push origin {headRefName}
```

**Reply to addressed review comments on GitHub.** For each comment that was fixed, reply with the commit SHA and a brief description of what was done. For false positives, reply explaining why no change was needed.

---

## Phase 6: CI Monitor & Fix Loop

Wait briefly for CI to start, then poll (do NOT use `--watch` as it can hang indefinitely):
```
gh pr checks {number} --json name,status,conclusion
```

Re-check every 30 seconds, up to 10 minutes. If still pending after 10 minutes, report status and ask the user whether to keep waiting.

**If CI passes** → proceed to Phase 7.

**If CI fails** (up to 3 fix attempts):

1. Identify the failing check:
   ```
   gh run view {run_id} --log-failed
   ```
   If `--log-failed` shows nothing useful:
   ```
   gh run view {run_id} --log | tail -100
   ```

2. Diagnose and fix the failure.
3. Re-run Phase 4 (quality gate).
4. Commit and push (Phase 5).
5. Go back to top of Phase 6.

**After 3 failed CI fix attempts:** Report what's failing and why, then stop. Don't keep looping.

---

## Phase 7: Merge Decision

Print final status:
```
PR #{number}: {title}
CI: ✅ PASS
Reviews: {summary}
Findings fixed: {N}
Comments addressed: {N}
Commits added: {N}
```

**Auto-merge conditions** (if `--merge` flag or user confirms):
- CI is passing
- No unresolved CHANGES_REQUESTED reviews
- PR is not draft
- PR is mergeable (no conflicts)

If all conditions met, ask the user for merge strategy:

"CI is green. Merge this PR? [squash/rebase/merge/no]"

Then execute:
```
gh pr merge {number} --{strategy} --delete-branch
```

If any condition NOT met, report what's blocking and let the user decide.

---

## Rules

- **Read before judging.** Never comment on code you haven't read in full. Verify line numbers.
- **Be specific.** "Line 42 returns 404 but should return 400 because X" not "this might have issues."
- **Fix the pattern, not just the instance.** When fixing a bug, grep for the same pattern across `src/`.
- **Respect the commit-msg hook.** Bug fixes need regression tests. Use `[skip-regression-check]` only if genuinely not feasible.
- **Don't over-fix.** Only change what was flagged. Don't refactor surrounding code or add improvements beyond the review scope.
- **Credit original authors.** If taking over someone else's PR, credit them in commits and comments.
- **No secrets in comments.** Never include customer data, credentials, or PII in GitHub comments.
- **Distinguish certainty.** "This IS a bug" vs "This COULD be a bug if X." Be honest.
- **Round up severity when uncertain.** Cheaper to dismiss a false alarm than miss a real bug.
- **Parallel where possible.** Use Agent tool for parallel file reads on large PRs. Batch `gh api` calls.
