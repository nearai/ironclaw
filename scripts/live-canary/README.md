# Live Canary Local and GitHub Setup

This directory contains the runner scripts for the live regression lanes:

- `run.sh` dispatches named lanes and writes artifacts.
- `scrub-artifacts.sh` scans artifacts before upload.
- `upgrade-canary.sh` checks previous-release DB compatibility.

Run commands from the repository root:

```bash
cd /tmp/ironclaw-live-canary
```

## Local Secrets

For local live runs, put provider secrets in either your shell environment or
`~/.ironclaw/.env`. The live harness loads `~/.ironclaw/.env`, so that is the
cleanest option.

Example `~/.ironclaw/.env` for Anthropic:

```bash
DATABASE_BACKEND=libsql
LIBSQL_PATH=/Users/firatsertgoz/.ironclaw/ironclaw.db

LLM_BACKEND=anthropic
ANTHROPIC_API_KEY=sk-ant-...
ANTHROPIC_MODEL=claude-sonnet-4-6

ALLOW_LOCAL_TOOLS=true
AGENT_AUTO_APPROVE_TOOLS=true
```

You can also pass secrets inline for a one-off run:

```bash
LLM_BACKEND=anthropic \
ANTHROPIC_API_KEY=sk-ant-... \
ANTHROPIC_MODEL=claude-sonnet-4-6 \
LANE=public-smoke \
scripts/live-canary/run.sh
```

## Local Commands

Replay committed live traces without LLM calls:

```bash
LANE=deterministic-replay scripts/live-canary/run.sh
```

Run the public live smoke lane:

```bash
LANE=public-smoke scripts/live-canary/run.sh
```

Run a specific persona:

```bash
LANE=persona-rotating SCENARIO=developer_full_workflow scripts/live-canary/run.sh
```

Use the UTC day-of-week persona rotation:

```bash
LANE=persona-rotating SCENARIO=auto scripts/live-canary/run.sh
```

Run the private OAuth lane:

```bash
LANE=private-oauth scripts/live-canary/run.sh
```

For private OAuth, the local `~/.ironclaw/ironclaw.db` must already contain:

```text
google_oauth_token
google_oauth_token_refresh_token
google_oauth_token_scopes
```

The usual setup is to run IronClaw normally once, complete Google Drive auth
with a dedicated test Google account, and then run the canary.

Run the provider matrix with Anthropic:

```bash
LLM_BACKEND=anthropic \
ANTHROPIC_API_KEY=sk-ant-... \
ANTHROPIC_MODEL=claude-sonnet-4-6 \
LANE=provider-matrix \
PROVIDER=anthropic \
PROVIDER_TEST_TARGET=e2e_live \
SCENARIO=zizmor_scan \
scripts/live-canary/run.sh
```

Run the provider matrix with an OpenAI-compatible endpoint:

```bash
LLM_BACKEND=openai_compatible \
LLM_BASE_URL=https://your-provider.example/v1 \
LLM_API_KEY=... \
LLM_MODEL=your-model \
LANE=provider-matrix \
PROVIDER=openai-compatible \
PROVIDER_TEST_TARGET=e2e_live_mission \
SCENARIO=mission_daily_news_digest_with_followup \
scripts/live-canary/run.sh
```

Run an upgrade canary:

```bash
LANE=upgrade-canary \
PREVIOUS_REF=v0.1.2 \
CURRENT_REF=HEAD \
scripts/live-canary/run.sh
```

Artifacts are written under:

```text
artifacts/live-canary/<lane>/<provider>/<timestamp>/
```

## GitHub Secrets and Variables

Add repository secrets under:

```text
Settings -> Secrets and variables -> Actions -> Secrets
```

Required secrets:

```text
LIVE_ANTHROPIC_API_KEY
LIVE_OPENAI_COMPATIBLE_API_KEY
LIVE_OPENAI_COMPATIBLE_BASE_URL
```

Use a secret for `LIVE_OPENAI_COMPATIBLE_BASE_URL` because internal provider
URLs can expose infrastructure details.

Add repository variables under:

```text
Settings -> Secrets and variables -> Actions -> Variables
```

Variables:

```text
LIVE_ANTHROPIC_MODEL=claude-sonnet-4-6
LIVE_OPENAI_COMPATIBLE_MODEL=<your model>
LIVE_CANARY_PRIVATE_OAUTH_ENABLED=false
```

Set `LIVE_CANARY_PRIVATE_OAUTH_ENABLED=true` only after the self-hosted runner
is ready.

## Self-Hosted OAuth Runner

The private OAuth lane requires a self-hosted runner with these labels:

```text
self-hosted
ironclaw-live
```

That runner needs a dedicated `~/.ironclaw/.env` and `~/.ironclaw/ironclaw.db`
with a test Google account authenticated. Do not use a maintainer's personal
Google account.

The workflow uploads only summary artifacts for this lane and runs the strict
artifact scrubber before upload. Raw OAuth logs and trace files should not be
uploaded.
