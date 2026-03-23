---
name: pr-review-batch
description: IronClaw maintainer PR review -- batch review open PRs against ironclaw project standards (Rust, WASM tools, dual-backend DB, security-first)
triggers:
  - review PR
  - review PRs
  - review open PRs
  - batch review
  - "review #"
  - check PRs
---

# IronClaw PR Review Workflow

Maintainer review workflow for the **nearai/ironclaw** repository. Optimized for batch review with parallel data fetching, security-first evaluation against IronClaw's Rust/WASM architecture, and structured GitHub review comments.

- **Repository:** nearai/ironclaw
- **Maintainer GitHub:** zmanian
- **Primary language:** Rust (async tokio, wasmtime, axum)
- **Key subsystems:** WASM tool sandbox, dual-backend DB (postgres + libsql), LLM provider decorator chain, multi-channel system, SKILL.md skills, registry/installer
- **CI jobs that matter:** Formatting, Clippy (default, all-features, libsql-only, Windows), Regression test enforcement
- **CI jobs that DON'T prove much:** classify, scope (these always pass, even on fork PRs with no secrets)

## Review Modes

The user controls how interactive the review is. Detect the mode from their message:

| User Says | Mode | Behavior |
|-----------|------|----------|
| "Review 938, 933" | **Autonomous** | Fetch, evaluate, post reviews without stopping |
| "Review PRs. Interview me" | **Interactive** | Present findings, ask for input before posting |
| "Check on open PRs" | **Triage** | Summarize state of each PR, ask what to review in depth |
| "Approve 683 and 687" | **Direct verdict** | Post the specified verdict without full analysis |

**Default is autonomous** unless the user says "interview", "ask me", "discuss", "check with me", or similar.

## Step 1: Parse PR Numbers

Extract PR numbers from the user's message. Accept formats:
- "Review 938, 933, 918"
- "Review #834 and #922"
- "Review all open PRs" (use `gh pr list --state open --limit 30`)

## Step 2: Fetch Data (Parallel)

For EACH PR, fetch all of these in parallel:

```bash
# Metadata: title, author, base/head branch, size
gh pr view <N> --json title,author,state,headRefName,baseRefName,additions,deletions,changedFiles \
  --jq '{title, author: .author.login, state, base: .baseRefName, head: .headRefName, additions, deletions, changedFiles}'

# Full diff
gh pr diff <N> --patch

# CI status
gh pr checks <N>

# Previous reviews (for re-reviews)
gh pr view <N> --json reviews --jq '.reviews[] | {author: .author.login, state: .state, body: .body[:200]}'
```

For large diffs (>1000 lines), use `gh pr diff <N> --patch | head -500` first, then fetch remaining sections as needed. Note Cargo.lock churn separately -- don't count it as meaningful diff.

## Step 3: Evaluate Each PR

Check in this priority order:

### 3a. CI Status
- All checks must pass -- not just classify/scope. Must have: Formatting, Clippy (all 3 feature combos), Regression test enforcement.
- **Fork PRs (critical gotcha):** Only classify/scope run because GitHub Actions secrets aren't available for fork PRs. The PR will APPEAR to have passing checks. Never trust this. Flag it -- local CI verification or maintainer-triggered re-run required before merge.

### 3b. Previous Reviews
- Check if zmanian already reviewed -- if so, this is a re-review
- For re-reviews: verify each previous feedback item was addressed, referencing specific commit hashes
- Note reviews from Gemini, Copilot -- cross-reference their findings but don't trust blindly

### 3c. Security (Highest Priority -- IronClaw-Specific)
- **Identity file write protection:** PROTECTED_IDENTITY_FILES (AGENTS.md, SOUL.md, USER.md, IDENTITY.md) must not become LLM-writable
- **Tool approval requirements:** ApprovalRequirement changes (Never vs UnlessAutoApproved vs Always) -- especially for tools that cross trust boundaries (tool_install, tool_auth, build_tool, shell)
- **WASM sandbox boundaries:** fuel limits, memory limits, network allowlists must not be weakened
- **Credential handling:** no secrets in logs/errors/SSE events; use `redact_params()` before broadcast
- **SSRF vectors:** URL validation must resolve DNS before checking for private/loopback IPs
- **Prompt injection defense:** sanitizer/validator/policy changes in `src/safety/`

### 3d. Correctness (IronClaw-Specific)
- **No `.unwrap()/.expect()` in production code** (tests are fine)
- **String safety:** no byte-index slicing (`&s[..n]`) on user/external strings -- use `is_char_boundary()` or `char_indices()`
- **Dual-backend DB:** new persistence features must support BOTH postgres AND libsql. Check for missing trait implementations.
- **Feature flags:** changes must compile under `--no-default-features --features libsql`, default, and `--all-features`
- **Transaction safety:** multi-step DB operations wrapped in transactions (both backends)
- **LLM provider decorator chain:** new `LlmProvider` trait methods must be delegated in ALL wrapper types (grep `impl LlmProvider for`)

### 3e. Architecture & Conventions
- `crate::` for cross-module imports (not `super::` except tests and intra-module)
- `thiserror` for error types in `error.rs`; map errors with context via `.map_err()`
- Strong types over strings (enums, newtypes)
- Module specs followed -- if a module has a CLAUDE.md (agent, web, db, llm, setup, tools, workspace), check it
- Module-owned initialization: init logic lives in owning module as public factory fn, not in main.rs/app.rs
- No unnecessary dependencies (check `~/.claude/approved-dependencies.md` list)

### 3f. Tests
- Bug fixes MUST have regression tests (enforced by CI regression-check job and commit-msg hook)
- Tests use `tempfile` crate, not hardcoded `/tmp/` paths
- No real network requests in tests (use mocks or RFC 5737 TEST-NET IPs like 192.0.2.1)
- Test names and comments match actual test behavior and assertions
- `[skip-regression-check]` in commit message or PR label only if genuinely not feasible

## Step 4: Interview (Interactive Mode)

In interactive mode, present findings and ask for the maintainer's judgment before posting. **Do NOT post reviews until the maintainer confirms.**

### When to Interview (Even in Autonomous Mode)

Always pause and ask the maintainer when you encounter:

1. **Judgment calls on architecture direction** -- "This PR adds a named provider for Z.AI. Should we prefer named providers or push contributors toward openai_compatible for niche providers?"
2. **Security tradeoffs with usability** -- "Removing approval from tool_install reduces friction but weakens the trust boundary. What's your stance?"
3. **Scope creep concerns** -- "This PR started as a bug fix but adds 300 lines of new feature. Accept as-is or ask to split?"
4. **Dependency additions** -- "This adds `datafusion` (heavy dep). Worth it for the use case?"
5. **Contradictory signals** -- "Gemini approved but Copilot flagged a real issue. The code works but the pattern is fragile."
6. **Taking over vs requesting changes** -- "This PR has 5+ issues. Want me to take it over or send detailed feedback?"
7. **Merge ordering for conflicting PRs** -- "PRs #933 and #918 both modify cli/mod.rs. Which should land first?"

### Interview Format

Present findings concisely, then ask a specific question:

```
**PR #922: Relax tool approval requirements**

The HTTP GET change is clean (tiered: credentials->Always, GET->Never, other->UnlessAutoApproved).

But it also removes approval from:
- build_tool (can execute shell commands)
- tool_install (downloads WASM modules)
- tool_auth (grants credentials to tools)

These cross the trust boundary. Options:
1. Approve as-is (maximum convenience)
2. Request changes: keep build_tool + extension tools gated, accept the rest
3. Request changes: revert everything except HTTP GET and list_dir

Which direction?
```

Wait for the maintainer's response before posting.

### Triage Mode

In triage mode, present a dashboard first:

```
| PR | Author | Title | CI | Reviews | Age | Risk |
|----|--------|-------|----|---------|-----|------|
| #938 | reidliu41 | Z.AI provider | green | none | 1d | low |
| #922 | ilblackdragon | relax approvals | green | copilot:concern | 2d | medium |
| #927 | ilblackdragon | chat onboarding | green | zmanian:changes | 3d | high |
```

Then ask: "Which ones should I review in depth? Or should I go through all of them?"

## Step 5: Determine Verdict

| Verdict | Criteria |
|---------|----------|
| **APPROVE** | Clean, follows IronClaw patterns, full CI green, no security issues, tests present |
| **REQUEST CHANGES** | Security regressions, functional bugs, .expect() in production, trust boundary violations, missing dual-backend support, missing error handling |
| **COMMENT** | Good direction but needs discussion, or already approved with observations |

In interactive mode, confirm the verdict with the maintainer before posting. In autonomous mode, post directly.

## Step 6: Post Reviews

Post reviews via `gh pr review` using HEREDOC for body formatting.

### New Review Format

```
## Review: <short summary of what PR does>

<1-2 sentence assessment>

Positives:
- <what works well>
- <pattern compliance>

### <Severity>: <issue title>
<Detailed explanation>

### <Severity>: <issue title>
<Detailed explanation>

Minor notes:
- <non-blocking observation>

<Concrete suggestion if requesting changes>
```

Severity levels: Critical, Concerning, Minor (non-blocking)

### Re-Review Format

```
## Re-review: <status summary>

All/N items from my previous review have been resolved:

1. **<item>** -- Fixed in commit <hash>. <What changed>.
2. **<item>** -- Fixed. <Details>.

<Additional observations if any>

LGTM.
```

## Step 7: Handle GitHub API Errors

GitHub 502s are common during batch posting. Retry with `sleep 5` between attempts. Post reviews sequentially (not in parallel) to avoid rate limits.

## Step 8: Summary

After all reviews are posted, provide a summary table:

```
| PR | Title | Verdict |
|----|-------|---------|
| #938 | Z.AI provider | Approved |
| #933 | channels list CLI | Approved |
| #918 | skills CLI | Approved |
```

Note cross-PR conflicts (e.g., PRs that both modify `src/cli/mod.rs` and snapshot files).

## Special Cases

### Fork PRs
Only classify/scope CI jobs run. **Never merge with only these passing.** Either:
- Run local CI: `cargo check --all-features && cargo clippy --all && cargo test`
- Or trigger full CI by pushing a maintainer commit to the PR branch

### Registry/WASM PRs
- Verify artifact URLs match the naming convention: `<kind>-<name>-<version>-wasm32-wasip2.tar.gz`
- Check SHA256 checksums against actual release assets
- Ensure `name` field in manifest matches crate_name in source config
- Cross-reference with `.github/workflows/release.yml` for automated patching

### Taking Over a PR
When a contributor PR has too many issues:
1. Create new branch from staging
2. Cherry-pick or apply the contributor's changes
3. Fix the issues
4. Create superseding PR referencing the original

### Cross-PR Context
When PRs are related (e.g., all touch registry manifests, or both modify cli/mod.rs), post context comments on each explaining how they fit together and merge ordering.

### Batch Merge
When the user says "merge" after reviews, use `gh pr merge <N> --squash` for each approved PR. Verify CI is still green before each merge.
