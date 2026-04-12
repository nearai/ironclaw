# Live Canary Accounts And Credentials

This file describes the real provider accounts, app configuration, and secrets
needed for [run_live_canary.py](/home/illia/ironclaw/scripts/auth_live_canary/run_live_canary.py).

The live canary does **not** log in to providers interactively on every run.
It starts a fresh local IronClaw instance and seeds known-good credentials into
that clean database. The accounts below must therefore be stable, low-risk test
accounts with minimal permissions.

## Operating Rules

- Use dedicated test accounts only. Do not reuse personal or production accounts.
- Keep each provider isolated to a separate account or workspace.
- Grant the narrowest scopes needed for the canary probes.
- Prefer read-only probes and disposable data.
- Keep one clearly named test artifact per provider so the canary always has a
  known target.
- Rotate and re-issue credentials when ownership changes.

## Required GitHub Actions Environment

The scheduled workflow expects an environment named `auth-live-canary`.

Configure these secrets there:

- `GOOGLE_OAUTH_CLIENT_ID`
- `GOOGLE_OAUTH_CLIENT_SECRET`
- `AUTH_LIVE_GOOGLE_ACCESS_TOKEN`
- `AUTH_LIVE_GOOGLE_REFRESH_TOKEN`
- `AUTH_LIVE_GOOGLE_SCOPES`
- `AUTH_LIVE_FORCE_GOOGLE_REFRESH`
- `AUTH_LIVE_GITHUB_TOKEN`
- `AUTH_LIVE_GITHUB_OWNER`
- `AUTH_LIVE_GITHUB_REPO`
- `AUTH_LIVE_GITHUB_ISSUE_NUMBER`
- `AUTH_LIVE_NOTION_ACCESS_TOKEN`
- `AUTH_LIVE_NOTION_REFRESH_TOKEN`
- `AUTH_LIVE_NOTION_QUERY`

Only the providers you want to run need to be populated. Empty providers are skipped.

## Google Setup

Google covers both `gmail` and `google_calendar` because they share
`google_oauth_token`.

Create:

- one dedicated Google account for canary use
- one Google Cloud project for the canary
- one OAuth client in that project

Required app config:

- enable Gmail API
- enable Google Calendar API
- set the OAuth consent screen for your org/test flow
- create an OAuth client compatible with the token you plan to mint

Recommended scopes:

- `https://www.googleapis.com/auth/gmail.modify`
- `https://www.googleapis.com/auth/gmail.compose`
- `https://www.googleapis.com/auth/calendar.events`

Recommended account fixtures:

- Gmail: keep at least one unread message in the inbox
- Calendar: keep at least one upcoming event on the primary calendar

Required secrets:

- `GOOGLE_OAUTH_CLIENT_ID`
- `GOOGLE_OAUTH_CLIENT_SECRET`
- `AUTH_LIVE_GOOGLE_ACCESS_TOKEN`
- `AUTH_LIVE_GOOGLE_REFRESH_TOKEN`

Notes:

- The runner seeds the access token into a fresh DB and, by default, expires it
  before the first Google-backed probe so refresh is exercised.
- `AUTH_LIVE_GOOGLE_ACCESS_TOKEN` is required if you set a refresh token.
- `AUTH_LIVE_GOOGLE_SCOPES` should match the scopes granted to the stored token.

## GitHub Setup

Create:

- one dedicated GitHub user or bot-style service account
- one dedicated test repository
- one known issue in that repository for the canary to read

Recommended permissions for the token:

- minimal read permission for issues and repo metadata
- if using a classic PAT, avoid broad write scopes unless a future canary needs them

Required secrets:

- `AUTH_LIVE_GITHUB_TOKEN`
- `AUTH_LIVE_GITHUB_OWNER`
- `AUTH_LIVE_GITHUB_REPO`
- `AUTH_LIVE_GITHUB_ISSUE_NUMBER`

Recommended fixture:

- keep issue `AUTH_LIVE_GITHUB_ISSUE_NUMBER` open and stable
- avoid editing or deleting it during routine maintenance

## Notion Setup

The Notion canary uses the hosted MCP server entry `notion` and seeds
`mcp_notion_access_token` into the fresh local database.

Create:

- one dedicated Notion workspace or test area
- one dedicated Notion integration for the canary
- one small database or page set that can be searched safely

Required secrets:

- `AUTH_LIVE_NOTION_ACCESS_TOKEN`
- `AUTH_LIVE_NOTION_QUERY`

Optional:

- `AUTH_LIVE_NOTION_REFRESH_TOKEN` if your token strategy supports refresh

Recommended fixture:

- create at least one page or database row that always matches `AUTH_LIVE_NOTION_QUERY`

## Local Setup

For local runs:

```bash
cd scripts/auth_live_canary
cp config.example.env config.env
set -a && source config.env && set +a
cd ../..
python3 scripts/auth_live_canary/run_live_canary.py
```

Run selected providers only:

```bash
python3 scripts/auth_live_canary/run_live_canary.py --case gmail --case github
```

List which cases are configured from your current env:

```bash
python3 scripts/auth_live_canary/run_live_canary.py --list-cases
```

## Failure Triage

When a live canary starts failing, classify it first:

- credential problem: token revoked, expired, missing scope, account disabled
- provider problem: API behavior changed, quota/rate limit, tenant policy change
- IronClaw problem: extension activation, secret lookup, refresh, auth injection, browser flow, Responses API flow

Check first:

- `artifacts/auth-live-canary/results.json`
- workflow logs
- browser screenshots from the artifact bundle
- whether the provider test account can still perform the small manual operation directly

## Rotation Checklist

- Mint replacement credentials on the dedicated test account.
- Update the `auth-live-canary` environment secrets.
- Run the workflow manually for the affected provider only.
- Confirm both `/v1/responses` and browser checks pass again.
