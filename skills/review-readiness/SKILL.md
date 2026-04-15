---
name: review-readiness
version: 0.1.0
description: PR readiness dashboard — tracks which reviews have been completed per branch and gates merge decisions. Shows code review, tests, security, QA, and linting status.
activation:
  keywords:
    - review readiness
    - ready to merge
    - PR readiness
    - ship checklist
    - merge checklist
    - review status
    - PR status
    - can I merge
    - ready to ship
  patterns:
    - "(?i)(is|are) (this|it|PR|the PR) ready (to|for) (merge|ship|review)"
    - "(?i)(review|merge|ship) (readiness|checklist|status)"
    - "(?i)what (checks|reviews) are (missing|left|needed)"
  tags:
    - developer
    - review
    - process
  max_context_tokens: 1200
---

# Review Readiness Dashboard

Track process completeness per PR branch. Gate merges on evidence, not gut feel.

## How it works

For each active PR branch, maintain a readiness state in `projects/<owner>-<repo>/readiness/<branch-slug>.md`:

```
---
type: review-readiness
repo: owner/repo
branch: feature-branch-name
pr_number: 123
updated_at: YYYY-MM-DD
---
# Review Readiness — PR #123

| Check | Status | Last Run | Score | Notes |
|-------|--------|----------|-------|-------|
| Code review | completed | 2026-03-28 | — | Approved by @alice |
| Tests | passing | 2026-03-28 | — | CI green, 3 new tests |
| Security review | completed | 2026-03-28 | 85/100 | 1 P3 finding (accepted) |
| QA review | pending | — | — | Not yet run |
| Linting | passing | 2026-03-28 | — | Zero warnings |

## Verdict: NOT READY
Missing: QA review

## Findings log
- [2026-03-28] Security: P3 — missing rate limit on new endpoint (accepted risk)
- [2026-03-28] Code review: approved, 2 nits addressed
```

## When to use

**Checking readiness:** User asks "is this PR ready?" or "can I merge?"
1. Read the readiness file for the branch
2. If no file exists, create one with all checks as `pending`
3. Present the dashboard
4. If all checks passed: "Ready to merge."
5. If checks missing: "Not ready. Missing: <list>. Run `/security-review` and `/qa-review` to complete."

**After running a review skill:** When `/security-review` or `/qa-review` completes, update the readiness file with the result and score.

**In the morning brief:** For PRs that are "READY TO MERGE" (approved + CI green), also check review readiness. If security or QA is missing, note it: "READY TO MERGE (code + CI), but security review not run."

## Automated updates

The readiness file is updated by:
- **Code review**: when PR gets GitHub approval or changes-requested
- **Tests**: from CI status (green/red/pending)
- **Security review**: when `/security-review` runs on the branch
- **QA review**: when `/qa-review` runs on the branch
- **Linting**: from CI or manual `cargo clippy` / `eslint` output

## Readiness verdict logic

- **READY**: All checks completed, no unresolved P1/P2 findings, CI green
- **ALMOST READY**: All checks completed but has accepted findings or pending CI
- **NOT READY**: One or more checks not yet run
- **BLOCKED**: P1 finding unresolved, or CI failing

## Effort estimates

When presenting readiness, show dual-time estimates for remaining work:
- "Missing: QA review (~15min AI-assisted, ~2h manual)"
- "Missing: Security review (~10min AI-assisted, ~1h manual)"

This reframes the cost — completeness is cheap with AI assistance.
