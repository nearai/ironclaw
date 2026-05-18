# Multi-Agent Code Review Skill — Design Spec

**Date:** 2026-05-17
**Status:** Draft
**Author:** opencode

## Problem

The existing `skills/code-review/SKILL.md` is a single-agent "paranoid architect" review. While thorough, it lacks:
- Specialized reviewer perspectives with dedicated scope
- Confidence-based scoring to filter false positives
- Structured aggregation of findings across multiple lenses
- Intent-aware review that evaluates changes against author goals

## Goals

- Expand the code review skill into a multi-agent orchestration with 5 specialized parallel reviewers
- Use confidence scoring (threshold ≥50) to filter noise while catching real issues
- Pass author intent as shared context to all reviewers for goal-aligned evaluation
- Post consolidated PR comment + line-level comments for Critical/High findings
- Preserve the existing single-agent review as `skills/quick-review/SKILL.md` for lightweight use cases

## Non-Goals

- CI/CD integration (out of scope for this skill)
- Build/typecheck execution (CI handles this)
- Automatic PR approval/merge

## Architecture

### Flow

```
User triggers review (/code-review or natural language)
        │
        ▼
┌─────────────────────────────────┐
│ Orchestrator                    │
│ - Parse PR reference            │
│ - Eligibility check             │
│ - Create worktree for PR branch │
│ - Gather context (diff, files)  │
│ - Find relevant CLAUDE.md files │
└───────────────┬─────────────────┘
                │
                ▼
┌─────────────────────────────────┐
│ Intent Analyzer (SEQUENTIAL)    │
│ - Extract author intent from    │
│   PR body, title, labels        │
│ - Identify scope, goals,        │
│   constraints, risk areas       │
│ - Output: structured summary    │
└───────────────┬─────────────────┘
                │ intent summary passed to all reviewers
                ▼
┌──────────────────────────────────────────────────────────────┐
│                    Parallel Reviewers (5x)                   │
│  ┌──────────┐ ┌──────┐ ┌────────────┐ ┌──────────┐ ┌───────┐│
│  │ Security │ │ Bugs │ │ Best Pract │ │ Contract │ │ Perf/ ││
│  │          │ │      │ │ + Tests    │ │ + Docs   │ │ Concur││
│  └──────────┘ └──────┘ └────────────┘ └──────────┘ └───────┘│
│  Each returns: findings with confidence scores (0-100)       │
└────────────────────────┬─────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────┐
│ Aggregator (in orchestrator)    │
│ - Merge findings                │
│ - Deduplicate                   │
│ - Filter ≥50 confidence         │
│ - Group by category             │
└───────────────┬─────────────────┘
                │
                ▼
┌─────────────────────────────────┐
│ Post Results                    │
│ - Consolidated PR comment       │
│ - Line-level comments for       │
│   Critical/High findings        │
│ - Clean up worktree             │
└─────────────────────────────────┘
```

### Model Selection

All subagents use the same model as the running opencode session. No model tiering.

### Execution Context

- **Skill location:** `skills/code-review/SKILL.md` (orchestrator)
- **Subagent SKILLs:** `skills/code-review/{intent-analyzer,security-reviewer,bugs-reviewer,best-practices-reviewer,contract-reviewer,performance-reviewer}/SKILL.md`
- **Existing skill preserved:** `skills/quick-review/SKILL.md` (renamed from current `skills/code-review/SKILL.md`)
- **Trigger:** `/code-review` command AND natural language patterns

## Component Details

### Orchestrator (`skills/code-review/SKILL.md`)

**Responsibilities:**
1. Parse PR reference (`owner/repo#N`, `owner/repo N`, `github.com/.../pull/N`, or `locally`)
2. Run eligibility check (closed, draft, trivial, already-reviewed)
3. Create a dedicated git worktree for the PR branch if it doesn't exist
4. Gather context: PR metadata, diff, changed file list, relevant `CLAUDE.md` files
5. Spawn intent analyzer sequentially, wait for structured summary
6. Spawn 5 parallel reviewers, each receiving the intent summary in their prompt
7. Collect all findings, merge, deduplicate, filter by confidence ≥50
8. Format consolidated PR comment grouped by reviewer category
9. Post line-level comments for Critical/High findings
10. Post consolidated PR-level comment
11. Clean up worktree

### Intent Analyzer (`skills/code-review/intent-analyzer/SKILL.md`)

**Input:** PR title, body, labels, changed file list
**Output:** Structured summary containing:
- Author's stated goal/purpose
- Scope of changes (subsystems touched)
- Explicit constraints (e.g., "backward compatible", "no breaking changes")
- Risk areas flagged by author
- Confidence level in understanding (low if PR body is sparse)

### Reviewer SKILLs (5x)

Each reviewer SKILL shares this structure:
- **Scope definition** — what's in, what's out
- **Review checklist** — specific lenses to apply
- **False positive guidance** — what to ignore
- **Output format** — structured findings: `file`, `line_range`, `description`, `severity` (Critical/High/Medium/Low/Nit), `confidence (0-100)`, `category`
- **Intent context injection point** — placeholder where orchestrator injects the intent summary

#### Security Reviewer
- AuthN/AuthZ bypass, IDOR
- Injection (SQL, command, log, header, prompt)
- Data leakage (secrets, PII, conversation content in logs/responses)
- DoS / resource exhaustion (unbounded input, expensive ops)
- Replay attacks, race conditions for financial abuse
- Cryptographic issues (timing attacks, weak randomness)
- **Out:** General bugs without security implications, performance issues without abuse vector

#### Bugs Reviewer
- Logic errors, off-by-one, wrong operators, inverted conditions
- Unreachable code, dead branches, impossible match arms
- Type confusion (mixing IDs, wrong enum variant)
- Incorrect error propagation (swallowed errors, wrong type)
- Broken invariants (uniqueness, ordering, state-machine transitions)
- Edge cases (empty/null input, zero-length collections, integer boundaries)
- Partial failure handling (wrote to DB but failed to emit event)
- Adversarial input (invalid UTF-8, huge payloads, deeply nested JSON)
- **Out:** Security-specific vulnerabilities (→ Security), performance issues (→ Performance), style/naming (→ Best Practices)

#### Best Practices Reviewer
- Follows existing patterns or introduces new ones without justification
- Unnecessary abstractions, premature generalizations
- Duplicated logic that should be extracted
- Module dependencies clean (no circular/tight coupling)
- **Test coverage:**
  - Every new public function/method tested
  - Error paths tested, not just happy paths
  - Edge cases covered (empty, boundary, concurrent)
  - Existing tests still valid (not asserting stale behavior)
  - Integration/e2e tests for full flows
- **Out:** Bugs (→ Bugs), security (→ Security), API contract changes (→ Contract)

#### Contract/Boundary Reviewer
- API contracts (request/response shapes, error codes)
- Type interfaces and trait contracts
- Module boundary violations
- Backward compatibility (breaking changes in public APIs)
- **Documentation:**
  - New assumptions documented in comments
  - Non-obvious algorithms or business rules explained
  - Module-level docs updated
  - API contracts documented
  - New patterns explained for future contributors
  - TODO/FIXME/HACK that should be tracked
- **Out:** Bugs in implementation (→ Bugs), test gaps (→ Best Practices)

#### Performance/Concurrency Reviewer
- TOCTOU, missing locks, race conditions
- Unbounded input without rate limits
- Expensive operations without caching/pagination
- OOM risks (large allocations, unbounded collections)
- N+1 queries, inefficient data access patterns
- Algorithmic complexity regressions
- Resource exhaustion (connection pools, file descriptors)
- **Out:** Security-focused DoS (→ Security), general logic bugs (→ Bugs)

### Aggregator (inline in orchestrator)

- **Deduplication:** Merge findings by file + line range + similar description
- **Filtering:** Include findings with confidence ≥50
- **Grouping:** Organize by reviewer category for consolidated output
- **Routing:** Findings with severity Critical or High → line-level comments; all findings → PR-level comment

Each reviewer classifies findings by severity alongside confidence:

| Severity | Meaning |
|---|---|
| **Critical** | Security vulnerability, data loss, or financial exploit |
| **High** | Bug that will cause incorrect behavior in production |
| **Medium** | Robustness issue, missing validation, incomplete error handling |
| **Low** | Style, naming, documentation, minor improvement |
| **Nit** | Optional, take-it-or-leave-it |

## Output Format

### Consolidated PR Comment

```
### Code Review

Intent: {summary from intent analyzer}

Found {N} issues:

**Security** ({N} issues)
1. {description} (confidence: {score})
   {link to file with full SHA + line range}

**Bugs** ({N} issues)
1. ...

**Best Practices** ({N} issues)
1. ...

**Contract/Docs** ({N} issues)
1. ...

**Performance/Concurrency** ({N} issues)
1. ...
```

### Line-Level Comments (Critical/High only)

Format: **{Severity}** — {one-line summary}. {detailed explanation}. {concrete fix}.

## Confidence Scoring Rubric

Passed verbatim to each reviewer:

| Score | Meaning |
|---|---|
| **0** | False positive, pre-existing issue, or doesn't hold up to light scrutiny |
| **25** | Might be real, but unverified — stylistic, not in CLAUDE.md |
| **50** | Real but minor — nitpick or low-impact in practice |
| **75** | Real and important — will impact functionality or violates CLAUDE.md |
| **100** | Certain — evidence directly confirms, will happen frequently |

**Threshold: ≥50** to include in output.

## Error Handling

| Failure Point | Behavior |
|---|---|
| PR not found / invalid ref | Return error message, don't proceed |
| PR ineligible (closed/draft/trivial) | Silent skip with brief notice |
| Intent analyzer fails | Proceed without intent context, note in output |
| One reviewer fails | Continue with remaining reviewers, note missing lens |
| All reviewers fail | Post "review could not be completed" message |
| GitHub API rate limit | Return error with retry suggestion |
| No findings ≥50 | Post "no issues found" comment |
| Line-level comment post fails | Fall back to PR-level comment for that finding |
| Worktree creation fails | Fall back to diff-only mode, note in output |

## False Positives to Filter

- Pre-existing issues not introduced in PR
- Something that looks like a bug but isn't
- Pedantic nitpicks a senior engineer wouldn't call out
- Issues linter/typechecker/compiler would catch (imports, types, formatting, broken tests)
- General code quality issues unless explicitly required in CLAUDE.md
- Issues silenced in code (lint ignore comments)
- Intentional changes related to the broader PR scope
- Real issues on lines the user did not modify

## File Structure

```
skills/
├── code-review/
│   ├── SKILL.md                          # Orchestrator
│   ├── intent-analyzer/
│   │   └── SKILL.md                      # Sequential intent extraction
│   ├── security-reviewer/
│   │   └── SKILL.md                      # Parallel: security lens
│   ├── bugs-reviewer/
│   │   └── SKILL.md                      # Parallel: bug detection
│   ├── best-practices-reviewer/
│   │   └── SKILL.md                      # Parallel: patterns + tests
│   ├── contract-reviewer/
│   │   └── SKILL.md                      # Parallel: API contracts + docs
│   └── performance-reviewer/
│       └── SKILL.md                      # Parallel: perf + concurrency
└── quick-review/
    └── SKILL.md                          # Existing single-agent review (renamed)
```

## Trigger Configuration

- **Command:** `/code-review`
- **Natural language patterns:**
  - `(?i)review\s.*(code|changes|diff|PR|pull request|commit)`
  - `(?i)(check|look at|inspect)\s.*(changes|diff|code)`
  - `(?i)review\s+[a-z0-9._-]+/[a-z0-9._-]+\s+#?\d+`
