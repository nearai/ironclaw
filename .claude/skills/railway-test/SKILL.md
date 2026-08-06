---
name: railway-test
description: Verify any pull request on its Railway preview through browser-driven acceptance testing derived from the PR description and diff, then publish exact-head evidence on the pull request. Defaults to nearai/ironclaw and covers deployment freshness, bearer-token login, affected-route discovery, primary user journeys, regression reproduction, forms and state read-back, navigation, permissions, errors, and optional streaming cadence. Use when the user invokes /railway-test or $railway-test, asks to test a PR on Railway, or wants browser-based preview QA with PR evidence before merging.
---

# Railway Test

Test the behavior the PR claims to change against the exact deployed PR head.
Derive the browser plan from the PR instead of running a fixed chat scenario.

## Inputs and secrets

Resolve from the request or current repository:

- PR number or URL. If omitted, use `gh pr view`.
- Repository. Default to `nearai/ironclaw`; the preview URL fallback is
  repo-specific, so an explicit repository must be supplied when testing
  another repo.
- Preview URL. Resolve it from the request or Railway status. For
  `nearai/ironclaw`, fall back to
  `https://ironclaw-ironclaw-pr-<PR>.up.railway.app`.
- Preview bearer token. Reuse an authenticated browser session when possible;
  otherwise request it from the user.

Never persist the bearer in the skill, repository, scripts, shell history,
URLs, PR text, logs, screenshots, or final response. Never print or repeat it.
Type it only into the preview login UI. Do not inspect or return browser
storage values containing it.

## Workflow

### 1. Understand the PR before opening Railway

Read repository guidance relevant to testing. For Ironclaw, read
`docs/internal/testing-playbook.md` before selecting tests.

Inspect:

```bash
gh pr view <PR> --repo <REPO> \
  --json number,title,body,url,baseRefName,headRefName,headRefOid,files
gh pr diff <PR> --repo <REPO>
```

Identify:

- the user-visible behavior promised by the PR;
- affected routes, screens, roles, and state;
- the exact regression or before/after claim;
- important success, validation, permission, and error paths;
- dependencies on external services, model behavior, or persisted data.

Write a compact Given/When/Then test matrix before interacting with the
preview. Use [references/test-recipes.md](references/test-recipes.md) to choose
only relevant cases. Do not default to chat or streaming tests.

### 2. Confirm the intended build is live

Run (resolve the path against this skill's `scripts/` directory):

```bash
scripts/preview_state.sh <PR> [REPO] [PREVIEW_URL]
```

Record `head_sha`, `railway_state`, and `asset`. Poll every 30–45 seconds,
up to 20 attempts (roughly 15 minutes), until the head-specific Railway check
passes. Send a concise progress update at least once per minute.

If the Railway check is still `missing` or `pending` after the attempt limit,
stop polling and report BLOCKED with the latest check diagnostics; never test
a partial build. A failing Railway check is BLOCKED as well until the
intended build is live.

For frontend changes, compare the asset hash with the pre-deploy baseline.
For backend-only changes, an unchanged asset is acceptable after the
head-specific Railway check passes. Do not test a stale build.

If Railway fails, inspect the linked check/deployment and stop browser testing
until the intended build is live.

The PR head can move while this skill runs. Re-read `headRefOid` immediately
before opening the preview (step 3) and again immediately before publishing
(step 7). If either read differs from `head_sha`, restart build confirmation
for the new head, or report BLOCKED for the stale build; never run or publish
browser evidence against a head you did not confirm.

### 3. Open and authenticate the preview

Re-read `headRefOid` and confirm it still equals the `head_sha` recorded in
step 2 before opening the preview; if it changed, return to step 2 for the new
head instead of testing a stale build.

Read and follow the `browser:control-in-app-browser` skill. Announce that this
skill is opening the preview and will use the supplied token only for UI login.

Use the in-app browser, not an external Playwright process. Open the route most
relevant to the PR; use `/chat` only for chat changes. Wait for session
initialization and inspect a DOM snapshot before assuming labels or controls.

- If already authenticated, continue.
- If a login form appears, fill the bearer into its token/password field and
  submit without exposing it in tool titles or output.
- If authentication fails, report only that the credential was rejected.

### 4. Execute the PR-specific browser matrix

For each selected case:

1. Capture the starting URL and a focused DOM snapshot.
2. Perform the smallest realistic user journey through visible controls.
3. Assert the promised outcome from rendered state, not click success.
4. Refresh, revisit, or read back when persistence matters.
5. Record concise evidence: route, action, observed result, and pass/fail.

Prefer caller-facing verification:

- For saved settings or CRUD, read back after refresh or navigation.
- For permissions, test both the intended role and a safe denied path when
  credentials are available.
- For navigation, test deep links and refresh behavior.
- For errors, use bounded invalid input; do not damage shared data.
- For backend-only changes, drive the nearest real UI caller when one exists.
- For side effects, create clearly named temporary test data and clean it up
  only when cleanup is safe and authorized.

Do not modify code or production state beyond reversible test data unless the
user explicitly requested a fix or broader mutation.

### 5. Run specialized checks only when relevant

- Streaming or incremental UI: read
  [references/streaming-cadence.md](references/streaming-cadence.md) and measure
  visible DOM growth. Raw wire delivery alone is insufficient.
- Upload/download: use a small non-sensitive fixture and verify content
  read-back, not only a success toast.
- Responsive/layout: inspect at the viewport sizes named in the PR or the
  nearest product-supported breakpoints.
- Auth/session: verify login, refresh, logout, and denied access only to the
  extent the PR changes those contracts.
- External provider/model: separate deterministic surface evidence from live
  canary variability.

Skip unrelated recipes and state why.

### 6. Diagnose failures without silently expanding scope

When a case fails:

- reproduce once;
- capture the smallest useful DOM, route, status, and timing evidence;
- distinguish stale deployment, authentication, frontend presentation,
  backend response, persistence, and external-service failures;
- inspect read-only logs or network evidence when available;
- do not implement a fix unless requested.

If the PR claim conflicts with live behavior, report the conflict rather than
weakening the test.

### 7. Build evidence, clean up, and publish PR evidence

Build the full evidence body FIRST, before any cleanup, so a browser or
cleanup failure can never lose the PASS/FAIL/BLOCKED result. Include:

- a `Railway preview QA — PASS|FAIL|BLOCKED` heading;
- the tested head SHA, Railway state, preview URL, and relevant route;
- the Given/When/Then matrix with concise observed evidence;
- the exact regression result and any refresh or read-back result;
- skipped cases with reasons and remaining risks or blockers;
- cleanup status;
- `<!-- railway-test:evidence head=<HEAD_SHA> -->` as a hidden marker.

Never include the bearer, credentials, sensitive browser state, or secrets.

Then clean up, guarded so failures cannot skip publication:

1. Remove tracked `railway-test-` test data in a `finally`-equivalent step;
   record cleanup status in the evidence regardless of outcome.
2. Finalize browser tabs best effort:

```js
await browser.tabs.finalize({ keep: [] })
```

3. Re-read `headRefOid` once more. If it differs from the tested head, append
   a BLOCKED note to the evidence instead of publishing evidence for a stale
   build.
4. Publish the prepared evidence as a comment on the tested pull request.
   Post evidence for PASS, FAIL, and BLOCKED runs; a failed or blocked test is
   still useful PR evidence. Never put the bearer, credentials, sensitive
   browser state, or secrets in the comment.

   Find the existing evidence comment for this exact head — the one whose
   body contains the hidden marker AND whose author is the current GitHub
   user:

   ```bash
   GH_USER="$(gh api user --jq .login)"
   gh api repos/<REPO>/issues/<PR>/comments \
     --jq ".[] | select(.user.login == \"$GH_USER\") | select(.body | contains(\"railway-test:evidence head=<HEAD_SHA>\")) | .id"
   ```

   - If found, update it instead of adding a duplicate for the same head:

     ```bash
     gh api repos/<REPO>/issues/comments/<COMMENT_ID> -F body=@<evidence-file>
     ```

   - Otherwise create a new comment:

     ```bash
     gh pr comment <PR> --repo <REPO> --body-file <evidence-file>
     ```

   A new deployed head gets a new evidence comment; the same head is never
   published twice.
5. Capture the resulting comment URL and include it in the final response.

Do not report the Railway test as fully delivered until the comment URL is
available. If GitHub authentication, permissions, or connectivity prevents
posting, retry once, then report the publication failure explicitly and return
the complete copy-ready Markdown evidence. Never claim PR evidence was posted
without verifying the resulting comment URL.
