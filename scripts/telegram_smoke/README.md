# Telegram Local Smoke Test

This is a real Telegram pre-release smoke test for engineers.

It does **not** mock Telegram. It logs in as a real Telegram user through `Telethon`,
talks to a dedicated test bot, and verifies that the bot replies through the
actual Telegram channel integration.

Use this when:
- the local Rust regression suite is already green
- you want a final confidence pass before a release
- you want to validate the real Telegram path with a human-controlled test account

Do **not** use your primary personal Telegram account. Use a dedicated test account.

## What It Covers

- DM round-trip
- edited message flow
- attachment message flow
- optional group `@mention` flow

By default the script only checks that the bot replies at all. If you run IronClaw
with a deterministic mock LLM, you can also require a fixed reply substring.

## Requirements

- Python 3.11+
- a local IronClaw instance already running with Telegram configured
- a Telegram bot dedicated to smoke testing
- a Telegram user account dedicated to smoke testing
- Telegram `api_id` and `api_hash` for that user account

## One-Time Python Setup

Reuse the existing E2E Python environment:

```bash
cd tests/e2e
python -m venv .venv
source .venv/bin/activate
pip install -e '.[telegram]'
```

`Telethon` is installed through the optional `telegram` dependency group.

## Configure The Smoke Runner

```bash
cd /path/to/ironclaw
cp scripts/telegram_smoke/config.example.env scripts/telegram_smoke/config.env
```

Fill in at least:

- `TG_SMOKE_API_ID`
- `TG_SMOKE_API_HASH`
- `TG_SMOKE_SESSION`
- `TG_SMOKE_BOT_USERNAME`

Optional but recommended:

- `TG_SMOKE_GROUP_TARGET`
- `TG_SMOKE_HEALTHCHECK_URL`
- `TG_SMOKE_EXPECT_SUBSTRING`

Load the config:

```bash
set -a
source scripts/telegram_smoke/config.env
set +a
```

## Start IronClaw

Run the exact build/config you want to validate before release.

Typical checklist:

1. Build the release candidate binary.
2. Start IronClaw with the Telegram channel enabled.
3. Ensure Telegram is already configured and activated.
4. Confirm the bot is reachable from your test Telegram account.

If you have a health endpoint available, set `TG_SMOKE_HEALTHCHECK_URL` so the
script fails fast when IronClaw is not up.

## First Login

On the first run, Telethon will prompt for:

- your phone number
- the Telegram login code
- your 2FA password, if enabled

It stores the session at `TG_SMOKE_SESSION`, so later runs are non-interactive.

## Run The Smoke Test

From the repo root:

```bash
source tests/e2e/.venv/bin/activate
set -a
source scripts/telegram_smoke/config.env
set +a

python scripts/telegram_smoke/run_smoke.py
```

Default cases:

- `dm`
- `edit`
- `attachment`

Run everything, including group mention:

```bash
python scripts/telegram_smoke/run_smoke.py --all
```

Run one specific case:

```bash
python scripts/telegram_smoke/run_smoke.py --case dm
python scripts/telegram_smoke/run_smoke.py --case group
python scripts/telegram_smoke/run_smoke.py --case attachment
```

List the available cases:

```bash
python scripts/telegram_smoke/run_smoke.py --list-cases
```

## Recommended Release Workflow

1. Run the local Rust Telegram regression suite first.
   ```bash
   cargo test --features integration --test telegram_auth_integration -- --nocapture
   ```
2. Start the release-candidate IronClaw build.
3. Run the real Telegram smoke script.
4. Treat any missing reply or timeout as a release blocker.

## Deterministic Replies

If you want strict assertions, run IronClaw against a deterministic mock LLM and
set:

```bash
export TG_SMOKE_EXPECT_SUBSTRING="SMOKE-ACK"
```

The script will then require every bot reply to contain that substring.

If you run against a real LLM, leave `TG_SMOKE_EXPECT_SUBSTRING` unset. In that
mode the script checks that Telegram traffic works end to end and that the bot
replies at all.

## Group Test Notes

The `group` case needs:

- `TG_SMOKE_GROUP_TARGET`
- `TG_SMOKE_BOT_USERNAME`

The bot must already be present in that group. The script sends an `@mention`
and requires the bot to reply to that exact message.

## Failure Modes

- `Timed out waiting for bot reply`
  - IronClaw is down, Telegram is misconfigured, the LLM path is stuck, or the
    bot is not responding in time.
- `SKIP group`
  - `TG_SMOKE_GROUP_TARGET` is not configured.
- Telegram RPC errors
  - bad login state, invalid chat target, bot missing from group, flood controls,
    or permissions problems.

## What This Does Not Replace

This is a smoke test, not the main regression suite.

Keep using:

- the Rust fake-API Telegram tests for PR regression coverage
- this script for local pre-release confidence with real Telegram
