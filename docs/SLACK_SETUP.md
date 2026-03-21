# Slack Channel Setup

This guide covers first-time setup of the Slack WASM channel for IronClaw in both Socket Mode and Webhook Mode.

## Table of Contents

- [Overview](#overview)
- [1. Install the Slack Channel](#1-install-the-slack-channel)
- [2. Create the Slack App](#2-create-the-slack-app)
- [3. OAuth Scopes](#3-oauth-scopes)
- [4. Configure Secrets](#4-configure-secrets)
- [5. Choose a Transport Mode](#5-choose-a-transport-mode)
- [Webhook Mode](#webhook-mode)
- [Socket Mode](#socket-mode)
- [6. Optional Access Control](#6-optional-access-control)
- [7. Behavior Scenarios](#7-behavior-scenarios)
- [8. Example Slack App Manifest](#8-example-slack-app-manifest)
- [Slash Commands and Interactivity](#slash-commands-and-interactivity)
- [Troubleshooting](#troubleshooting)
- [References](#references)

## Overview

The Slack channel lets you interact with IronClaw from Slack DMs, `@mentions`, and threaded replies. It supports:

- **Socket Mode**: no public inbound Slack webhook required
- **Webhook Mode**: Slack Events API over HTTPS
- **DM pairing**: approve unknown DM senders before they can message the agent
- **Owner restriction**: lock the channel to a single Slack user ID
- **Thread replies**: continue conversations in Slack threads

## Prerequisites

- IronClaw installed and configured
- A Slack app created at <https://api.slack.com/apps>
- A bot token (`xoxb-...`)
- For Webhook Mode: the Slack signing secret
- For Socket Mode: an app-level token (`xapp-...`)

## 1. Install the Slack Channel

If the Slack channel is not already installed:

```bash
rustup target add wasm32-wasip2
./channels-src/slack/build.sh

mkdir -p ~/.ironclaw/channels
cp channels-src/slack/slack.wasm channels-src/slack/slack.capabilities.json ~/.ironclaw/channels/
```

Enable WASM channels and verify the install:

```bash
ironclaw config set channels.wasm_channels_enabled true
ironclaw config set channels.wasm_channels '["slack"]'
ironclaw channels list --verbose
```

If you use a non-default channels directory:

```bash
ironclaw config set channels.wasm_channels_dir '"/absolute/path/to/channels"'
```

## 2. Create the Slack App

Create a new Slack app and configure:

1. **OAuth & Permissions**
   - Install the app to your workspace
   - Copy the **Bot User OAuth Token** (`xoxb-...`)
2. **Event Subscriptions**
   - Enable events
   - Subscribe to:
     - `app_mention`
     - `message.im`
   - Optionally subscribe to:
     - `message.channels` for threaded replies in public channels
     - `message.groups` for threaded replies in private channels
     - `message.mpim` for multi-person DMs
3. **Bot Token Scopes**
   - Add:
     - `app_mentions:read`
     - `chat:write`
     - `im:history`
   - Optionally add:
     - `channels:history` if using `message.channels`
     - `groups:history` if using `message.groups`
     - `mpim:history` if using `message.mpim`
     - `files:read` if you want shared files downloaded and processed
4. **Socket Mode**
   - Enable it only if you want Socket Mode
   - Create an app-level token
   - Copy the **App-Level Token** (`xapp-...`)
5. **Basic Information**
   - Copy the **Signing Secret** if you will use Webhook Mode

## 3. OAuth Scopes

Use the following bot token scopes for the current IronClaw Slack channel.

### Required Scopes

| Scope | Why it is needed in IronClaw |
|--------|------------------------------|
| `app_mentions:read` | Lets IronClaw receive `@mention` events in channels via `app_mention` |
| `chat:write` | Lets IronClaw send replies and DM pairing messages with `chat.postMessage` |
| `im:history` | Lets IronClaw receive direct messages from users via `message.im` |

### Optional Scopes

| Scope | Add it when | Why it is needed in IronClaw |
|--------|-------------|------------------------------|
| `channels:history` | You want threaded follow-up messages in public channels | Lets IronClaw receive `message.channels` events for public channel threads |
| `groups:history` | You want threaded follow-up messages in private channels | Lets IronClaw receive `message.groups` events for private channel threads |
| `mpim:history` | You use multi-person DMs | Lets IronClaw receive `message.mpim` events |
| `files:read` | You want the agent to process files shared in Slack messages | Lets IronClaw download shared Slack files from `url_private` links |

### Notes

- `files:read` is optional, but recommended if people will share attachments with the bot
- Slash commands are not required for the current IronClaw Slack channel
- Slack interactivity is not required for the current IronClaw Slack channel

## 4. Configure Secrets

The easiest setup path is:

```bash
ironclaw onboard --channels-only
```

Enable the Slack channel when prompted. The wizard reads the Slack channel manifest and saves the required secrets to IronClaw's secrets store.

### Using the IronClaw CLI

IronClaw currently does not provide a separate `secret set` CLI subcommand for Slack channel secrets.

For first-time Slack channel secret configuration, the supported `ironclaw` CLI path is:

```bash
ironclaw onboard --channels-only
```

Use it like this:

#### Webhook Mode

When the Slack setup prompts appear, provide:

- `slack_bot_token`
- `slack_signing_secret`

Leave the Socket Mode app token blank if you want Webhook Mode only.

#### Socket Mode

When the Slack setup prompts appear, provide:

- `slack_bot_token`
- `slack_app_token`

You can leave the signing secret blank if you are only using Socket Mode.

### Using Environment Variables Instead

If you prefer environment variables:

#### Webhook Mode

```bash
export SLACK_BOT_TOKEN='xoxb-...'
export SLACK_SIGNING_SECRET='...'
unset SLACK_APP_TOKEN
```

#### Socket Mode

```bash
export SLACK_BOT_TOKEN='xoxb-...'
export SLACK_APP_TOKEN='xapp-...'
```

## 5. Choose a Transport Mode

### Webhook Mode

Webhook Mode requires a public HTTPS URL reachable by Slack.

Configure:

- `slack_bot_token`
- `slack_signing_secret`
- `settings.socket_mode_enabled = false`
- `settings.event_subscriptions.request_url = https://your-public-host.example.com/webhook/slack`

Set the public URL in IronClaw:

```bash
ironclaw config set tunnel.public_url '"https://your-public-host.example.com"'
```

If you also need to enable IronClaw's HTTP listener explicitly:

```bash
ironclaw config set channels.http_enabled true
ironclaw config set channels.http_port 8080
```

Slack Events Request URL:

```text
https://your-public-host.example.com/webhook/slack
```

Run:

```bash
ironclaw run
```

### Socket Mode

Socket Mode does not need a public Slack Events Request URL.

Configure:

- `slack_bot_token`
- `slack_app_token`
- `settings.socket_mode_enabled = true`
- `settings.event_subscriptions.bot_events` for the events you want

Relevant IronClaw settings:

```bash
ironclaw config set channels.wasm_channels_enabled true
ironclaw config set channels.wasm_channels '["slack"]'
```

Run:

```bash
ironclaw run
```

## 6. Optional Access Control

The Slack channel defaults to `dm_policy = "pairing"`, which means unknown DM senders receive a pairing code and must be approved before future DMs are delivered.

Pairing commands:

```bash
ironclaw pairing list slack
ironclaw pairing list slack --json
ironclaw pairing approve slack ABC12345
```

Optional owner lock:

```bash
ironclaw config set channels.wasm_channel_owner_ids.slack 123456789
```

Replace `123456789` with the Slack user ID you want to allow. When set, only that Slack user can message the channel.

Slack-specific policy options live in `~/.ironclaw/channels/slack.capabilities.json` under `config`:

| Option | Values | Default | Description |
|--------|--------|---------|-------------|
| `owner_id` | Slack user ID string | `null` | When set, only this user can message the channel |
| `dm_policy` | `open`, `allowlist`, `pairing` | `pairing` | DM access policy |
| `allow_from` | `["U123", "*"]` | `[]` | Pre-approved Slack user IDs for DM access |

There is currently no dedicated `ironclaw config set ...` path for `dm_policy` or `allow_from`; those remain channel manifest config.

## 7. Behavior Scenarios

The Slack channel uses three main controls for access behavior:

- `channels.wasm_channel_owner_ids.slack`: host-level owner restriction
- `config.dm_policy` in `~/.ironclaw/channels/slack.capabilities.json`: DM access policy
- `config.allow_from` in `~/.ironclaw/channels/slack.capabilities.json`: pre-approved Slack user IDs for DMs

Important behavior rules:

- `dm_policy` applies only to DMs
- channel messages bypass DM pairing logic when no owner binding is set
- `owner_id` overrides everything and applies to DMs, public channels, and private channels

### Scenario 1: Default Configuration

Expected config:

- `channels.wasm_channel_owner_ids.slack` is unset
- `config.owner_id` is `null`
- `config.dm_policy` is `"pairing"`
- `config.allow_from` is `[]`

Expected behavior:

- DMs: unknown users get a pairing code and must be approved before future DMs are delivered
- Public Channels: users can talk to IronClaw by `@mention` and continue in thread replies
- Private Channels: users can talk to IronClaw the same way if the app is in the channel and Slack events/scopes are configured

Commands to inspect this scenario:

```bash
ironclaw config get channels.wasm_channel_owner_ids.slack
ironclaw pairing list slack
sed -n '1,220p' ~/.ironclaw/channels/slack.capabilities.json
cat ~/.ironclaw/slack-allowFrom.json
cat ~/.ironclaw/slack-pairing.json
```

What to expect:

- `ironclaw config get channels.wasm_channel_owner_ids.slack` returns `Setting not found`
- `ironclaw pairing list slack` usually shows no pending requests until someone sends a DM
- `slack.capabilities.json` shows `dm_policy` as `"pairing"`
- `slack-allowFrom.json` is empty or does not exist yet

### Scenario 2: Owner Only Allowed to Talk to IronClaw

Set the owner binding:

```bash
ironclaw config set channels.wasm_channel_owner_ids.slack 123456789
```

Expected behavior:

- DMs: only that Slack user can message IronClaw
- Public Channels: only that Slack user can `@mention` IronClaw or continue thread replies
- Private Channels: only that Slack user can `@mention` IronClaw or continue thread replies

What changes:

- DM pairing is effectively bypassed because the owner check runs first
- `allow_from` does not help non-owner users once an owner binding is set

Commands to inspect this scenario:

```bash
ironclaw config get channels.wasm_channel_owner_ids.slack
ironclaw pairing list slack
sed -n '1,220p' ~/.ironclaw/channels/slack.capabilities.json
```

What to expect:

- `ironclaw config get channels.wasm_channel_owner_ids.slack` returns the configured Slack user ID
- `ironclaw pairing list slack` is usually irrelevant because non-owner users are rejected before DM pairing logic matters
- `slack.capabilities.json` may still say `dm_policy: "pairing"`, but the owner restriction takes precedence

### Scenario 3: Owner Only Plus Specific Allowed Users

There are two distinct cases:

1. Host-level owner binding is set
2. `allow_from` contains additional Slack user IDs

Example owner binding:

```bash
ironclaw config set channels.wasm_channel_owner_ids.slack 123456789
```

Example manifest config:

```json
{
  "config": {
    "owner_id": null,
    "dm_policy": "allowlist",
    "allow_from": ["U11111111", "U22222222"]
  }
}
```

Expected behavior:

- DMs: only the owner can talk to IronClaw
- Public Channels: only the owner can talk to IronClaw
- Private Channels: only the owner can talk to IronClaw

Why:

- once the owner binding is set, the owner check runs first and blocks everyone else
- additional users in `allow_from` do not override the owner restriction

Commands to inspect this scenario:

```bash
ironclaw config get channels.wasm_channel_owner_ids.slack
sed -n '1,220p' ~/.ironclaw/channels/slack.capabilities.json
cat ~/.ironclaw/slack-allowFrom.json
```

What to expect:

- the owner binding is set in `ironclaw config`
- `allow_from` may contain extra users in the manifest or pairing store
- despite that, only the owner can successfully message the channel

### Can I Allow Specific Users Without Setting an Owner?

Yes, for DMs only.

Example manifest config:

```json
{
  "config": {
    "owner_id": null,
    "dm_policy": "allowlist",
    "allow_from": ["U11111111", "U22222222"]
  }
}
```

Behavior:

- DMs: only the listed users can message IronClaw
- Public Channels: any user can still talk to IronClaw by default
- Private Channels: any user can still talk to IronClaw by default

### Is There Channel-Level Pairing or Per-Channel User Access?

No. The current Slack channel does not support channel-level pairing or per-channel access rules.

There is currently no built-in way to say:

- user A may talk in channel X
- user B may talk in channel Y
- only approved users may talk in a specific public or private Slack channel

Current access controls are only:

- global owner restriction for the whole Slack channel
- DM policy (`open`, `allowlist`, `pairing`)
- DM allowlist / approved paired users

So today:

- DMs can be gated
- channel messages are globally allowed unless an owner binding is set
- there is no channel-specific allowlist or channel-specific pairing

## 8. Example Slack App Manifest

Replacement tokens:

- `<APP_NAME>`: Slack app display name
- `<APP_DESCRIPTION>`: short Slack app description
- `<APP_BACKGROUND_COLOR>`: hex color like `#003df5`
- `<EVENTS_REQUEST_URL>`: public Slack webhook URL, for example `https://example.com/webhook/slack`

### Minimal Webhook Mode Manifest JSON

```json
{
  "_metadata": {
    "major_version": 2,
    "minor_version": 1
  },
  "display_information": {
    "name": "<APP_NAME>",
    "description": "<APP_DESCRIPTION>",
    "background_color": "<APP_BACKGROUND_COLOR>"
  },
  "features": {
    "app_home": {
      "home_tab_enabled": false,
      "messages_tab_enabled": true,
      "messages_tab_read_only_enabled": false
    },
    "bot_user": {
      "display_name": "<APP_NAME>",
      "always_online": false
    }
  },
  "oauth_config": {
    "scopes": {
      "bot": [
        "app_mentions:read",
        "chat:write",
        "im:history"
      ]
    }
  },
  "settings": {
    "event_subscriptions": {
      "request_url": "<EVENTS_REQUEST_URL>",
      "bot_events": [
        "app_mention",
        "message.im"
      ]
    },
    "interactivity": {
      "is_enabled": false
    },
    "org_deploy_enabled": false,
    "socket_mode_enabled": false,
    "token_rotation_enabled": false
  }
}
```

If you want channel thread replies, private channel support, multi-person DMs, or file processing, add the matching events and scopes described above.

### Maximal Webhook Mode Manifest JSON

This example includes all scopes and event subscriptions needed to use all features currently supported by the IronClaw Slack channel:

- channel `@mentions`
- DMs
- threaded replies in public channels
- threaded replies in private channels
- multi-person DMs
- shared file download and processing

```json
{
  "_metadata": {
    "major_version": 2,
    "minor_version": 1
  },
  "display_information": {
    "name": "<APP_NAME>",
    "description": "<APP_DESCRIPTION>",
    "background_color": "<APP_BACKGROUND_COLOR>"
  },
  "features": {
    "app_home": {
      "home_tab_enabled": false,
      "messages_tab_enabled": true,
      "messages_tab_read_only_enabled": false
    },
    "bot_user": {
      "display_name": "<APP_NAME>",
      "always_online": false
    }
  },
  "oauth_config": {
    "scopes": {
      "bot": [
        "app_mentions:read",
        "chat:write",
        "im:history",
        "channels:history",
        "groups:history",
        "mpim:history",
        "files:read"
      ]
    }
  },
  "settings": {
    "event_subscriptions": {
      "request_url": "<EVENTS_REQUEST_URL>",
      "bot_events": [
        "app_mention",
        "message.im",
        "message.channels",
        "message.groups",
        "message.mpim"
      ]
    },
    "interactivity": {
      "is_enabled": false
    },
    "org_deploy_enabled": false,
    "socket_mode_enabled": false,
    "token_rotation_enabled": false
  }
}
```

### Minimal Socket Mode Manifest JSON

```json
{
  "_metadata": {
    "major_version": 2,
    "minor_version": 1
  },
  "display_information": {
    "name": "<APP_NAME>",
    "description": "<APP_DESCRIPTION>",
    "background_color": "<APP_BACKGROUND_COLOR>"
  },
  "features": {
    "app_home": {
      "home_tab_enabled": false,
      "messages_tab_enabled": true,
      "messages_tab_read_only_enabled": false
    },
    "bot_user": {
      "display_name": "<APP_NAME>",
      "always_online": false
    }
  },
  "oauth_config": {
    "scopes": {
      "bot": [
        "app_mentions:read",
        "chat:write",
        "im:history"
      ]
    }
  },
  "settings": {
    "event_subscriptions": {
      "bot_events": [
        "app_mention",
        "message.im"
      ]
    },
    "interactivity": {
      "is_enabled": false
    },
    "org_deploy_enabled": false,
    "socket_mode_enabled": true,
    "token_rotation_enabled": false
  }
}
```

If you want channel thread replies, private channel support, multi-person DMs, or file processing, add the matching events and scopes described above.

### Maximal Socket Mode Manifest JSON

This example includes all scopes and event subscriptions needed to use all features currently supported by the IronClaw Slack channel:

- channel `@mentions`
- DMs
- threaded replies in public channels
- threaded replies in private channels
- multi-person DMs
- shared file download and processing

```json
{
  "_metadata": {
    "major_version": 2,
    "minor_version": 1
  },
  "display_information": {
    "name": "<APP_NAME>",
    "description": "<APP_DESCRIPTION>",
    "background_color": "<APP_BACKGROUND_COLOR>"
  },
  "features": {
    "app_home": {
      "home_tab_enabled": false,
      "messages_tab_enabled": true,
      "messages_tab_read_only_enabled": false
    },
    "bot_user": {
      "display_name": "<APP_NAME>",
      "always_online": false
    }
  },
  "oauth_config": {
    "scopes": {
      "bot": [
        "app_mentions:read",
        "chat:write",
        "im:history",
        "channels:history",
        "groups:history",
        "mpim:history",
        "files:read"
      ]
    }
  },
  "settings": {
    "event_subscriptions": {
      "bot_events": [
        "app_mention",
        "message.im",
        "message.channels",
        "message.groups",
        "message.mpim"
      ]
    },
    "interactivity": {
      "is_enabled": false
    },
    "org_deploy_enabled": false,
    "socket_mode_enabled": true,
    "token_rotation_enabled": false
  }
}
```

### How to Apply the Manifest in Slack

1. Open your Slack app settings page
2. Go to **App Manifest**
3. Switch to JSON if needed
4. Paste one of the manifest examples above
5. Replace the placeholder tokens with your real values
6. Click **Save Changes**
7. Reinstall or update the app in your workspace if Slack prompts you to do so

## Slash Commands and Interactivity

The current IronClaw Slack channel does not require Slack Slash Commands or Slack interactivity.

- Do not add the `commands` scope
- Do not configure slash command request URLs
- Leave Slack interactivity disabled unless you are adding custom functionality outside the current channel

## Troubleshooting

### Slack channel does not start in Socket Mode

- Verify `slack_app_token` exists in the secrets store or `SLACK_APP_TOKEN` is exported
- Verify Socket Mode is enabled in the Slack app
- Verify the app-level token is an `xapp-...` token

### Slack webhook validation fails

- Verify `slack_signing_secret` matches the Slack app
- Verify Slack is posting to `/webhook/slack`
- Verify the public URL is HTTPS and reachable

### Bot can receive events but cannot reply

- Verify the bot token is valid
- Verify the app has `chat:write`
- Reinstall the Slack app to the workspace after changing scopes

### Pairing code not received in DM

- Verify `dm_policy` is still `pairing`
- Verify the bot can call `chat.postMessage`
- Check whether `owner_id` is set, which overrides DM pairing

## References

- Slack Socket Mode docs: <https://docs.slack.dev/apis/events-api/using-socket-mode>
- Slack app manifest reference: <https://docs.slack.dev/reference/app-manifest>
