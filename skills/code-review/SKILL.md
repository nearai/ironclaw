---
name: code-review
version: "2.0.0"
description: Paranoid architect review of code changes for bugs, security, missing tests, and undocumented assumptions. Works on local git diffs OR a GitHub pull request (e.g. `owner/repo N`). For PRs, can post findings as line-level review comments.
activation:
  keywords:
    - "review"
    - "code review"
    - "review changes"
  patterns:
    - "(?i)review\\s.*(code|changes|diff|PR|pull request|commit)"
    - "(?i)(check|look at|inspect)\\s.*(changes|diff|code)"
    - "(?i)review\\s+[a-z0-9._-]+/[a-z0-9._-]+\\s+#?\\d+"
  tags:
    - "code-review"
    - "quality"
    - "security"
  max_context_tokens: 2500
requires:
  skills:
    - github
---

# Paranoid Architect Code Review

You are reviewing this change as a paranoid architect. Your job is to find every bug, vulnerability, race condition, edge case, and undocumented assumption before it ships. Assume adversarial users, concurrent access, and Murphy's law.

You handle two input shapes:

- **Local changes** — uncommitted edits or recent commits in the working tree.
- **GitHub pull request** — `owner/repo N`, `owner/repo#N`, or a `github.com/.../pull/N` URL. If the message contains anything shaped like `owner/repo` followed by a number, treat it as a PR request and use the GitHub path, not git.

## Step 1 — Load the changes

### GitHub PR path

Wrap the whole flow in `async def` and `return` from it, then `FINAL(await review())`. `FINAL()` only records the answer, it does not stop execution; without a `return`, code after `FINAL(...)` keeps running and crashes when it tries to use a variable that was never set on an error path.

```repl
async def review():
    pr_url    = f"https://api.github.com/repos/{owner}/{repo}/pulls/{number}"
    files_url = f"{pr_url}/files?per_page=100"
    # Sequential awaits instead of asyncio.gather — the Monty sandbox
    # does NOT reliably capture `import asyncio` into the function
    # closure, and the LLM has hit `NameError: name 'asyncio' is not
    # defined` when calls cross repl-block boundaries. Three serial
    # GETs against api.github.com are fast enough, and this avoids the
    # whole class of closure-capture bugs.
    meta_r = await http(method="GET", url=pr_url)
    diff_r = await http(
        method="GET", url=pr_url,
        headers=[{"name": "Accept", "value": "application/vnd.github.v3.diff"}],
    )
    files_r = await http(method="GET", url=files_url)
    for r, label in [(meta_r, "metadata"), (diff_r, "diff"), (files_r, "files")]:
        if r["status"] != 200:
            return (f"GitHub {label} fetch for {owner}/{repo}#{number} "
                    f"returned HTTP {r['status']}: {r['body']}")
    pr    = meta_r["body"]       # dict: title, state, head, base, user, head.sha, ...
    diff  = diff_r["body"]       # str: unified diff
    files = files_r["body"]      # list: per-file summaries with patch hunks
    head_sha = pr["head"]["sha"] # needed if you post line-level comments later
    # ... build the review ...
    return body

FINAL(await review())
```

Do NOT wrap `body` with `.get("body", body)` or `isinstance(..., str)` normalization. On a 2xx, `meta_r["body"]` is the parsed JSON; on the diff request it is a string. If status is not 2xx, return fast — silently falling through produces empty reviews where every field is "unknown".

### Local path

Run `shell` with `git diff` (unstaged), `git diff --cached` (staged), or `git diff HEAD~1` (last commit). For local reviews, skip the GitHub posting steps entirely and present findings in the chat.

## Step 2 — Read every changed file in full

For each file in the diff, read the **entire current file**, not just the hunks. You need surrounding context to catch:

- Callers of modified functions that now behave differently
- Trait/interface contracts the change may violate
- Invariants established elsewhere that the diff breaks

Fetch file contents via GitHub's raw media type so you get the text directly — the default `/contents/` response is base64-encoded and Monty's CodeAct sandbox does **not** ship the `base64` module (so `import base64` raises `ModuleNotFoundError`). Use the `application/vnd.github.raw` Accept header and read `body` as a plain string:

```repl
r = await http(
    method="GET",
    url=f"https://api.github.com/repos/{owner}/{repo}/contents/{path}?ref={head_sha}",
    headers=[{"name": "Accept", "value": "application/vnd.github.raw"}],
)
if r["status"] != 200:
    # Missing-on-head usually means the PR deleted the file; fall back to
    # `base=pr['base']['sha']` or skip.
    continue
file_text = r["body"]   # plain str, no base64
```

If the PR touches more than 20 files, prioritize: service logic > routes/handlers > models/types > tests > docs.

## Step 3 — Deep review (six lenses)

Walk the changes through each lens. For every finding, capture: file, line range, severity, category, concrete description, and a suggested fix.

### 3a. Correctness and bugs

- Off-by-one errors, wrong comparison operators, inverted conditions
- Unreachable code, dead branches, impossible match arms
- Type confusion (mixing up IDs, wrong enum variant, string vs newtype)
- Incorrect error propagation (swallowed errors, wrong error type or status)
- Broken invariants (uniqueness, ordering, state-machine transitions)
- Concurrency issues (TOCTOU, missing locks, races between check and use)

### 3b. Edge cases and failure handling

- Empty input, `None`/`null`, zero-length collections
- External-service failure (DB down, HTTP timeout, malformed response)
- Integer boundaries (overflow, underflow, `i64::MAX`, negative when expecting positive)
- Adversarial input (invalid UTF-8, huge payloads, deeply nested JSON)
- Are all error paths tested? Does every `?` propagation make sense?
- Partial-failure handling (wrote to DB but failed to emit event, or vice versa)

### 3c. Security (assume a malicious actor)

- **AuthN/AuthZ bypass**: Can an unauthenticated user reach this? Can workspace A access workspace B's data? IDOR?
- **Injection**: SQL via string interpolation, command, log, header, prompt injection
- **Data leakage**: Secrets, PII, or conversation content in logs, error messages, or API responses
- **Resource exhaustion / DoS**: Unbounded input, expensive operations without rate limits, OOM via large allocations
- **Financial abuse**: Tokens or credits consumed without tracking, usage limits bypassed, billing manipulated
- **Replay / races**: Same request replayed for double-spend, concurrent requests bypassing limits
- **Cryptographic issues**: Timing attacks on comparisons, weak randomness, missing HMAC verification

### 3d. Test coverage

- Every new public function/method tested?
- Error paths tested, not just happy paths?
- Edge cases covered (empty, boundary, concurrent)?
- Do existing tests still make sense, or do they assert stale behavior?
- Are there integration/e2e tests for the full flow?
- If a test is missing, name the exact test that should exist.

### 3e. Documentation and assumptions

- New assumptions documented in comments? ("this field is always non-empty because X")
- Non-obvious algorithms or business rules explained?
- Module-level docs updated to reflect new capabilities?
- API contracts (request/response shapes, error codes) documented?
- New patterns explained for future contributors?
- TODO/FIXME/HACK that should be tracked as issues?

### 3f. Architectural concerns

- Follows existing patterns, or introduces a new one without justification?
- Unnecessary abstractions or premature generalizations?
- Duplicated logic that should be extracted?
- Module dependencies clean, or circular/tight coupling introduced?
- Will this make future work harder?

## Step 4 — Present findings

The review **must**:

- Start with `Review of {owner}/{repo}#{number}: {pr["title"]}` (or `Review of local changes` for the local path)
- Cite at least one specific `path:line` from the diff, never a generic "looks good"
- Use this severity scale:

| Severity | Meaning |
|----------|---------|
| **Critical** | Security vulnerability, data loss, or financial exploit |
| **High**     | Bug that will cause incorrect behavior in production |
| **Medium**   | Robustness issue, missing validation, incomplete error handling |
| **Low**      | Style, naming, documentation, minor improvement |
| **Nit**      | Optional, take-it-or-leave-it |

Render findings as a table:

| # | Severity | Category | File:Line | Finding | Suggested fix |
|---|----------|----------|-----------|---------|---------------|

Then ask the user which findings to post as PR comments. Default: all Critical, High, and Medium. Skip this prompt for the local path.

## Step 5 — Post comments on GitHub (PR path only)

Use the same `async def` + `FINAL(await ...)` pattern. Line-level review comments require `commit_id` (the head SHA you captured in step 1) and the line number on the **post-image** side of the diff (`side: "RIGHT"`). For findings spanning multiple files or architectural critiques, post a single PR-level issue comment instead.

```repl
async def post():
    # Line-level comment on a specific file:line
    r = await http(
        method="POST",
        url=f"https://api.github.com/repos/{owner}/{repo}/pulls/{number}/comments",
        body={
            "body": "**High** — `state.store` accessed directly, bypassing dispatch. See `.claude/rules/tools.md`.",
            "commit_id": head_sha,
            "path": "src/channels/web/handlers/foo.rs",
            "line": 142,
            "side": "RIGHT",
        },
    )
    if r["status"] not in (200, 201):
        return f"Posting line comment failed: HTTP {r['status']}: {r['body']}"

    # Architectural / multi-file finding as a PR-level comment
    r2 = await http(
        method="POST",
        url=f"https://api.github.com/repos/{owner}/{repo}/issues/{number}/comments",
        body={"body": "**Architectural note**: ..."},
    )
    if r2["status"] not in (200, 201):
        return f"Posting PR comment failed: HTTP {r2['status']}: {r2['body']}"

    return f"Posted {len(line_findings)} line comments and {len(pr_findings)} PR comments."

FINAL(await post())
```

Format every comment as: bold severity tag, one-line summary, detailed explanation, concrete fix (with code if useful).

## Rules

- **Read every changed file in full before writing a single finding.** Context matters more than throughput.
- **Never comment on code you have not actually read.** Verify line numbers against the file you fetched, not against the diff offset.
- **Be specific.** "This might have issues" is useless. "Line 42 returns 404 but should return 400 because X" is useful.
- **Distinguish "this IS a bug" from "this COULD be a bug if X."** Be honest about certainty.
- **Don't nitpick formatting or style** unless it causes actual confusion. Focus on substance.
- **If the code is good, say so.** Don't invent problems to look thorough. An honest "no issues, here is what I checked" beats a padded list.
- **Round severity up when in doubt.** Cheaper to dismiss a false alarm than to miss a real bug.
- **Respect privacy:** never include customer data, secrets, or PII in posted comments.
- **Be proportional.** A one-line typo fix does not need a full security audit. Match depth to change scope.
