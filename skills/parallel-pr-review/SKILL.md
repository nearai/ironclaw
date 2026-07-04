---
name: parallel-pr-review
version: 0.1.0
description: Use when asked to "review the open PRs", review a batch or stack of pull requests, or run a recurring PR-review pass on a repo — especially with many PRs, stacked branches, conflicts, or security-sensitive changes. Covers grouping, fan-out to review subagents, verdict synthesis, and posting.
activation:
  keywords:
    - review the open PRs
    - review open PRs
    - batch PR review
    - stacked PRs
    - review pull requests
    - PR review pass
  patterns:
    - "(?i)review\\s+(the\\s+)?(open\\s+)?(prs|pull requests)"
    - "(?i)(batch|stack(ed)?)\\s+(of\\s+)?(prs|pull requests)"
    - "(?i)(anything|what'?s)\\s+new\\s+to\\s+review"
  tags:
    - code-review
    - github
    - pull-request
  max_context_tokens: 2500
requires:
  bins: [gh, git]
---

# Parallel PR Review

## Overview

Review many open PRs at once by fanning out one read-only review subagent per PR (or per stack), each producing a structured verdict, then synthesizing a cross-PR summary and posting reviews. The reviewer (you) stays in the loop holding conclusions; subagents absorb the per-diff reading.

**Core principle:** one agent per independent unit of review, a consistent rubric and output format across all of them, then a synthesis pass that finds what no single PR review can see.

## When to Use

- "Review the open PRs" / "anything new to review?" / recurring review loops
- A batch of PRs from one author, or a stacked series (PR based on another PR's branch)
- Any time reading every diff yourself would blow context

**When NOT to use:** a single PR (just review it directly); a PR you authored mid-task (use normal self-review).

## Workflow

1. **Enumerate + triage.** List PRs with the metadata that drives decisions:
   ```
   gh pr list --state open --limit 100 --json number,title,author,additions,deletions,baseRefName,mergeable \
     --jq 'sort_by(.number) | reverse[] | "#\(.number) @\(.author.login) +\(.additions)/-\(.deletions) base:\(.baseRefName) \(.mergeable)  \(.title)"'
   ```
   - `base != main` ⇒ **stacked PR** (review with its stack, note merge order).
   - `CONFLICTING` ⇒ flag; have the agent diagnose cause + resolution. Re-check: some "conflicting" clears after an upstream rebase.
   - Compare against PRs already reviewed in prior passes — only review what's new (and re-check any that grew).

2. **Group.** One agent per PR for standalone changes; one agent per **stack** (review each stacked PR's own `gh pr diff`, in dependency order). Give **security-sensitive** PRs (contracts, crypto/keys, auth/attestation, money/billing, DoS bounds) their own dedicated agent with an adversarial prompt. For **large or structural** PRs (big refactors, new modules, files crossing ~1k lines), add a dedicated **maintainability-lens** agent (see Maintainability lens below).

3. **Fan out** review subagents in parallel — all in one message so they run concurrently. Use a code-review agent type; each gets the prompt template below. Tell them **not to modify files**.

4. **Synthesize.** Collect verdicts into one table; then write the cross-cutting section: recurring bug classes, themes spanning PRs, "already rebased/clean now," and which PRs to land first.

5. **Post** (only when asked) — see Posting.

6. **Clean up** review branches subagents fetched (see Cleanup).

## Subagent prompt template

Give each agent: the PR number(s), how to get the diff (`gh pr diff <n>`), an instruction to read surrounding context with Read/Grep, and a fixed rubric:

```
Focused [adversarial, if security] code review of PR #<n> in <repo path>.
Get the diff: `gh pr diff <n>`. Read surrounding context with Read/Grep on the
actual changed files. Do NOT modify any files.

Review priorities, in order:
1. Correctness — real bugs, races, wrong-key/off-by-one, missed cases.
2. Security — <adversarial angle for this PR: spoofable identity, replay,
   unbounded growth/DoS, fail-open, key reuse, double-pay, TOCTOU>.
3. Privacy/logging — <repo's logging rule, e.g. "log ids/counts/sizes/durations/
   error-types only, never prompts/completions/keys/raw bytes">.
4. Tests — are they non-vacuous (would fail on a real regression)? CI-runnable
   (not #[ignore]/feature-gated off)? Deterministic (no port/timing flake)?
5. Structural quality (behavior-preserving) — would a cleaner reframing DELETE
   categories of complexity, not just polish? Flag: ad-hoc conditionals/special
   cases bolted onto unrelated flows; feature-specific logic leaking into a
   general module; thin wrappers adding indirection without clarity; unnecessary
   casts / `any` / optional params muddying contracts; copy-paste instead of an
   extracted helper; a bespoke helper duplicating an existing canonical utility;
   a file pushed past ~1k lines without strong reason. Propose the restructuring,
   not the incremental tidy. Must not change semantics; outranked by 1-2.
6. [stacked/conflicting] Diagnose the conflict: git fetch origin; git log
   --oneline origin/main -20; name the colliding PR and the resolution.

Output: one-line summary, VERDICT (APPROVE / APPROVE WITH NITS / REQUEST CHANGES),
then numbered findings `severity [file:line] — issue → suggested fix`.
Be precise and skeptical; distinguish real bugs from speculation.
```

Tailor priority 2 per PR — that targeting is what makes the reviews sharp (e.g. "is the owner-proof replayable?", "is this per-deployment map bounded + pruned on undeploy?", "does the validity check use a spoofable clock?").

## Output format (consistent across all agents)

- **Verdict:** `APPROVE` / `APPROVE WITH NITS` / `REQUEST CHANGES` (add `REJECT/CLOSE` for superseded/duplicate PRs).
- **Findings:** `severity [file:line] — issue → fix`, severities Critical / Important / Low / Nit, plus `Praise`/`Good` for verified-correct load-bearing code.
- Demand **non-vacuity checks** on test PRs: a test that would pass even with the bug present is a finding, not coverage. Agents can prove it by mutation ("deleting X makes it fail").

## Maintainability lens (thermo-nuclear)

Rubric priority 5 is bug-and-risk review's structural complement: it audits **behavior-preserving** quality. Run it on every PR; for large/structural PRs give it a **dedicated agent** (as security PRs get a dedicated adversarial agent).

- **Bias to deletion ("code judo").** The win is a restructuring that makes whole branches / helpers / conditionals _disappear_ — not "a bit cleaner." Reject incremental polish when dramatic simplification is available. Prefer removing pieces over redistributing complexity.
- **Flag in impact order:** (1) structural regressions; (2) missed dramatic simplifications; (3) spaghetti/branching growth (ad-hoc conditionals, scattered special cases); (4) boundary/type-contract problems (casts, `any`, optional params); (5) file-size/decomposition (a PR pushing a file past ~1k lines without strong reason); (6) modularity/abstraction (feature logic in general modules, thin wrappers, bespoke helpers duplicating canonical utilities); (7) legibility.
- **Approval bar:** no structural regression, no obvious missed simplification, no unjustified file growth, no new spaghetti, no hacky abstraction obscuring intent.
- **Guardrails:** every structural finding must be semantics-preserving and must cite the concrete restructuring; correctness and security still outrank it; don't let it become style-nitpicking (a formatter's job). Adapted from cursor-team-kit's `thermo-nuclear-code-quality-review`.

## Posting

Post only when the user asks. Default to **review comments**, not formal approve/request-changes, when reviewing someone else's PRs — a comment carries the verdict in its body without flipping GitHub's merge-blocking state unilaterally.

```bash
# write each body to a file (heredocs survive markdown/newlines safely), then:
gh pr review <n> --comment --body-file /tmp/reviews/<n>.md
```

- Loop over PRs; print OK/FAILED per PR so a failed post is visible.
- For a superseded/duplicate PR, post a "close as superseded by #X" **conversational** comment (`gh pr comment`) instead of a review.
- Surface big cross-cutting observations as their own `gh pr comment` so they're actionable, not buried in a long review body.

## Cleanup

Subagents fetch/checkout PR branches. Afterward, return the repo to a clean `main`:

```bash
git worktree list            # ensure no stray worktrees
git branch --format='%(refname:short)'   # find leftover PR branches
```

Before deleting a leftover branch, confirm its commits exist on a remote (no local-only work lost):

```bash
git branch -r --contains "$(git rev-parse <branch>)"   # must print an origin/... ref
git branch -D <branch>                                  # then safe to delete
```

## Common Mistakes

- **One mega-agent for all PRs** → shallow, context-blown. One unit of review per agent.
- **Generic rubric** → generic findings. Tailor the security angle per PR.
- **Reviewing stacked PRs against the wrong base** → use each PR's own `gh pr diff`, note merge order.
- **Trusting `mergeable`/`reviews` blindly** → conflicts clear on rebase; "1 review" is often the author's own. Re-check.
- **Accepting test PRs at face value** → require a non-vacuity argument.
- **Formal approve/request-changes on someone else's PRs without being asked** → use `--comment`.
- **`jq` choking on PR JSON** → review bodies contain newlines; query non-body fields, or use `--jq` server-side.
- **Leaving fetched branches behind** → always run Cleanup.
