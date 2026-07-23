# Slack Personal (User-Token) WASM Tool — IronClaw

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

The compiled component is embedded into the IronClaw binary via
`crates/ironclaw_first_party_extensions/assets/slack_user/` and registered in
`crates/ironclaw_composition/src/available_extensions.rs`.

## Configuring

**The shipped setup is OAuth.** In the packaged product this component ships as
the `slack_user` first-party extension ("Slack (personal)"), and users connect
it through the in-product Slack OAuth flow — the `slack_personal` product-auth
provider declared in
`crates/ironclaw_first_party_extensions/assets/slack_user/manifest.toml`, which
is the single source of truth for the shipped setup. Clicking **Connect** in the
WebUI redirects through Slack's consent screen and IronClaw obtains and stores
the user token automatically; **there is no token to create or paste.** The
OAuth flow requests the same **User Token Scopes** the tool needs: `search:read`,
`channels:history`, `groups:history`, `im:history`, `mpim:history`,
`channels:read`, `groups:read`, `im:read`, `mpim:read`, `users:read`, and
`chat:write` (the last only for posting).

### Manual token — dev / source builds only

When you build and run this component straight from `tools-src/` (outside the
packaged product) there is no OAuth flow, so you supply a Slack **User OAuth
Token** (`xoxp-`) directly under the `slack_user_token` credential handle:
create a private app at https://api.slack.com/apps, add the **User Token
Scopes** listed above under **OAuth & Permissions**, install it to your
workspace, and copy the User OAuth Token. This manual path is a
developer/source-tree convenience only — end users always get the OAuth flow
above.

Either way, the token is injected as a bearer credential at the host boundary;
the WASM component never sees it.

## Security

A user token acts as you and can read your private conversations. Treat it like
a password. IronClaw stores it encrypted and scans all tool output for
credential leakage before returning it.
