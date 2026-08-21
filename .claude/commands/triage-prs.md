---
description: Classify all open PRs by module, review state, scope, and architectural impact — produces a prioritized triage dashboard
disable-model-invocation: true
allowed-tools: Bash(gh pr list:*), Bash(gh pr view:*), Bash(gh pr diff:*), Bash(gh api:*), Bash(gh pr checks:*), Bash(git log:*), Read, Grep, Glob, Agent
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
gh pr list --state merged --search "merged:>=$(python3 -c 'from datetime import datetime, timedelta, timezone; print((datetime.now(timezone.utc) - timedelta(days=7)).strftime("%Y-%m-%d"))')" --limit 100 --json number,title,body,mergedAt
```

## Step 2: Classify each PR by module

For each open PR, determine the primary module it touches by examining the `files` field. Classify into these categories based on the dominant directory. Each path appears in exactly one row; if a future edit introduces an overlap, the more specific row wins over the **Reborn stack** umbrella:

| Category | Directories |
|----------|------------|
| **Reborn stack (most current work)** | `crates/loop/ironclaw_turn_runner/`, `crates/app/ironclaw_cli/`, `crates/app/ironclaw_composition/`, `crates/events/ironclaw_event_store/`, `crates/domains/ironclaw_identity/`, `crates/product/ironclaw_openai_compat*/`, `crates/domains/ironclaw_trace_commons/`, `crates/product/ironclaw_webui/`, `crates/product/ironclaw_assistant/`, `crates/kernel/ironclaw_turns/`, `crates/domains/ironclaw_threads/`, `crates/loop/ironclaw_agent_loop/`, `crates/kernel/ironclaw_host_runtime/`, `crates/loop/ironclaw_loop_host/`, `crates/kernel/ironclaw_capabilities/` |
| **LLM & Inference** | `crates/domains/ironclaw_llm/` |
| **Agent Core** | `crates/domains/ironclaw_skills/` |
| **Tools & Extensions** | `crates/extensions/ironclaw_extension_support/`, `crates/extensions/ironclaw_extension_host/`, `crates/extensions/ironclaw_extension_registry/` |
| **Channels** | `crates/extensions/packages/slack/`, `crates/extensions/packages/telegram/` |
| **Storage & Memory** | `crates/substrates/ironclaw_filesystem/`, `crates/domains/ironclaw_memory*/`, `crates/substrates/ironclaw_libsql_runtime/`, `migrations/` |
| **Security** | `crates/substrates/ironclaw_safety/`, `crates/substrates/ironclaw_secrets/`, `crates/kernel/ironclaw_trust/`, `crates/kernel/ironclaw_authorization/`, `crates/kernel/ironclaw_approvals/` |
| **Config & Setup** | `crates/app/ironclaw_config/` |
| **Sandbox & Processes** | `crates/lanes/ironclaw_sandbox/`, `crates/kernel/ironclaw_processes/`, `crates/lanes/ironclaw_wasm*/` |
| **Hooks** | `crates/loop/ironclaw_hooks/` |
| **Events & Projections** | `crates/events/ironclaw_event_log/`, `crates/events/ironclaw_event_projections/`, `crates/events/ironclaw_event_streams/` |
| **CI/CD & Docs** | `.github/`, `README.md`, `CLAUDE.md`, `*.md` (no src) |
| **Other** | Anything else |

If a PR touches multiple modules, assign it to the **primary** module (most files changed) but note the cross-cutting modules.

Note: CI's own scope labeler (`.github/workflows/pr-label-scope.yml` + `.github/labeler.yml`) still fires a handful of non-`src/**` labels (`scope: ci` for `.github/**`, `scope: docs` for `**/*.md` and `docs/**`, `scope: dependencies` for `Cargo.toml`/`Cargo.lock`, `DB MIGRATION` for `migrations/**`), but every `src/**`-scoped label (`scope: agent`, `scope: tool*`, `scope: db*`, `scope: safety`, `scope: llm`, …) targets pre-Reborn paths that no longer exist and can never fire on a current PR — `grep -c 'src/' .github/labeler.yml` shows the extent. For unlabelled `crates/**` changes, or to classify anything the labeler doesn't cover, use this manual table; when a `scope: *` label from the list above is already present, use it instead of re-deriving.

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

CI already classifies every PR on open/sync via `.github/workflows/pr-label-classify.yml`
(`.github/scripts/pr-labeler.sh`), which sets an exclusive `size: XS|S|M|L|XL` label (by total
additions+deletions) and an exclusive `risk: *` label. The `size: *` label is diff-stat-based and
safe to trust as-is; read it from the `labels` field already fetched in Step 1 instead of
re-deriving line-count buckets — only fall back to computing size yourself (below) if the PR
predates the labeler or its label is missing (e.g. CI hasn't run yet).

**Do not trust `risk: *` at face value for `crates/**` PRs.** `classify_risk` in
`.github/scripts/pr-labeler.sh` pattern-matches only pre-Reborn `src/**` paths (plus `Cargo.toml`
and `.github/workflows/*`); a PR that touches only `crates/**` — including
`crates/kernel/ironclaw_trust/`, `crates/substrates/ironclaw_secrets/`, or
`crates/substrates/ironclaw_safety/` — always falls through to `risk: low`
(`grep -n 'src/' .github/scripts/pr-labeler.sh` shows the exhaustive pattern list). Treat
`risk: *` as advisory only, and always cross-check against the **Security** row of the Step 2
table before trusting a `risk: low` PR. If the `risk: *` label is absent entirely (classifier
hasn't run, or a transient API failure per the script's own resilience notes), mark risk as
**unknown** and flag the PR for manual review rather than assuming low.

Fallback scope table (only if `size: *` label absent):

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
- For large PR lists (>15), use the Agent tool to parallelize fetching PR details and diffs in appropriately sized batches for the active runtime and workload.
- Be concise in summaries. One line per PR in tables.
- When assessing "ready to merge", be conservative. If there's any unresolved concern from a repo member, it's not ready.
- Flag any PR that has been open >14 days with no review as needing attention.
- If a PR description says "Closes #N" but #N was already closed by another merged PR, flag it as potentially superseded.
- Do NOT post comments or take any action on PRs. This skill is read-only analysis.
