# Live Canary Regression Lanes

IronClaw has two complementary regression systems:

- deterministic CI, which exercises committed tests and mock-backed auth flows without depending on third-party providers;
- live canaries, which use real provider credentials or real provider consent pages to catch auth drift, refresh failures, redirect breakage, and fresh-machine regressions.

This branch now uses the same top-level structure as the upstream live-canary
design:

- `.github/workflows/test.yml` for the normal blocking test lanes;
- `.github/workflows/live-canary.yml` for scheduled and manual canary lanes;
- `scripts/live-canary/run.sh` for lane dispatch;
- `scripts/live-canary/scrub-artifacts.sh` for artifact scanning;
- `scripts/live-canary/README.md` for local commands and GitHub setup.

The underlying executors are still auth-focused:

- `scripts/auth_canary/run_canary.py`
- `scripts/auth_live_canary/run_live_canary.py`
- `scripts/auth_browser_canary/run_browser_canary.py`

## Lane Summary

| Lane | Scope | Runner | Trigger | Blocking |
| --- | --- | --- | --- | --- |
| `auth-smoke` | Fresh-machine mock-backed auth smoke: hosted OAuth, MCP OAuth, and multi-user MCP isolation | GitHub-hosted | Hourly and manual | No |
| `auth-full` | Larger mock-backed auth matrix including failure and refresh cases | GitHub-hosted | Manual | No |
| `auth-channels` | WASM channel auth diagnostic lane | GitHub-hosted | Manual | No |
| `auth-live-seeded` | Real-provider runtime checks using seeded tokens against a clean DB | GitHub-hosted | Hourly and manual | No |
| `auth-browser-consent` | Real browser-consent OAuth using Playwright against provider login UIs | GitHub-hosted | Nightly and manual | No |

## Required Repository Configuration

### Mock-backed auth lane

No provider credentials are required for `auth-smoke`, `auth-full`, or
`auth-channels`.

### Seeded live-provider lane

Secrets are documented in
[scripts/auth_live_canary/ACCOUNTS.md](/home/illia/ironclaw/scripts/auth_live_canary/ACCOUNTS.md).

Current provider material includes:

- Google OAuth client credentials and seeded access/refresh tokens
- GitHub seeded token plus a stable issue fixture
- Notion seeded access token and a stable query fixture

### Browser-consent lane

Secrets and storage-state material are documented in
[scripts/auth_browser_canary/ACCOUNTS.md](/home/illia/ironclaw/scripts/auth_browser_canary/ACCOUNTS.md).

Current provider material includes:

- Google OAuth app credentials plus browser storage state
- GitHub OAuth app credentials plus browser storage state and issue fixture
- Notion browser storage state

## Commands

Run the auth smoke lane locally:

```bash
LANE=auth-smoke scripts/live-canary/run.sh
```

Run the seeded live-provider lane:

```bash
LANE=auth-live-seeded scripts/live-canary/run.sh
```

Run the browser-consent lane:

```bash
LANE=auth-browser-consent scripts/live-canary/run.sh
```

Run selected provider cases only:

```bash
LANE=auth-live-seeded CASES=gmail,github scripts/live-canary/run.sh
LANE=auth-browser-consent CASES=google,github scripts/live-canary/run.sh
```

## Artifact Policy

Artifacts are written under `artifacts/live-canary/`.

Before upload, the workflow runs `scripts/live-canary/scrub-artifacts.sh`.
That script is a lightweight guardrail against uploading obvious token-shaped
strings from logs or result files.

The browser-consent and seeded live lanes may capture screenshots and JSON
results, but should not upload raw long-lived credential material.
