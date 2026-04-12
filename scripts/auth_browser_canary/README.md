# Browser Auth Canary

This runner performs real browser-based auth against provider consent pages on a
fresh local IronClaw instance.

Use [scripts/live-canary/run.sh](/home/illia/ironclaw/scripts/live-canary/run.sh)
as the top-level entrypoint for scheduled and manual lane dispatch. This file
documents the underlying executor for the `auth-browser-consent` lane.

It is separate from `scripts/auth_live_canary/` on purpose:

- `auth_live_canary` seeds tokens into a fresh DB and verifies runtime use/refresh
- `auth_browser_canary` starts with no provider tokens and completes auth in the browser

## What It Covers

- Gmail / Google OAuth through the real provider page
- GitHub OAuth through the real provider page
- Notion MCP OAuth through the real provider page
- post-auth verification through browser chat
- post-auth verification through `/v1/responses`

## Credentials Model

This runner does **not** seed provider access tokens into IronClaw.

Instead it uses one of these for the provider account in Playwright:

1. preferred: `*_STORAGE_STATE_PATH`
2. fallback: provider username/password env vars

The preferred mode is a pre-authenticated Playwright storage-state file for the
dedicated test account. That is more stable than typing credentials on every run,
especially for Google.

In CI, the workflow expects base64-encoded storage-state secrets and writes them
to temporary files before invoking the runner.

See the canonical live-canary account and storage-state guide in
[scripts/live-canary/ACCOUNTS.md](/home/illia/ironclaw/scripts/live-canary/ACCOUNTS.md).

## Required OAuth App Configuration

The browser runner needs the provider OAuth app credentials in the environment
of the fresh IronClaw instance it starts:

- Google: `GOOGLE_OAUTH_CLIENT_ID`, `GOOGLE_OAUTH_CLIENT_SECRET`
- GitHub: `GITHUB_OAUTH_CLIENT_ID`, `GITHUB_OAUTH_CLIENT_SECRET`

Notion MCP uses provider-side OAuth discovery from the configured server and
does not require those client env vars here.

## Setup

```bash
cd scripts/auth_browser_canary
cp config.example.env config.env
set -a && source config.env && set +a
```

Then run:

```bash
cd ../..
python3 scripts/auth_browser_canary/run_browser_canary.py
```

Run a single provider:

```bash
python3 scripts/auth_browser_canary/run_browser_canary.py --case google
python3 scripts/auth_browser_canary/run_browser_canary.py --case github
python3 scripts/auth_browser_canary/run_browser_canary.py --case notion
```

List configured cases:

```bash
python3 scripts/auth_browser_canary/run_browser_canary.py --list-cases
```

## Recommended Operating Mode

Use this as a lower-frequency canary than the seeded-token runner.

Good uses:

- nightly
- pre-release
- after auth UI changes
- after provider app / redirect configuration changes

Less good uses:

- every few minutes
- every PR
- environments where provider anti-bot checks are likely to block Chromium

## Artifacts

Results go to:

```text
artifacts/auth-browser-canary/results.json
```

On browser failures the runner also writes screenshots to the same directory.
