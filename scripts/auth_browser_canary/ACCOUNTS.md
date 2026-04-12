# Browser Canary Accounts And Session Material

This file describes how to provide real provider account information to
[run_browser_canary.py](/home/illia/ironclaw/scripts/auth_browser_canary/run_browser_canary.py).

Unlike the seeded-token live canary, this runner starts with an empty IronClaw
database and completes the provider consent flow in Playwright. That means you
must provide:

- OAuth app credentials for the provider, when required by the tool auth flow
- browser session material for the dedicated test account
- any stable fixture identifiers needed for the post-auth verification call

## Operating Rules

- Use dedicated test accounts only.
- Keep one account per provider where possible.
- Keep permissions narrow and fixtures disposable.
- Prefer storage-state files over typing credentials live.
- Do not reuse personal browser sessions.

## What The Runner Reads

The browser canary discovers providers from environment variables.

### Google

- `GOOGLE_OAUTH_CLIENT_ID`
- `GOOGLE_OAUTH_CLIENT_SECRET`
- `AUTH_BROWSER_GOOGLE_STORAGE_STATE_PATH`
- optional fallback: `AUTH_BROWSER_GOOGLE_USERNAME`, `AUTH_BROWSER_GOOGLE_PASSWORD`

### GitHub

- `GITHUB_OAUTH_CLIENT_ID`
- `GITHUB_OAUTH_CLIENT_SECRET`
- `AUTH_BROWSER_GITHUB_STORAGE_STATE_PATH`
- `AUTH_BROWSER_GITHUB_OWNER`
- `AUTH_BROWSER_GITHUB_REPO`
- `AUTH_BROWSER_GITHUB_ISSUE_NUMBER`
- optional fallback: `AUTH_BROWSER_GITHUB_USERNAME`, `AUTH_BROWSER_GITHUB_PASSWORD`

### Notion

- `AUTH_BROWSER_NOTION_STORAGE_STATE_PATH`
- optional fallback: `AUTH_BROWSER_NOTION_USERNAME`, `AUTH_BROWSER_NOTION_PASSWORD`

## Preferred Account Input: Playwright Storage State

The stable way to provide account information is a Playwright storage-state JSON
file for each provider. The runner loads that file into a fresh browser context
before starting the OAuth flow, so the provider already sees a logged-in test
account.

Create one file per provider, for example:

- `google.json`
- `github.json`
- `notion.json`

### How To Capture A Storage-State File

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

Repeat with:

- Google: `TARGET_URL = "https://accounts.google.com/"`
- GitHub: `TARGET_URL = "https://github.com/login"`
- Notion: `TARGET_URL = "https://www.notion.so/login"`

Then point the runner at the saved file:

```bash
export AUTH_BROWSER_GITHUB_STORAGE_STATE_PATH="$PWD/tests/e2e/github-storage-state.json"
```

## GitHub Actions Secret Shape

The scheduled workflow expects an environment named `auth-browser-canary`.

Configure these secrets there:

- `GOOGLE_OAUTH_CLIENT_ID`
- `GOOGLE_OAUTH_CLIENT_SECRET`
- `AUTH_BROWSER_GOOGLE_STORAGE_STATE_B64`
- `AUTH_BROWSER_GOOGLE_USERNAME`
- `AUTH_BROWSER_GOOGLE_PASSWORD`
- `GITHUB_OAUTH_CLIENT_ID`
- `GITHUB_OAUTH_CLIENT_SECRET`
- `AUTH_BROWSER_GITHUB_STORAGE_STATE_B64`
- `AUTH_BROWSER_GITHUB_OWNER`
- `AUTH_BROWSER_GITHUB_REPO`
- `AUTH_BROWSER_GITHUB_ISSUE_NUMBER`
- `AUTH_BROWSER_GITHUB_USERNAME`
- `AUTH_BROWSER_GITHUB_PASSWORD`
- `AUTH_BROWSER_NOTION_STORAGE_STATE_B64`
- `AUTH_BROWSER_NOTION_USERNAME`
- `AUTH_BROWSER_NOTION_PASSWORD`

The workflow base64-decodes each `*_STORAGE_STATE_B64` secret into a temporary
file and exports the matching `*_STORAGE_STATE_PATH` env var before invoking the
runner.

Create the base64 value locally like this:

```bash
base64 -w0 tests/e2e/github-storage-state.json
```

On macOS use:

```bash
base64 < tests/e2e/github-storage-state.json | tr -d '\n'
```

## Provider Fixture Requirements

### Google

- One dedicated Google account
- Gmail enabled with at least one readable message
- Calendar enabled with at least one upcoming event

### GitHub

- One dedicated GitHub OAuth app with the callback URLs IronClaw uses
- One dedicated GitHub test account authorized for that OAuth app
- One stable issue in a dedicated repository
- The issue coordinates stored in:
  - `AUTH_BROWSER_GITHUB_OWNER`
  - `AUTH_BROWSER_GITHUB_REPO`
  - `AUTH_BROWSER_GITHUB_ISSUE_NUMBER`

Recommended OAuth scopes:

- `repo`
- `workflow`
- `read:org`

### Notion

- One dedicated Notion workspace or teamspace
- One OAuth-capable integration
- One stable page or database entry matching the test query

## Local Usage

```bash
cd scripts/auth_browser_canary
cp config.example.env config.env
set -a && source config.env && set +a
cd ../..
python3 scripts/auth_browser_canary/run_browser_canary.py --list-cases
python3 scripts/auth_browser_canary/run_browser_canary.py --case github
```

If `--list-cases` does not print a provider, one of its required env vars is
missing.
