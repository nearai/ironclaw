# Set Up Slack for the Reborn Binary

This guide is for the standalone `ironclaw-reborn serve` Slack host-beta path,
not the legacy v1 Slack WASM channel.

Slack support has two gates:

1. The binary must be built with the `slack-v2-host-beta` Cargo feature.
2. Runtime config must set `[slack].enabled = true`.

Slack secrets are environment variables only. Do not put bot tokens, signing
secrets, OAuth client secrets, or LLM keys in `config.toml`.

## Build or Run With Slack

For local source runs:

```bash
cargo run -q \
  -p ironclaw_reborn_cli \
  --features slack-v2-host-beta \
  --bin ironclaw-reborn \
  -- serve
```

For a local source build:

```bash
cargo build \
  -p ironclaw_reborn_cli \
  --features slack-v2-host-beta \
  --bin ironclaw-reborn
```

`slack-v2-host-beta` includes `webui-v2-beta`, so do not pass both unless you
prefer to be explicit:

```bash
--features webui-v2-beta,slack-v2-host-beta
```

`Dockerfile.reborn` already builds with `webui-v2-beta,slack-v2-host-beta`.
Slack is still disabled unless the mounted or seeded Reborn config enables it.

## Public Endpoint

Slack Events API must reach the Reborn listener over a public HTTPS URL:

```text
https://<public-host>/webhooks/slack/events
```

For local development, expose the local listener through a tunnel and use the
tunnel URL in Slack. The listener defaults to `127.0.0.1:3000`; use
`serve --host 0.0.0.0 --port 3000` only when intentionally exposing it behind a
proxy, tunnel, or container port.

Do not use `IRONCLAW_REBORN_PROFILE=local-dev-yolo` for a public listener.
That profile grants trusted host access and `serve` refuses non-loopback binds.

## Environment Variables

Minimum local env shape:

```bash
export IRONCLAW_REBORN_HOME="$PWD/.reborn-home"
export IRONCLAW_REBORN_PROFILE="local-dev"

# WebUI env-bearer auth; required by `ironclaw-reborn serve`.
export IRONCLAW_REBORN_WEBUI_TOKEN="$(openssl rand -hex 32)"
export IRONCLAW_REBORN_WEBUI_USER_ID="reborn-cli"

# LLM provider selected by [llm.default] in config.toml.
export OPENAI_API_KEY="sk-..."

# Slack secrets. The config stores only these variable names.
export IRONCLAW_REBORN_SLACK_SIGNING_SECRET="<slack-signing-secret>"
export IRONCLAW_REBORN_SLACK_BOT_TOKEN="xoxb-..."
```

Optional public WebUI login or OAuth flows may also need
`IRONCLAW_REBORN_WEBUI_BASE_URL` and provider-specific SSO variables. The Slack
Events API route itself does not require WebUI SSO.

Docker/Railway env shape:

```bash
IRONCLAW_REBORN_SERVE_HOST=0.0.0.0
PORT=3000
IRONCLAW_REBORN_HOME=/data/ironclaw-reborn
IRONCLAW_REBORN_PROFILE=local-dev
IRONCLAW_REBORN_WEBUI_TOKEN=<random-hex-32-bytes-or-longer>
IRONCLAW_REBORN_WEBUI_USER_ID=reborn-cli
IRONCLAW_REBORN_SLACK_SIGNING_SECRET=<slack-signing-secret>
IRONCLAW_REBORN_SLACK_BOT_TOKEN=xoxb-...
OPENAI_API_KEY=sk-...
```

## Reborn Config

Edit `$IRONCLAW_REBORN_HOME/config.toml`. If the file does not exist yet, run
`ironclaw-reborn config init` or start the Docker image once to seed it.

Minimal Slack config:

```toml
[slack]
enabled = true
installation_id = "install-alpha"
team_id = "T1234567890"
api_app_id = "A1234567890"
signing_secret_env = "IRONCLAW_REBORN_SLACK_SIGNING_SECRET"
bot_token_env = "IRONCLAW_REBORN_SLACK_BOT_TOKEN"
```

Field notes:

| Field | Required | Purpose |
| --- | --- | --- |
| `enabled` | Yes | Mounts `POST /webhooks/slack/events`. Env vars alone do not enable Slack. |
| `installation_id` | Yes | Stable local id for this Slack app/workspace installation. Choose a durable operator-owned string. |
| `team_id` | Yes | Slack workspace/team id, usually visible as `team_id` in Events API payloads. |
| `api_app_id` | Yes | Slack app id, visible as `api_app_id` in Events API payloads. Required for personal-binding pairing. |
| `signing_secret_env` | No | Env var containing the Slack signing secret. Defaults to `IRONCLAW_REBORN_SLACK_SIGNING_SECRET`. |
| `bot_token_env` | No | Env var containing the Slack bot token. Defaults to `IRONCLAW_REBORN_SLACK_BOT_TOKEN`. |
| `slack_user_id` | No | Legacy static Slack user mapping. Omit for the pairing-code flow. |
| `user_id` | No | Reborn user id for the legacy mapped user and host-mediated Slack egress. Defaults to `IRONCLAW_REBORN_WEBUI_USER_ID`. |
| `shared_subject_user_id` | No | Reborn user scope for shared Slack channel turns. Omit when using explicit channel routes. |
| `[[slack.channel_routes]]` | No | Static Slack channel id to Reborn user scope mappings for app mentions and thread replies. |

Optional static routing examples:

```toml
[slack]
enabled = true
installation_id = "install-alpha"
team_id = "T1234567890"
api_app_id = "A1234567890"
signing_secret_env = "IRONCLAW_REBORN_SLACK_SIGNING_SECRET"
bot_token_env = "IRONCLAW_REBORN_SLACK_BOT_TOKEN"

# Optional: one shared Reborn subject for unrouted shared-channel turns.
shared_subject_user_id = "slack-team-agent"

[[slack.channel_routes]]
channel_id = "CENG123456"
subject_user_id = "eng-team-agent"

[[slack.channel_routes]]
channel_id = "CSUPPORT123"
subject_user_id = "support-team-agent"
```

In host-beta admin-managed mode, dynamic channel routes saved through WebUI
take precedence over static `channel_routes`. Unrouted shared Slack channels
fail closed instead of silently inheriting a personal/default user scope.

## Slack App Configuration

Create or edit a Slack app at `api.slack.com/apps`.

Basic Information:

- Copy `Signing Secret` into `IRONCLAW_REBORN_SLACK_SIGNING_SECRET`.
- Copy `App ID` into `[slack].api_app_id`.

OAuth & Permissions:

- Add bot token scopes:
  - `chat:write` for final replies and temporary working messages.
  - `im:write` for opening DMs used by the pairing-code flow.
  - `app_mentions:read` for channel mentions.
  - `im:history` for direct-message events.
  - `channels:history` if the bot should receive public-channel message events
    beyond `app_mention`.
  - `groups:history` if the bot should receive private-channel message events.
  - `mpim:history` if the bot should receive group-DM message events.
  - `files:read` if Slack file attachments should be downloaded and processed.
- Install or reinstall the app to the workspace after changing scopes.
- Copy `Bot User OAuth Token` into `IRONCLAW_REBORN_SLACK_BOT_TOKEN`.

Event Subscriptions:

- Enable Events.
- Set Request URL to:

```text
https://<public-host>/webhooks/slack/events
```

- Subscribe to bot events:
  - `app_mention`
  - `message.im`
  - Optional: `message.channels`
  - Optional: `message.groups`
  - Optional: `message.mpim`

App Home:

- Enable messages so users can DM the app.

Install:

- Install or reinstall the app after adding scopes or event subscriptions.
- Invite the app to any Slack channel where channel mentions should work.

Minimal app manifest sketch:

```yaml
display_information:
  name: IronClaw Reborn
features:
  bot_user:
    display_name: IronClaw Reborn
    always_online: false
oauth_config:
  scopes:
    bot:
      - chat:write
      - im:write
      - app_mentions:read
      - im:history
      - channels:history
      - groups:history
      - mpim:history
      - files:read
settings:
  event_subscriptions:
    request_url: https://<public-host>/webhooks/slack/events
    bot_events:
      - app_mention
      - message.im
      - message.channels
      - message.groups
      - message.mpim
  org_deploy_enabled: false
  socket_mode_enabled: false
  token_rotation_enabled: false
```

Use least privilege for production. For example, omit `groups:history` if the
bot does not need private-channel events, and omit `files:read` if attachment
processing is not needed.

## Start and Verify

Start the service:

```bash
cargo run -q \
  -p ironclaw_reborn_cli \
  --features slack-v2-host-beta \
  --bin ironclaw-reborn \
  -- serve --host 127.0.0.1 --port 3000
```

With Docker:

```bash
docker run --rm \
  --env-file .env.reborn \
  -p 127.0.0.1:3000:3000 \
  ironclaw-reborn:local
```

Verification checklist:

- Slack Event Subscriptions shows the Request URL as verified.
- `POST /webhooks/slack/events` returns the Slack URL-verification challenge
  during setup.
- A DM to the app either produces a pairing code or routes through the paired
  Reborn user.
- A channel `@app` mention replies in the same channel thread.
- Bot-originated and subtyped Slack messages are ignored.

## Troubleshooting

### [slack].enabled = true requires ... slack-v2-host-beta\n\nRebuild or rerun ironclaw-reborn with --features slack-v2-host-beta.\n\n### Slack route never receives events\n\nConfirm the Slack Request URL is exactly https://<public-host>/webhooks/slack/events, the public URL reaches the Reborn listener, and Socket Mode is disabled for this host-beta path.\n\n### Slack URL verification fails\n\nConfirm IRONCLAW_REBORN_SLACK_SIGNING_SECRET matches the app signing secret and that any proxy preserves the raw request body and Slack signature headers.\n\n### Slack replies fail with missing_scope\n\nAdd or confirm chat:write, reinstall the Slack app, and update IRONCLAW_REBORN_SLACK_BOT_TOKEN if Slack issued a new token.\n\n### Pairing code DM fails\n\nConfirm im:write and chat:write, reinstall the app, and verify the bot token starts with xoxb-.\n\n### Channel mention does not reach Reborn\n\nConfirm the app is invited to the channel, app_mention is subscribed, and [slack].team_id / [slack].api_app_id match the Slack app that emitted the event.\n\n### Shared-channel turns are rejected\n\nAdd a static [[slack.channel_routes]] entry, configure shared_subject_user_id, or use the WebUI Slack channel picker to allow the channel.

## Slack References

- Events API: https://docs.slack.dev/apis/events-api/
- Message events: https://docs.slack.dev/reference/events/message/
- `app_mention`: https://api.slack.com/events/app_mention
- Sending messages: https://docs.slack.dev/messaging/sending-and-scheduling-messages/
- Request signing: https://docs.slack.dev/authentication/verifying-requests-from-slack/
