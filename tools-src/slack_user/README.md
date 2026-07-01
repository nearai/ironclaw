# Slack Personal (User-Token) WASM Tool — IronClaw Reborn

A standalone WASM component that lets IronClaw act **as you** in Slack, using a
personal **user token** (`xoxp-`) rather than a bot token. This enables reading
your full message history, your DMs, and searching everything you can see —
things a bot token fundamentally cannot do.

This is the user-identity counterpart to the bot-identity `slack` tool. The two
are deliberately separate tools with **separate secrets** (`slack_user_token`
vs `slack_bot_token`) so the personal token never collides with the bot token
used by the Slack channel.

## Features

- **search_messages**: Search across all messages you can see (DMs, group DMs,
  and channels you belong to). Supports Slack search operators like
  `from:@me`, `in:#channel`, `after:2024-01-01`.
- **list_conversations**: List channels, private channels, DMs (`im`), and
  group DMs (`mpim`) you belong to — use this to discover DM conversation IDs.
- **get_conversation_history**: Read history of any channel or DM by ID, with
  `latest`/`oldest` pagination cursors.
- **get_user_info**: Look up a user's name, real name, and email.
- **send_message**: Post a message as you (requires `chat:write`).

## Building

```bash
cd tools-src/slack_user
cargo component build --release --target wasm32-wasip2
```

The compiled component is embedded into the Reborn binary via
`crates/ironclaw_first_party_extensions/assets/slack_user/` and registered in
`crates/ironclaw_reborn_composition/src/available_extensions.rs`.

## Configuring

Provide a Slack **User OAuth Token** (`xoxp-`) under the `slack_user_token`
credential handle. Create a private app at https://api.slack.com/apps and, under
**OAuth & Permissions**, add **User Token Scopes** (not Bot Token Scopes):
`search:read`, `channels:history`, `groups:history`, `im:history`,
`mpim:history`, `channels:read`, `groups:read`, `im:read`, `mpim:read`,
`users:read`, and `chat:write` (for posting). Install to your workspace and copy
the User OAuth Token.

The token is injected as a bearer credential at the host boundary; the WASM
component never sees it.

## Security

A user token acts as you and can read your private conversations. Treat it like
a password. IronClaw stores it encrypted and scans all tool output for
credential leakage before returning it.
