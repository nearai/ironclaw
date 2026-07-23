# Live Canary Regression Lanes

IronClaw has two complementary regression systems:

- deterministic CI, which replays committed tests and traces without depending
  on real third-party providers for the main blocking path;
- live canaries, which use selected real providers and LLM lanes to catch
  provider drift, refresh failures, release upgrade problems, and regressions
  that mocks will miss.

The implementation lives in:

- `.github/workflows/reborn-tests.yml` / `.github/workflows/reborn-e2e.yml` for the normal blocking test lanes (the v1 `test.yml` was removed under Tier B);
- `.github/workflows/live-canary.yml` for scheduled and manual live lanes;
- `scripts/live-canary/run.sh` for lane dispatch;
- `scripts/live-canary/scrub-artifacts.sh` for artifact scanning;
- `scripts/live-canary/upgrade-canary.sh` for previous-release upgrade checks.

Retired auth and workflow runner coverage is preserved by
`scripts/live-canary/MIGRATION.md`. New live product-auth checks belong in the
Reborn WebUI v2 QA lane; deterministic product contracts belong in the
serve-backed Python E2E inventory.

## Lane Summary

| Lane | Scope | Runner | Trigger | Blocking |
| --- | --- | --- | --- | --- |
| `deterministic-replay` | Deterministic Rust integration replay | GitHub-hosted | Manual | No |
| `public-smoke` | Real LLM plus public tools such as `zizmor_scan` and mission digest | GitHub-hosted | Daily and manual | Opens issue on scheduled failure |
| `persona-rotating` | Real LLM multi-turn persona workflow, one persona per day | GitHub-hosted | Daily and manual | Opens issue on scheduled failure |
| `private-oauth` | Google Drive auth gate and transparent refresh against a dedicated test account | Self-hosted `ironclaw-live` runner | Manual; scheduled only when enabled | Opens issue on scheduled failure |
| `provider-matrix` | Same live behavior against multiple provider adapters | GitHub-hosted | Weekly and manual | Opens issue on scheduled failure |
| `release-public-full` | Full public live suite for release candidates | GitHub-hosted | Manual | Release checklist gate |
| `upgrade-canary` | Previous release DB opened by current checkout | GitHub-hosted | Manual | Release checklist gate |
| `reborn-webui-v2-live-qa` | Shipping serve binary against selected real product integrations | GitHub-hosted or approved live environment | Scheduled, PR-gated, and manual | Environment-gated for PRs |

## Required Repository Configuration

### Public live LLM lanes

Secrets:

- `LIVE_ANTHROPIC_API_KEY`
- `LIVE_OPENAI_COMPATIBLE_API_KEY`
- `LIVE_OPENAI_COMPATIBLE_BASE_URL`

Variables:

- `LIVE_ANTHROPIC_MODEL`
- `LIVE_OPENAI_COMPATIBLE_MODEL`
- `LIVE_CANARY_PRIVATE_OAUTH_ENABLED`

### Reborn WebUI v2 Slack lane

The Reborn WebUI v2 live QA runner must not write legacy `[slack]` setup fields
into `config.toml`. The generated Reborn config only enables Slack:

```toml
[slack]
enabled = true
```

Bot installation setup is applied headlessly after `ironclaw serve`
boots by calling `PUT /api/webchat/v2/channels/slack/setup` with the WebUI
operator bearer token. Required repository variables:

- `REBORN_WEBUI_V2_LIVE_QA_SLACK_INSTALLATION_ID`
- `REBORN_WEBUI_V2_LIVE_QA_SLACK_TEAM_ID`
- `REBORN_WEBUI_V2_LIVE_QA_SLACK_API_APP_ID`
- `REBORN_WEBUI_V2_LIVE_QA_SLACK_ROUTE_USER_ID`

Required secrets:

- `IRONCLAW_REBORN_SLACK_SIGNING_SECRET`
- `IRONCLAW_REBORN_SLACK_BOT_TOKEN`

Required for `qa_3a_slack_connect`, `qa_5a_slack_connect`, and
`qa_8a_slack_connect`, which assert both sides of the personal OAuth path:
the Slack OAuth start URL is generated from client credentials, and a real
live Slack user account is already bound in Reborn product-auth state:

- variable `REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_ID`
- secret `REBORN_WEBUI_V2_LIVE_QA_SLACK_OAUTH_CLIENT_SECRET`
- secret `AUTH_LIVE_SLACK_ACCESS_TOKEN`

`AUTH_LIVE_SLACK_ACCESS_TOKEN` must be a real Slack user token for the live QA
Slack user. The harness validates it with Slack `auth.test`, then seeds the
generated Reborn home with an encrypted `slack_personal` product-auth account.

## Commands

Run public live smoke locally:

```bash
IRONCLAW_LIVE_TEST=1 \
LLM_BACKEND=anthropic \
ANTHROPIC_API_KEY=... \
LANE=public-smoke \
scripts/live-canary/run.sh
```

Run a private OAuth lane on the dedicated runner:

```bash
LANE=private-oauth scripts/live-canary/run.sh
```

The former `auth-smoke`, `auth-full`, and `auth-channels` lanes are retired.
Run their `ironclaw serve` replacement scenarios from
`tests/e2e/scenarios/`; see `scripts/live-canary/MIGRATION.md`.

Live product-auth coverage now runs through the
`reborn-webui-v2-live-qa` lane; see `scripts/live-canary/MIGRATION.md`.

Browser-consent coverage now runs through the
`reborn-webui-v2-live-qa` lane; see `scripts/live-canary/MIGRATION.md`.

Run selected auth provider cases only:

```bash
LANE=reborn-webui-v2-live-qa CASES=qa_2a_gmail_connect,qa_4b_github_connect \
  scripts/live-canary/run.sh
```

## Artifact Policy

Artifacts are written under `artifacts/live-canary/`.

Before upload, the workflow runs `scripts/live-canary/scrub-artifacts.sh`.
That script is a guardrail against uploading obvious token-shaped strings from
logs or result files.

Private OAuth and product-auth lanes must not upload raw OAuth logs or
long-lived credential material.
