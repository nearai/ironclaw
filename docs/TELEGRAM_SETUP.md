# Telegram Channel Setup

> Version baseline: IronClaw v0.12.0 (`v0.12.0` tag snapshot)

This guide covers configuring the Telegram channel for IronClaw, including DM pairing for access control.

## Overview

The Telegram channel lets you interact with IronClaw via Telegram DMs and groups. It supports:

- **Webhook mode** (recommended): Instant delivery via tunnel
- **Polling mode**: No tunnel required; ~30s delay
- **DM pairing**: Approve unknown users before they can message the agent
- **Group mentions**: `@YourBot` or `/command` to trigger in groups

## Prerequisites

- IronClaw installed and configured (`ironclaw onboard`)
- A Telegram bot token from [@BotFather](https://t.me/BotFather)

## Quick Start

### 1. Create a Bot

1. Message [@BotFather](https://t.me/BotFather) on Telegram
2. Send `/newbot` and follow the prompts
3. Copy the bot token (e.g., `123456789:ABCdefGHIjklMNOpqrsTUVwxyz`)

### 2. Configure via Setup Wizard

```bash
ironclaw onboard
```

When prompted, enable the Telegram channel and paste your bot token. The wizard will:

- Validate the token
- Optionally configure a webhook secret
- Set up tunnel (if you want webhook mode)

### 3. (Optional) Configure Tunnel for Webhooks

For instant message delivery, expose your agent via a tunnel:

```bash
# ngrok
ngrok http 8080

# Cloudflare
cloudflared tunnel --url http://localhost:8080
```

Set the tunnel URL in settings or via `TUNNEL_URL` env var. Without a tunnel, the channel uses polling (~30s delay).

## DM Pairing

When an unknown user DMs your bot, they receive a pairing code. You must approve them before they can message the agent.

### Flow

1. Unknown user sends a message to your bot
2. Bot replies: `To pair with this bot, run: ironclaw pairing approve telegram ABC12345`
3. You run: `ironclaw pairing approve telegram ABC12345`
4. User is added to the allow list; future messages are delivered

### Commands

```bash
# List pending pairing requests
ironclaw pairing list telegram

# List as JSON
ironclaw pairing list telegram --json

# Approve a user by code
ironclaw pairing approve telegram ABC12345
```

### Configuration

Edit `~/.ironclaw/channels/telegram.capabilities.json` (or the config injected by the host):

| Option | Values | Default | Description |
|--------|--------|---------|-------------|
| `dm_policy` | `open`, `allowlist`, `pairing` | `pairing` | `open` = allow all; `allowlist` = config + approved only; `pairing` = allowlist + send pairing reply to unknown |
| `allow_from` | `["user_id", "username", "*"]` | `[]` | Pre-approved IDs/usernames. `*` allows everyone. |
| `owner_id` | Telegram user ID | `null` | When set, only this user can message (overrides dm_policy) |
| `bot_username` | Bot username (no @) | `null` | Used for mention detection in groups; when set, only strips this mention from messages |
| `respond_to_all_group_messages` | `true`/`false` | `false` | When true, respond to all group messages; when false, only @mentions and /commands |

## Manual Installation

If the channel isn't installed via the wizard:

```bash
# Build the Telegram channel (requires wasm32-wasip2 target)
rustup target add wasm32-wasip2
./channels-src/telegram/build.sh

# Install
mkdir -p ~/.ironclaw/channels
cp channels-src/telegram/telegram.wasm channels-src/telegram/telegram.capabilities.json ~/.ironclaw/channels/
```

## Secrets

The channel expects a secret named `telegram_bot_token`. Configure via:

- **Setup wizard**: Saves to encrypted secrets store
- **Environment**: `TELEGRAM_BOT_TOKEN=your_token`
- **Secrets store**: `ironclaw` CLI (if available)

## Webhook Secret (Optional)

For webhook validation, set `telegram_webhook_secret` in secrets. Telegram will send `X-Telegram-Bot-Api-Secret-Token` with each request; the host validates it before forwarding.

## Troubleshooting

### Messages not delivered

- **Polling mode**: Check logs for `getUpdates` errors. Ensure the bot token is valid.
- **Webhook mode**: Verify tunnel is running and `TUNNEL_URL` is correct. Telegram requires HTTPS.

### Pairing code not received

- Verify the channel can send messages (HTTP allowlist includes `api.telegram.org`)
- Check `dm_policy` is `pairing` (not `allowlist` which blocks without reply)

### Group mentions not working

- Set `bot_username` in config to your bot's username (e.g., `MyIronClawBot`)
- Ensure the message contains `@YourBot` or starts with `/`

### "Connection refused" when starting

- For webhook mode: Start your tunnel before `ironclaw run`
- For polling only: No tunnel needed; ignore tunnel-related warnings
