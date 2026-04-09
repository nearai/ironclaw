# Slack Local Smoke Test

Exercises the real Slack WASM channel integration against a running IronClaw instance using live Slack API calls.

## Prerequisites

- **Python 3.11+**
- **Running IronClaw** instance with the Slack channel configured and activated
- **Slack App** with:
  - Bot token (`xoxb-`) with scopes: `chat:write`, `channels:history`, `groups:history`, `im:history`, `files:read`
  - User token (`xoxp-`) with scopes: `chat:write`, `files:write`, `channels:history`, `im:history`
- **Test bot** added to the DM channel (and optionally a public channel for mention tests)

## Setup

```bash
# From the repo root
cd tests/e2e
python -m venv .venv
source .venv/bin/activate
pip install -e '.[slack]'

# Configure
cd ../../scripts/slack_smoke
cp config.example.env config.env
# Edit config.env with your tokens and channel IDs
```

## Usage

```bash
# Load env vars
set -a && source config.env && set +a

# Run default cases (dm, attachment, thread)
python run_smoke.py

# Run all cases including mention
python run_smoke.py --all

# Run a specific case
python run_smoke.py --case dm
python run_smoke.py --case mention

# List available cases
python run_smoke.py --list-cases
```

## Smoke Cases

| Case | Default | Description |
|------|---------|-------------|
| `dm` | yes | Send DM via user token, poll for bot reply |
| `attachment` | yes | Upload file to DM channel, poll for bot reply |
| `thread` | yes | Send DM, wait for reply, reply in thread, verify bot continues in thread |
| `mention` | no | Send `<@BOT_USER_ID> msg` in public channel, poll for threaded reply |

## How It Works

Unlike Telegram (which uses a user-client library like Telethon), Slack smoke uses two tokens:

1. **User token** (`xoxp-`): Sends messages as a real Slack user, which triggers Slack to send webhook events to IronClaw
2. **Bot token** (`xoxb-`): Reads `conversations.history` / `conversations.replies` to find the bot's replies

Flow per case:
1. Send message via user token
2. Slack sends webhook event to IronClaw
3. IronClaw processes event and calls `chat.postMessage`
4. Smoke runner polls conversation history with bot token to find the reply

## Recommended Release Workflow

1. Run the Rust test suite: `cargo test --test slack_auth_integration`
2. Run E2E tests: `cd tests/e2e && pytest scenarios/test_slack_e2e.py -v`
3. Run this smoke test against a staging instance with real Slack

## Notes

- The `mention` case requires both `SLACK_SMOKE_PUBLIC_CHANNEL` and `SLACK_SMOKE_BOT_USER_ID`
- Use `SLACK_SMOKE_EXPECT_SUBSTRING` with a mock LLM for deterministic reply matching
- Exit codes: 0 = all passed, 1 = failure, 2 = config error
