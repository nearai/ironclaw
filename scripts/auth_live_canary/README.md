# Live Auth Canary

This runner starts a fresh local IronClaw instance, seeds real provider
credentials into a clean database, and verifies provider-backed auth through:

Use [scripts/live-canary/run.sh](../live-canary/run.sh)
as the top-level entrypoint for scheduled and manual lane dispatch. This file
documents the underlying executor for the `auth-live-seeded` lane.

- `/v1/responses`
- the browser gateway UI via Playwright

It uses the existing mock LLM from `tests/e2e/mock_llm.py` only for deterministic
tool selection. The thing under test is the real provider auth/runtime path,
not model behavior.

## What It Proves

- a brand-new machine can build and start the gateway
- seeded credentials are accepted on a fresh database
- extension install + activation succeeds without manual recovery
- the Responses API can execute provider-backed tools
- the browser UI can execute provider-backed tools
- Google refresh still works when the stored access token is deliberately expired

## Current Provider Cases

- `gmail`
  Uses `google_oauth_token`
  Runs through Responses API and browser
- `google_calendar`
  Uses `google_oauth_token`
  Runs through Responses API
- `github`
  Uses `github_token`
  Runs through Responses API and browser
- `notion`
  Uses `mcp_notion_access_token`
  Runs through Responses API

## Setup

See the canonical live-canary account and credential guide in
[scripts/live-canary/ACCOUNTS.md](../live-canary/ACCOUNTS.md).

Copy the example config and fill in the real test credentials:

```bash
cd scripts/auth_live_canary
cp config.example.env config.env
set -a && source config.env && set +a
```

For Google refresh verification you should provide both:

- `AUTH_LIVE_GOOGLE_ACCESS_TOKEN`
- `AUTH_LIVE_GOOGLE_REFRESH_TOKEN`

along with:

- `GOOGLE_OAUTH_CLIENT_ID`
- `GOOGLE_OAUTH_CLIENT_SECRET`

The runner will seed the token into the clean DB, then backdate its expiry so
the first Google-backed probe has to refresh.

## Usage

From the repo root:

```bash
python3 scripts/auth_live_canary/run_live_canary.py
```

Run only selected providers:

```bash
python3 scripts/auth_live_canary/run_live_canary.py --case gmail --case github
```

CI-style fresh-machine install:

```bash
python3 scripts/auth_live_canary/run_live_canary.py --playwright-install with-deps
```

Reuse an existing venv and binary:

```bash
python3 scripts/auth_live_canary/run_live_canary.py \
  --skip-python-bootstrap \
  --skip-build
```

List the currently configured cases:

```bash
python3 scripts/auth_live_canary/run_live_canary.py --list-cases
```

## Artifacts

The runner writes JSON results to:

```text
artifacts/auth-live-canary/results.json
```

Browser failures also write screenshots into the same output directory.

## Important Boundary

This is the practical high-frequency live canary.

It does **not** automate the provider login UI on every run. Instead it seeds
known-good test credentials into a fresh local IronClaw instance and then
verifies that the runtime can still use and refresh them. That is the right
shape for hourly checks because it catches:

- bad secret persistence
- broken refresh logic
- bad redirect/client config shipped with the runtime
- provider-side token validation changes
- silent regressions in extension activation or tool execution

If you want a full provider-consent browser automation pass too, that should be
a separate lower-frequency suite.
