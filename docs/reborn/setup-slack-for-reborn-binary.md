# Set Up Slack for the Reborn Binary

This guide is for the standalone `ironclaw-reborn serve` Slack host-beta path,
not the legacy v1 Slack WASM channel.

Slack support has two gates:

1. The binary must be built with the `slack-v2-host-beta` Cargo feature.
2. Runtime config must set `[slack].enabled = true`.

Slack bot token and signing secret are configured in WebUI Slack setup and
stored in the Reborn secret store. Do not put OAuth client secrets or LLM keys
in `config.toml`.

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
OPENAI_API_KEY=sk-...
```

## Reborn Config

Edit `$IRONCLAW_REBORN_HOME/config.toml`. If the file does not exist yet, run
`ironclaw-reborn config init` or start the Docker image once to seed it.

Minimal Slack config:

```toml
[slack]
enabled = true
```

`enabled` is the only Slack boot setting. It mounts
`POST /webhooks/slack/events` and exposes Slack channel setup in WebUI.
Slack installation ids, team/app ids, the bot token, the signing secret,
and channel mappings are configured after startup from WebUI channel setup.

In WebUI, open Extensions, Slack, then Slack workspace setup. Save:

| Field | Purpose |
| --- | --- |
| Installation ID | Stable local id for this Slack app/workspace installation. Choose a durable operator-owned string. |
| Team ID | Slack workspace/team id, usually visible as `team_id` in Events API payloads. |
| App ID | Slack app id, visible as `api_app_id` in Events API payloads. |
| Bot user | Optional Reborn user id for Slack host-mediated egress. Defaults to the WebUI operator. |
| Shared subject | Optional Reborn user scope available for shared-channel routing. |
| Bot token | Slack bot token. Stored in the Reborn secret store; never returned by the API. |
| Signing secret | Slack signing secret. Stored in the Reborn secret store; never returned by the API. |

After Slack setup is configured, use the same Slack channel setup section to
map Slack channel ids to team agents.

Unrouted shared Slack channels fail closed instead of silently inheriting a
personal/default user scope.

## Slack App Configuration

Create or edit a Slack app at `api.slack.com/apps`.

Basic Information:

- Copy `Signing Secret` into WebUI Slack workspace setup.
- Copy `App ID` into WebUI Slack workspace setup.

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
- Copy `Bot User OAuth Token` into WebUI Slack workspace setup.

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

### [slack].enabled = true requires ... slack-v2-host-beta

Rebuild or rerun ironclaw-reborn with --features slack-v2-host-beta.

### Slack route never receives events

Confirm the Slack Request URL is exactly https://<public-host>/webhooks/slack/events, the public URL reaches the Reborn listener, and Socket Mode is disabled for this host-beta path.

### Slack URL verification fails

Confirm the WebUI Slack setup signing secret matches the app signing secret and that any proxy preserves the raw request body and Slack signature headers.

### Slack replies fail with missing_scope

Add or confirm chat:write, reinstall the Slack app, and update the bot token in WebUI Slack setup if Slack issued a new token.

### Pairing code DM fails

Confirm im:write and chat:write, reinstall the app, and verify the bot token in WebUI Slack setup starts with xoxb-.

### Channel mention does not reach Reborn

Confirm the app is invited to the channel, app_mention is subscribed, and the Team ID / App ID in WebUI Slack setup match the Slack app that emitted the event.

### Shared-channel turns are rejected

Configure Shared subject or use the WebUI Slack channel picker to allow the channel.

## Slack References

- Events API: https://docs.slack.dev/apis/events-api/
- Message events: https://docs.slack.dev/reference/events/message/
- `app_mention`: https://api.slack.com/events/app_mention
- Sending messages: https://docs.slack.dev/messaging/sending-and-scheduling-messages/
- Request signing: https://docs.slack.dev/authentication/verifying-requests-from-slack/
