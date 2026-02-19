---
description: Classify all open PRs by module, review state, scope, and architectural impact — produces a prioritized triage dashboard
disable-model-invocation: true
allowed-tools: Bash(gh pr list:*), Bash(gh pr view:*), Bash(gh pr diff:*), Bash(gh api:*), Bash(gh pr checks:*), Bash(git log:*), Read, Grep, Glob, Task
argument-hint: "[--label=<filter>] [--author=<filter>]"
---

# PR Triage Dashboard

You are triaging all open PRs on this repository. Your job is to produce a prioritized, module-grouped dashboard that tells the maintainer exactly which PRs need attention and in what order.

## Step 1: Fetch all open PRs

Fetch every open PR with metadata:

```
gh pr list --state open --limit 100 --json number,title,author,labels,additions,deletions,headRefName,createdAt,updatedAt,isDraft,reviewRequests,reviews,files,body
```

If `$ARGUMENTS` contains `--label=<X>`, append `--label '<X>'` to the `gh pr list` command. If it contains `--author=<X>`, append `--author '<X>'` to the command.

Also fetch recently merged PRs (last 7 days) to detect superseded/conflicting work:

```
gh pr list --state merged --search "merged:>=$(date -v-7d +%Y-%m-%d)" --limit 100 --json number,title,body,mergedAt
```

## Step 2: Classify each PR by module

For each open PR, determine the primary module it touches by examining the `files` field. Classify into these categories based on the dominant `src/` subdirectory:

| Category | Directories |
|----------|------------|
| **LLM & Inference** | `src/llm/` |
| **Agent Core** | `src/agent/`, `src/skills/` |
| **Tools** | `src/tools/`, `tools-src/` |
| **Channels** | `src/channels/`, `channels-src/` |
| **Storage & Memory** | `src/db/`, `src/workspace/`, `migrations/` |
| **Security** | `src/safety/`, `src/secrets/` |
| **Config & Setup** | `src/config.rs`, `src/setup/`, `src/cli/` |
| **Sandbox & Orchestration** | `src/sandbox/`, `src/orchestrator/`, `src/worker/` |
| **Hooks & Extensions** | `src/hooks/`, `src/extensions/` |
| **Context & History** | `src/context/`, `src/history/`, `src/estimation/`, `src/evaluation/` |
| **Web Gateway** | `src/channels/web/` |
| **CI/CD & Docs** | `.github/`, `README.md`, `CLAUDE.md`, `*.md` (no src) |
| **Other** | Anything else |

If a PR touches multiple modules, assign it to the **primary** module (most files changed) but note the cross-cutting modules.

## Step 3: Assess review state

For each PR, determine its review status:

- **Approved** — At least one human APPROVED review, no outstanding CHANGES_REQUESTED
- **Changes requested** — At least one CHANGES_REQUESTED review still unresolved
- **Reviewed (comments only)** — Human comments but no formal approve/reject
- **Automated only** — Only bot reviews (gemini-code-assist, copilot, etc.)
- **No review** — No reviews at all

Also check:
- CI status: `gh pr checks {number}` — PASS / FAIL / NONE
- Draft status: is the PR marked as draft?
- Staleness: how many days since `updatedAt`?

## Step 4: Determine scope and risk

Classify each PR by scope:

| Scope | Criteria |
|-------|----------|
| **Tiny** | <50 lines changed (additions + deletions), 1-2 files |
| **Small** | 50-200 lines, 1-5 files |
| **Medium** | 200-500 lines, 3-10 files |
| **Large** | 500-2000 lines, 5-20 files |
| **XL** | 2000+ lines or 20+ files |

## Step 5: Classify as fix vs. architectural

For each PR, determine its nature:

### Fixes (merge fast)
- Bug fixes with clear root cause
- Security patches
- Crash/panic prevention
- Typo/doc corrections
- Code quality (removing .unwrap(), etc.)

### Features (standard review)
- New functionality within existing patterns
- New tool implementations
- Configuration additions
- Test additions

### Architectural (deep review needed)
- New modules or subsystems
- Changes to core traits or interfaces
- New database backends or storage engines
- New provider abstractions
- Changes touching 5+ modules
- Anything modifying the agent loop, session model, or security layer
- New dependencies (check Cargo.toml changes)

## Step 6: Detect conflicts and superseded PRs

Check for:
- Multiple PRs fixing the same issue (look at "Closes #N" / "Fixes #N" in PR bodies)
- PRs touching the same files (potential merge conflicts)
- PRs that are follow-ups to other open PRs (dependency chains)
- PRs superseded by recently merged work

## Step 7: Produce the dashboard

Present the output in this format:

### Quick Stats
```
Open: N | Draft: N | Needs review: N | Changes requested: N | Ready to merge: N
```

### Ready to Merge
PRs that are approved, CI passing, and non-draft. List with one-line summary.

### Needs Human Review (Fixes)
Fixes that have no human review yet, sorted by severity (security > crash > bug > quality).

### Needs Human Review (Features)
Features with no human review, sorted by scope (smallest first).

### Needs Deep Architectural Review
Large/XL PRs, new modules, or cross-cutting changes. For each, include:
- Which modules are affected
- What new patterns or abstractions are introduced
- Key risk areas to focus review on

### Changes Requested (Waiting on Author)
PRs where a reviewer asked for changes. Include who requested and a 1-line summary of what's needed.

### Stale / Blocked
PRs with no activity >7 days, or blocked by other PRs.

### Conflicts & Overlaps
Any detected conflicts, superseded PRs, or dependency chains.

### By Module
Group all PRs by their primary module in a compact table:

| Module | PRs | Key PR to review first |
|--------|-----|----------------------|

### Superseded PRs (recommend closing)
PRs that are clearly superseded by merged work. Include reasoning.

## Rules

- Use `gh` CLI for all GitHub operations. Never guess PR state — always check.
- For large PR lists (>15), use the Task tool to parallelize fetching PR details and diffs.
- Be concise in summaries. One line per PR in tables.
- When assessing "ready to merge", be conservative. If there's any unresolved concern from a repo member, it's not ready.
- Flag any PR that has been open >14 days with no review as needing attention.
- If a PR description says "Closes #N" but #N was already closed by another merged PR, flag it as potentially superseded.
- Do NOT post comments or take any action on PRs. This skill is read-only analysis.
