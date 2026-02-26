# Signal Channel Setup

This guide covers configuring the native Signal channel for IronClaw, using the signal-cli HTTP daemon.

## Overview

The Signal channel connects IronClaw to Signal messaging via a running **signal-cli** daemon. Unlike WASM-based channels (Telegram, Slack), the Signal channel is a native Rust implementation built directly into IronClaw.

It supports:

- **DM conversations**: Direct messages to the bot's Signal account
- **Group conversations**: Mentions in Signal groups
- **Tool approval workflow**: Interactive approve/deny prompts for tool execution
- **DM pairing**: Allowlist-based access control with optional pairing mode
- **Allowlist controls**: Phone numbers, UUIDs, or wildcard access

## Prerequisites

- IronClaw installed and configured (`ironclaw onboard`)
- A Signal account (phone number) dedicated to the bot
- [signal-cli](https://github.com/AsamK/signal-cli) installed and registered to that number
- The signal-cli daemon running in HTTP mode before IronClaw starts

## Quick Start

### 1. Install and Register signal-cli

Follow the [signal-cli installation guide](https://github.com/AsamK/signal-cli/wiki/DBus-service). Register your bot's phone number:

```bash
# Register (you'll receive an SMS verification code)
signal-cli -a +1234567890 register

# Verify
signal-cli -a +1234567890 verify <CODE>
```

### 2. Start the signal-cli HTTP Daemon

IronClaw connects to signal-cli's HTTP/JSON-RPC API. Start the daemon before IronClaw:

```bash
# Start daemon on localhost port 8080
signal-cli -a +1234567890 daemon --http 127.0.0.1:8080
```

The daemon exposes:
- `GET /api/v1/events` — SSE stream of incoming messages
- `POST /api/v1/rpc` — JSON-RPC endpoint for sending messages
- `GET /api/v1/check` — Health check

### 3. Configure IronClaw

Add Signal configuration to `~/.ironclaw/.env`:

```env
# Required
SIGNAL_HTTP_URL=http://127.0.0.1:8080
SIGNAL_ACCOUNT=+1234567890

# Optional: who can DM the bot (default: SIGNAL_ACCOUNT only)
SIGNAL_ALLOW_FROM=+1234567890,+0987654321

# Optional: group access (default: deny all groups)
SIGNAL_ALLOW_FROM_GROUPS=*

# Optional: DM policy (pairing | allowlist | open, default: pairing)
SIGNAL_DM_POLICY=pairing

# Optional: Group policy (allowlist | open | disabled, default: allowlist)
SIGNAL_GROUP_POLICY=allowlist
```

### 4. Start IronClaw

```bash
ironclaw
```

IronClaw will connect to the signal-cli daemon and begin listening for messages.

## Configuration Reference

### Required Variables

| Variable | Description | Example |
|----------|-------------|---------|
| `SIGNAL_HTTP_URL` | Base URL of signal-cli HTTP daemon | `http://127.0.0.1:8080` |
| `SIGNAL_ACCOUNT` | Bot's E.164 phone number | `+1234567890` |

### Optional Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SIGNAL_ALLOW_FROM` | `SIGNAL_ACCOUNT` | Comma-separated DM allowlist (phone/UUID/`*`) |
| `SIGNAL_ALLOW_FROM_GROUPS` | *(empty — deny all)* | Group IDs or `*` to allow all groups |
| `SIGNAL_DM_POLICY` | `pairing` | DM access policy: `open`, `allowlist`, or `pairing` |
| `SIGNAL_GROUP_POLICY` | `allowlist` | Group access policy: `disabled`, `allowlist`, or `open` |
| `SIGNAL_GROUP_ALLOW_FROM` | *(inherits `SIGNAL_ALLOW_FROM`)* | Group sender allowlist |
| `SIGNAL_IGNORE_ATTACHMENTS` | `false` | Skip messages containing only attachments |
| `SIGNAL_IGNORE_STORIES` | `true` | Skip Signal story messages |

### SIGNAL_ALLOW_FROM Format

Each entry in `SIGNAL_ALLOW_FROM` is one of:

| Format | Example | Description |
|--------|---------|-------------|
| E.164 phone number | `+1234567890` | Allow by phone number |
| Bare UUID | `a1b2c3d4-e5f6-7890-abcd-ef1234567890` | Allow by Signal UUID |
| UUID with prefix | `uuid:a1b2c3d4-e5f6-7890-abcd-ef1234567890` | Allow by Signal UUID (alternate form) |
| Wildcard | `*` | Allow everyone (pairing requests still sent in `pairing` mode) |

### DM Policy Modes

| Mode | Behavior |
|------|----------|
| `open` | All DMs accepted regardless of `SIGNAL_ALLOW_FROM` |
| `allowlist` | Only senders in `SIGNAL_ALLOW_FROM` are accepted; others silently ignored |
| `pairing` | Allowlist-based; unknown senders receive a pairing code prompt |

### Group Policy Modes

| Mode | Behavior |
|------|----------|
| `disabled` | All group messages ignored |
| `allowlist` | Only groups in `SIGNAL_ALLOW_FROM_GROUPS` accepted |
| `open` | All groups accepted (respects `SIGNAL_ALLOW_FROM_GROUPS` if set) |

## Tool Approval Workflow

When IronClaw needs to use a tool (e.g., write a file, execute code), it sends an approval prompt to the Signal conversation:

```
🔧 Tool Request
Tool: write_file
Description: Write content to a file

Parameters:
{
  "path": "/home/user/notes.txt",
  "content": "Meeting notes..."
}

Reply: yes/approve/ok (once) | always (session) | no/deny/reject (block)
```

Supported responses:
- **Approve once**: `yes`, `y`, `approve`, `ok`, `/approve`, `/yes`, `/y`
- **Approve always**: `always`, `a`, `yes always`, `approve always`, `/always`, `/a`
- **Deny**: `no`, `n`, `deny`, `reject`, `cancel`, `/deny`, `/no`, `/n`

## Debug Mode

Toggle verbose tool status updates in the Signal conversation:

```
/debug
```

When debug mode is enabled, the bot sends status updates for each tool start, completion, and result (truncated to 500 characters).

## Troubleshooting

### Bot not receiving messages

1. Verify signal-cli daemon is running: `curl http://127.0.0.1:8080/api/v1/check`
2. Check `SIGNAL_HTTP_URL` matches the daemon's address
3. Confirm `SIGNAL_ACCOUNT` matches the registered phone number

### Unknown users blocked

If `SIGNAL_DM_POLICY=allowlist` and users can't message the bot, add their phone number or UUID to `SIGNAL_ALLOW_FROM`.

### Pairing mode — users don't receive a code

In `pairing` mode, unknown users receive a message with a pairing code. Approve them:

```bash
# List pending requests
ironclaw pairing list signal

# Approve by code
ironclaw pairing approve signal ABC12345
```

### Groups not working

Set `SIGNAL_ALLOW_FROM_GROUPS=*` to allow all groups, or specify group IDs. Set `SIGNAL_GROUP_POLICY=open` to accept messages from all group members.

## Architecture Notes

The Signal channel is a **native Rust implementation** (`src/channels/signal.rs`), not a WASM plugin. It:

- Connects to signal-cli's SSE event stream at `/api/v1/events` for real-time message delivery
- Sends responses via JSON-RPC calls to `/api/v1/rpc`
- Maintains an LRU cache (up to 10,000 entries) of reply targets to route responses to the correct DM or group
- Automatically reconnects to the daemon with exponential backoff on connection failure

Unlike WASM channels (Telegram, Slack), the Signal channel does not require installation via the extension registry — it activates automatically when `SIGNAL_HTTP_URL` and `SIGNAL_ACCOUNT` are set.
