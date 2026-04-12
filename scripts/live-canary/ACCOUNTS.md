# Live Canary Accounts, Secrets, and Provider Setup

This is the canonical account and credential guide for the live canary system.
Use it when adding or rotating providers for:

- `auth-live-seeded`
- `auth-browser-consent`
- any future auth canary lane added under `scripts/live-canary/run.sh`

The shared implementation for auth lanes lives in:

- [scripts/live_canary/common.py](/home/illia/ironclaw/scripts/live_canary/common.py)
- [scripts/live_canary/auth_registry.py](/home/illia/ironclaw/scripts/live_canary/auth_registry.py)
- [scripts/live_canary/auth_runtime.py](/home/illia/ironclaw/scripts/live_canary/auth_runtime.py)

When adding a new provider, the expected path is:

1. add its case entry in `scripts/live_canary/auth_registry.py`
2. reuse the shared setup/runtime helpers
3. document its required account material here

## Lane Model

The auth canaries split into two live-provider styles.

### `auth-live-seeded`

This lane starts a fresh local IronClaw instance and seeds known-good provider
credentials into the clean database.

Use it for:

- hourly or frequent live checks
- refresh-token coverage
- stable provider runtime probes

### `auth-browser-consent`

This lane starts with no provider tokens in IronClaw, opens the real provider
OAuth flow in Playwright, completes browser consent, then verifies both browser
chat and `/v1/responses`.

Use it for:

- nightly or pre-release checks
- redirect URI and consent UI validation
- provider login/consent drift detection

## Operating Rules

- Use dedicated test accounts only.
- Do not reuse personal or production accounts.
- Keep one provider account or workspace per integration where possible.
- Keep scopes narrow and fixtures disposable.
- Prefer read-only or low-risk probes.
- Keep one stable fixture per provider so failures are easy to classify.

## GitHub Actions Environments

The unified workflow uses two auth-specific environments:

- `auth-live-canary` for `auth-live-seeded`
- `auth-browser-canary` for `auth-browser-consent`

Only providers with populated secrets are executed.

## Shared Provider Fixtures

Every provider should have one stable, low-risk probe target.

- Gmail: one inbox with at least one readable message or draft
- Google Calendar: one calendar with at least one upcoming event
- Google Drive: one accessible stable fixture query or file set
- Google Docs: one readable fixture document
- Google Sheets: one readable fixture spreadsheet/range
- Google Slides: one readable fixture presentation
- GitHub: one dedicated repository with one stable issue
- Brave Search: one low-volume API key shared by Web Search and LLM Context
- Slack: one workspace with a bot token that can list channels
- Telegram: one logged-in user-mode MTProto session
- Composio: one API key with at least one readable connected-account state
- Notion: one test workspace with one searchable page or database row
- Linear: one workspace with one searchable issue

## Seeded Lane Secrets

These are read by `scripts/auth_live_canary/run_live_canary.py`.

### Google

Required when enabling Gmail or Calendar probes:

- `GOOGLE_OAUTH_CLIENT_ID`
- `GOOGLE_OAUTH_CLIENT_SECRET`
- `AUTH_LIVE_GOOGLE_ACCESS_TOKEN`
- `AUTH_LIVE_GOOGLE_REFRESH_TOKEN`
- `AUTH_LIVE_GOOGLE_SCOPES`
- `AUTH_LIVE_FORCE_GOOGLE_REFRESH`

Notes:

- `AUTH_LIVE_GOOGLE_ACCESS_TOKEN` is required if a refresh token is provided.
- The runner seeds the token, then can deliberately expire the access token so
  refresh is exercised on first use.
- Gmail and Calendar share `google_oauth_token`.

Recommended scopes:

- `https://www.googleapis.com/auth/gmail.modify`
- `https://www.googleapis.com/auth/gmail.compose`
- `https://www.googleapis.com/auth/calendar.events`
- `https://www.googleapis.com/auth/drive`
- `https://www.googleapis.com/auth/documents`
- `https://www.googleapis.com/auth/spreadsheets`
- `https://www.googleapis.com/auth/presentations`

Required only for the combined `ops_workflow` case:

- `AUTH_LIVE_GOOGLE_DOC_ID`
- `AUTH_LIVE_GOOGLE_SHEET_ID`
- `AUTH_LIVE_GOOGLE_SLIDES_ID`

Optional:

- `AUTH_LIVE_GOOGLE_DRIVE_QUERY` (defaults to `trashed = false`)
- `AUTH_LIVE_GOOGLE_SHEET_RANGE` (defaults to `A1:Z10`)

### GitHub

Required:

- `AUTH_LIVE_GITHUB_TOKEN`
- `AUTH_LIVE_GITHUB_OWNER`
- `AUTH_LIVE_GITHUB_REPO`
- `AUTH_LIVE_GITHUB_ISSUE_NUMBER`

Use a dedicated low-privilege token that can read the fixture issue.

### Notion

Required:

- `AUTH_LIVE_NOTION_ACCESS_TOKEN`
- `AUTH_LIVE_NOTION_QUERY`

Optional:

- `AUTH_LIVE_NOTION_REFRESH_TOKEN`

The probe should match a stable test page or database entry.

### Linear

Required:

- `AUTH_LIVE_LINEAR_ACCESS_TOKEN`
- `AUTH_LIVE_LINEAR_QUERY`

Optional:

- `AUTH_LIVE_LINEAR_REFRESH_TOKEN`
- `AUTH_LIVE_LINEAR_TOOL_NAME`
- `AUTH_LIVE_LINEAR_TOOL_ARGS_JSON`

Use `AUTH_LIVE_LINEAR_TOOL_NAME` and `AUTH_LIVE_LINEAR_TOOL_ARGS_JSON` if the
Linear MCP server's tool name or argument schema changes. The default tool name
is `linear_search_issues`, with arguments `{"query": "<AUTH_LIVE_LINEAR_QUERY>"}`.

### Brave Search

Required for Web Search and LLM Context probes:

- `AUTH_LIVE_BRAVE_API_KEY`

### Slack

Required:

- `AUTH_LIVE_SLACK_BOT_TOKEN`

The combined workflow uses `list_channels` to avoid posting on every scheduled
run.

### Telegram

Required:

- `AUTH_LIVE_TELEGRAM_API_ID`
- `AUTH_LIVE_TELEGRAM_API_HASH`
- `AUTH_LIVE_TELEGRAM_SESSION_JSON`

The seeded runner writes these to `telegram/api_id`, `telegram/api_hash`, and
`telegram/session.json` in the fresh workspace before activating the tool. The
combined workflow uses `get_me` to avoid sending messages on every scheduled
run.

### Composio

Required:

- `AUTH_LIVE_COMPOSIO_API_KEY`

The combined workflow uses `connected_accounts`, which is read-only.

### Combined Ops Workflow

Run this after provisioning every fixture above:

```bash
LANE=auth-live-seeded CASES=ops_workflow scripts/live-canary/run.sh
```

It installs and activates Gmail, Google Calendar, Google Drive, Google Docs,
Google Sheets, Google Slides, GitHub, Web Search, LLM Context, Slack, Telegram,
Composio, Notion, and Linear, then dispatches one deterministic `/v1/responses`
turn that calls every tool.

## Browser-Consent Lane Secrets

These are read by `scripts/auth_browser_canary/run_browser_canary.py`.

### Preferred Account Input

Use Playwright storage-state JSON files per provider. This is more stable than
typing credentials into provider UIs on every run.

Per-provider env vars:

- `AUTH_BROWSER_GOOGLE_STORAGE_STATE_PATH`
- `AUTH_BROWSER_GITHUB_STORAGE_STATE_PATH`
- `AUTH_BROWSER_NOTION_STORAGE_STATE_PATH`

Fallback username/password env vars are supported, but should be treated as a
last resort:

- `AUTH_BROWSER_GOOGLE_USERNAME`, `AUTH_BROWSER_GOOGLE_PASSWORD`
- `AUTH_BROWSER_GITHUB_USERNAME`, `AUTH_BROWSER_GITHUB_PASSWORD`
- `AUTH_BROWSER_NOTION_USERNAME`, `AUTH_BROWSER_NOTION_PASSWORD`

### OAuth App Credentials

Google browser auth requires:

- `GOOGLE_OAUTH_CLIENT_ID`
- `GOOGLE_OAUTH_CLIENT_SECRET`

GitHub browser auth requires:

- `GITHUB_OAUTH_CLIENT_ID`
- `GITHUB_OAUTH_CLIENT_SECRET`

Notion currently relies on the provider-side OAuth metadata from the configured
MCP server and does not require separate client env vars here.

### GitHub Fixture Coordinates

GitHub browser verification also requires:

- `AUTH_BROWSER_GITHUB_OWNER`
- `AUTH_BROWSER_GITHUB_REPO`
- `AUTH_BROWSER_GITHUB_ISSUE_NUMBER`

## Capturing Playwright Storage State

From the repo root:

```bash
cd tests/e2e
. .venv/bin/activate
python - <<'PY'
import asyncio
from pathlib import Path
from playwright.async_api import async_playwright

TARGET_URL = "https://github.com/login"
OUTPUT = Path("github-storage-state.json").resolve()

async def main():
    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=False)
        context = await browser.new_context()
        page = await context.new_page()
        await page.goto(TARGET_URL)
        print(f"Log in manually, then press Enter to save {OUTPUT}")
        input()
        await context.storage_state(path=str(OUTPUT))
        await browser.close()

asyncio.run(main())
PY
```

Provider URLs:

- Google: `https://accounts.google.com/`
- GitHub: `https://github.com/login`
- Notion: `https://www.notion.so/login`

## GitHub Actions Storage-State Secrets

For CI, encode each storage-state file as base64 and store it as a secret:

- `AUTH_BROWSER_GOOGLE_STORAGE_STATE_B64`
- `AUTH_BROWSER_GITHUB_STORAGE_STATE_B64`
- `AUTH_BROWSER_NOTION_STORAGE_STATE_B64`

Create the value locally:

```bash
base64 -w0 tests/e2e/github-storage-state.json
```

On macOS:

```bash
base64 < tests/e2e/github-storage-state.json | tr -d '\n'
```

The workflow decodes each secret into a temporary file and exports the matching
`*_STORAGE_STATE_PATH` variable before invoking the runner.

## Local Setup

Seeded lane:

```bash
cd scripts/auth_live_canary
cp config.example.env config.env
set -a && source config.env && set +a
cd ../..
python3 scripts/auth_live_canary/run_live_canary.py --list-cases
```

Browser-consent lane:

```bash
cd scripts/auth_browser_canary
cp config.example.env config.env
set -a && source config.env && set +a
cd ../..
python3 scripts/auth_browser_canary/run_browser_canary.py --list-cases
```

Canonical wrapper usage:

```bash
LANE=auth-live-seeded scripts/live-canary/run.sh
LANE=auth-browser-consent scripts/live-canary/run.sh
```

## Failure Triage

Classify failures first:

- credential failure: token revoked, scope missing, account disabled
- provider failure: quota, rate limit, consent UI change, policy change
- IronClaw failure: secret persistence, refresh, extension activation, auth injection, callback handling

Check first:

- `artifacts/live-canary/<lane>/<provider>/<timestamp>/results.json`
- workflow logs
- browser screenshots for browser-consent failures
- whether the test account can still perform the small fixture operation directly

## Rotation Checklist

- Mint or capture replacement credentials for the dedicated test account.
- Update the matching GitHub Actions environment secrets.
- Run only the affected lane and provider manually.
- Confirm both browser and `/v1/responses` verification pass again where applicable.
