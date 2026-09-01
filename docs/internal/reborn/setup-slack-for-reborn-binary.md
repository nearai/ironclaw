# Set Up Slack for the Reborn Binary

This guide is for the standalone `ironclaw serve` Slack host path,
not the legacy v1 Slack WASM channel.

Slack support ships in the binary, and there is no configuration key or
environment variable that turns it on. The Slack webhook route is always
mounted; Slack goes live once the Slack extension is installed and its setup is
completed in the WebUI at `/extensions`.

Slack bot token and signing secret are configured in WebUI Slack setup and
stored in the Reborn secret store. Do not put OAuth client secrets or LLM keys
in `config.toml`.

## Build or Run With Slack

For local source runs:

```bash
cargo run -q \
  -p ironclaw \
  --bin ironclaw \
  -- serve
```

For a local source build:

```bash
cargo build \
  -p ironclaw \
  --bin ironclaw
```

Neither command needs a Slack-specific build flag or feature: the route is
compiled in and mounted unconditionally.

## Public Endpoint

Slack Events API must reach the Reborn listener over a public HTTPS URL:

```text
https://<public-host>/webhooks/extensions/slack/events
```

Slack user OAuth must also redirect back to the Reborn product-auth
callback:

```text
https://<public-host>/api/reborn/product-auth/oauth/slack/callback
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

# WebUI env-bearer auth; required by `ironclaw serve`.
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

The Reborn config file lives at `$IRONCLAW_REBORN_HOME/config.toml`; run
`ironclaw config init` or start the Docker image once to seed it if it does not
exist yet.

It carries no Slack settings. There is no `[slack]` section to add and no Slack
key to set: `slack.enabled` is retired, so `ironclaw config set slack.enabled`
answers with migration guidance instead of writing anything.

`POST /webhooks/extensions/slack/events` is mounted unconditionally, and it
answers `503 temporarily_unavailable` until the Slack extension's ingress
signing secret is registered. Installing the Slack extension from `/extensions`
and completing its setup is what registers that secret, exposes the
manifest-declared Slack deployment fields in Admin Configuration, and makes a
personal Slack connection available through the Slack extension's user OAuth
flow.

Slack installation ids, team/app ids, the bot token, the signing secret, and
OAuth client credentials are configured after startup from Admin
Configuration. These deployment values are never shown in a user's extension
setup flow.

> **"Admin Configuration" and the "Slack deployment configuration card" are the
> same place.** Concretely: web interface -> **Admin** -> **Configuration** ->
> **Slack deployment configuration**. (The **Extensions** page handles only the
> personal half — installing the extension and connecting your own account.)
> Every "Admin Configuration" reference below means that card.

As an operator, open Admin, Configuration, then Slack deployment configuration.
Save:

| Field | Purpose |
| --- | --- |
| Installation ID | Stable local id for this Slack app/workspace installation. Choose a durable operator-owned string. |
| Team ID | Slack workspace/team id, usually visible as `team_id` in Events API payloads. |
| App ID | Slack app id, visible as `api_app_id` in Events API payloads. |
| Bot user ID | Slack member id for the app's bot user (for example, the `U…` id returned at installation). |
| Bot token | Slack bot token. Stored in the Reborn secret store; never returned by the API. |
| Signing secret | Slack signing secret. Stored in the Reborn secret store; never returned by the API. |
| OAuth client ID | Client id for the Slack app's user OAuth flow. |
| OAuth client secret | Client secret for the Slack app's user OAuth flow. Stored in the Reborn secret store. |

There is no shared-channel configuration at all. Inviting the bot into a
Slack channel is what enables that channel: Slack only delivers a channel's
events to the app because the bot is a member, so the bot's presence — an
event arriving through the verified webhook — is itself the admission. To
stop serving a channel, remove the bot from it.

In a shared channel the bot answers each participant as themselves — there
is no shared subject user or per-channel subject route to assign, and each
shared-channel message runs as the Slack user who sent it. Users separately
install Slack from Extensions and complete their own OAuth flow to pair. A
participant who mentions the bot before pairing gets a short pairing notice
threaded on their own message (never a broadcast into the room) pointing
them at the Slack connect flow in Extensions; their personal membership and
credential state does not mutate the operator configuration.

### Migrating an existing config.toml

An existing file that still carries a `[slack]` or `[telegram]` section keeps
parsing, so an older deployment does not break on upgrade. A leftover Slack
*setup* field (`installation_id`, `team_id`, `api_app_id`, `slack_user_id`,
`user_id`, `shared_subject_user_id`, `channel_routes`, `signing_secret_env`,
`bot_token_env`) fails `serve` closed with a migration pointer rather than
being silently ignored. A section left with only inert keys still boots, and
logs a deprecation notice. Either way the fix is the same: delete the section,
because nothing reads it.

## Slack App Configuration

Create or edit a Slack app at `api.slack.com/apps`.

### Native Agent (required)

Replies reach Slack through its native Agent surface, not as a finished
`chat.postMessage`: each run is one agent session in the conversation's thread
(`agents.sessions.setStatus` — `processing` while the run works, `suspended`
while it waits on an approval or sign-in, `active` when it ends) and one
streaming message (`chat.startStream` → `chat.appendStream` per delta / task
card → `chat.stopStream`). The app therefore needs:

- The **Agents** feature enabled (app settings sidebar → Agents; in a manifest,
  `features.agent_view` with an `agent_description` of at most 300 characters
  and `suggested_prompts` entries of `{title, message}`). Slack adds the
  `assistant:write` bot scope when the feature is enabled; declare it in the
  manifest as well so an import carries it.
- The Messages tab enabled and writable (`features.app_home.messages_tab_enabled
  = true`, `messages_tab_read_only_enabled = false`).
- Bot scopes `assistant:write` and `chat:write` (every session and streaming
  method requires `chat:write`).
- Bot event subscriptions `app_home_opened`, `app_context_changed`,
  `message.im`, `agent_session_stopped` (this is what makes Slack show the
  **Stop** button; pressing it is normalized into the channel's `stop`
  command), and `agent_session_title_changed`. Do not subscribe to the
  legacy `assistant_thread_*` events — Slack's Agent View validator rejects
  a manifest that carries them (the parser still tolerates them arriving
  from older installs).

Two things to know before switching an existing app:

- **`agent_view` is irreversible.** Slack: "Once you change your app's manifest
  from `assistant_view` to `agent_view`, you can't revert to the Assistant
  messaging experience."
- **There is no conventional-message fallback.** A workspace whose app lacks
  the Agents feature answers `feature_disabled` (or `not_agent_app`) to
  `agents.sessions.setStatus`; the reply fails clearly, recorded as a failed
  delivery whose reason names `features.agent_view`, until an operator enables
  the feature and reinstalls the app. A bot token without `chat:write` fails
  the same way with `missing_scope`.

The canonical, directly importable Agent-enabled app manifest is
`crates/extensions/packages/slack/app_manifest.json`; the public page
`docs/channels/slack.mdx` embeds a test-pinned identical copy, and
`tests/agent_app_manifest_lockstep.rs` in the Slack package pins both against
the extension manifest's egress allowlist and the calls the code makes.

Basic Information:

- Copy `Signing Secret` into Admin Configuration for Slack.
- Copy `App ID` into Admin Configuration for Slack.

OAuth & Permissions:

- Add the redirect URL:

```text
https://<public-host>/api/reborn/product-auth/oauth/slack/callback
```

- Add bot token scopes:
  - `assistant:write` (added by the Agents feature) and `chat:write` for the
    agent session and the streamed reply, plus temporary notices.
  - `im:write` for opening DMs after a user has connected with OAuth.
  - `app_mentions:read` for channel mentions.
  - `im:history` for direct-message events.
  - `channels:history` if the bot should receive public-channel message events
    beyond `app_mention`.
  - `groups:history` if the bot should receive private-channel message events.
  - `mpim:history` if the bot should receive group-DM message events.
  - `files:read` if Slack file attachments should be downloaded and processed.
  - `files:write` if explicitly attached workspace files should be delivered
    back to Slack.
  - `commands` to register the `/ironclaw` slash command (see Slash Command
    below).
- Add user token scopes:
  - `users:read` for binding the authenticated Slack user to the Reborn user.
- Install or reinstall the app to the workspace after changing scopes.
- Copy `Bot User OAuth Token` into WebUI Slack workspace setup.

Event Subscriptions:

- Enable Events.
- Set Request URL to:

```text
https://<public-host>/webhooks/extensions/slack/events
```

- Subscribe to bot events:
  - `app_mention`
  - `message.im`
  - The Agent family (see Native Agent above): `app_home_opened`,
    `app_context_changed`, `agent_session_stopped`,
    `agent_session_title_changed` — and NOT the rejected legacy
    `assistant_thread_*` pair
  - Optional: `message.channels`
  - Optional: `message.groups`
  - Optional: `message.mpim`

Slash Command:

- Slack allows a slash command's Request URL to be any URL, so it can point
  at the exact same signed endpoint as Event Subscriptions above — there is
  no second route to register, and the events ingress already accepts the
  slash command's form payload, answers its `ssl_check` verification probe,
  and rejects non-DM invocations by design (see Troubleshooting).
- Create a command:
  - Command: `/ironclaw`
  - Request URL:

```text
https://<public-host>/webhooks/extensions/slack/events
```

  - Short Description: `Run IronClaw commands`
  - Usage Hint: `status | model <name> | help`
- Save. Registering a slash command adds the `commands` bot token scope;
  install or reinstall the app after saving.

App Home:

- Enable messages so users can DM the app (the Messages tab is where an Agent
  DM lives; it must not be read-only).

Install:

- Install or reinstall the app after adding scopes or event subscriptions.
- Invite the app to any Slack channel where channel mentions should work.

Minimal app manifest sketch:

```yaml
display_information:
  name: IronClaw Reborn
features:
  agent_view:
    agent_description: "IronClaw is your autonomous assistant: it researches, drafts, runs tools, and acts across your connected apps, streaming each step into the thread."
    suggested_prompts:
      - title: Summarize a thread
        message: Summarize the discussion in the thread I share next and list the open questions.
      - title: Check my integrations
        message: Which integrations are connected for me right now, and what can you do with them?
  app_home:
    home_tab_enabled: false
    messages_tab_enabled: true
    messages_tab_read_only_enabled: false
  bot_user:
    display_name: IronClaw Reborn
    always_online: false
  slash_commands:
    - command: /ironclaw
      description: Run IronClaw commands
      usage_hint: "status | model <name> | help"
      url: https://<public-host>/webhooks/extensions/slack/events
      should_escape: false
oauth_config:
  redirect_urls:
    - https://<public-host>/api/reborn/product-auth/oauth/slack/callback
  scopes:
    bot:
      - assistant:write
      - chat:write
      - im:write
      - app_mentions:read
      - im:history
      - channels:history
      - groups:history
      - mpim:history
      - files:read
      - files:write
      - commands
    user:
      - users:read
settings:
  event_subscriptions:
    request_url: https://<public-host>/webhooks/extensions/slack/events
    bot_events:
      - app_mention
      - message.im
      - message.channels
      - message.groups
      - message.mpim
      - app_home_opened
      - app_context_changed
      - agent_session_stopped
      - agent_session_title_changed
  org_deploy_enabled: false
  socket_mode_enabled: false
  token_rotation_enabled: false
```

Use least privilege for production. For example, omit `groups:history` if the
bot does not need private-channel events. Omit `files:read` if inbound
attachment processing is not needed, and omit `files:write` if outbound file
delivery is not needed. Outbound files use Slack's supported external upload
flow; the retired `files.upload` method is never called.

## Start and Verify

Start the service:

```bash
cargo run -q \
  -p ironclaw \
  --bin ironclaw \
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
- `POST /webhooks/extensions/slack/events` returns the Slack URL-verification challenge
  during setup.
- After the operator saves deployment configuration and the user installs and
  connects the Slack extension, the OAuth callback
  binds that Slack user to the authenticated Reborn user.
- A DM to the app routes through the OAuth-connected Reborn user.
- `/ironclaw status` sent in the bot DM replies with the rendered status result.
- A channel `@app` mention replies in the same channel thread, as a streaming
  agent message: the session shows *processing* while the run works, task
  cards appear per tool call, and the text grows until the run ends.
- Slack's **Stop** button cancels the run and the session leaves *processing*.
- Bot-originated and subtyped Slack messages are ignored.

## Troubleshooting

### Slack events are rejected with 503 or 401

There is no config or env enablement gate to check; the route is always mounted.

A 503 `temporarily_unavailable` means the Slack extension's ingress signing secret is not
registered yet. Register it in Admin Configuration for Slack (the Slack card — see the
note under "Reborn Config" for the exact UI path; landing on the wrong Extensions tab is
the usual reason this step is missed).

A 401 means a signing secret is registered but does not match the app. Compare the value
there against **Basic Information -> Signing Secret** in the Slack app.

### Slack route never receives events

Confirm the Slack Request URL is exactly https://<public-host>/webhooks/extensions/slack/events, the public URL reaches the Reborn listener, and Socket Mode is disabled for this host path.

### Slash command shows dispatch_failed

The `/ironclaw` command is not registered on this Slack app, or its Request URL does not exactly match https://<public-host>/webhooks/extensions/slack/events. Add or fix the command under Slack App Configuration, Slash Command, then reinstall the app.

### Slash command reply is slow or times out

The ingress only answers 200 once the request is durably admitted, so a healthy deployment replies well inside Slack's short response budget, but a degraded backend can occasionally push that past Slack's client-side timeout even though the bot still posts the reply moments later in the DM. This is a rare, pre-existing exposure and is not unique to slash commands.

### Slack URL verification fails

Confirm the Admin Configuration Slack signing secret matches the app signing secret and that any proxy preserves the raw request body and Slack signature headers.

### Slack replies fail with missing_scope

Add or confirm `chat:write` for text (it also covers the session and streaming
methods), `assistant:write` for the Agent feature, `files:read` for inbound
attachments, and `files:write` for outbound attachments. Reinstall the Slack
app, and update the bot token in Admin Configuration if Slack issued a new
token.

### Slack replies fail with feature_disabled or not_agent_app

The app is installed without Slack's Agents feature. There is no fallback to
plain messages: enable **Agents** in the app settings (`features.agent_view`
in the manifest), confirm `assistant:write` and `chat:write` are among the bot
scopes, reinstall the app, and update the bot token in Admin Configuration if
Slack issued a new one. The failed delivery's reason names the missing
capability.

### The Stop button never appears in Slack

Slack shows it only while a session is `processing` and only when the app
subscribes to `agent_session_stopped`. Add the subscription and reinstall.

### Slack OAuth callback fails

Confirm the Slack redirect URL is exactly https://<public-host>/api/reborn/product-auth/oauth/slack/callback, the user scope includes users:read, the app was reinstalled after changing OAuth settings, and the Admin Configuration Slack client id/client secret match the Slack app.

### Channel mention does not reach Reborn

Confirm the app is invited to the channel, app_mention is subscribed, and the Team ID / App ID in Admin Configuration match the Slack app that emitted the event.

### Shared channel gets no answer, or a pairing notice

There is no channel allowlist to configure: any channel the bot has been
invited to is served, because the channel's events reaching the verified
webhook is itself the admission. If a mention produces nothing at all, the
event is not arriving — see "Channel mention does not reach Reborn" above.
If the bot instead answers a specific user with a pairing notice threaded on
their message, that user has not connected Slack yet — they complete the
Slack OAuth connect from Extensions, since every shared-channel participant
runs as themselves.

### Slash command outside the bot DM is denied

This is by design: `/ironclaw` requires a direct conversation, so an invocation from any other channel is rejected with a denial notice posted back into that same channel, and no command executes. Message the app directly instead.

## Slack References

- Events API: https://docs.slack.dev/apis/events-api/
- Message events: https://docs.slack.dev/reference/events/message/
- `app_mention`: https://api.slack.com/events/app_mention
- Sending messages: https://docs.slack.dev/messaging/sending-and-scheduling-messages/
- Slash commands: https://docs.slack.dev/interactivity/implementing-slash-commands/
- Request signing: https://docs.slack.dev/authentication/verifying-requests-from-slack/
- Developing agents: https://docs.slack.dev/ai/developing-agents
- Agent sessions: https://docs.slack.dev/ai/agent-sessions
- Migrating to agent messaging (irreversibility): https://docs.slack.dev/ai/migrating-to-agent-messaging
- App manifest (`features.agent_view`): https://docs.slack.dev/reference/app-manifest
- Streaming: https://docs.slack.dev/reference/methods/chat.startStream,
  https://docs.slack.dev/reference/methods/chat.appendStream,
  https://docs.slack.dev/reference/methods/chat.stopStream
- Sessions: https://docs.slack.dev/reference/methods/agents.sessions.setStatus,
  https://docs.slack.dev/reference/methods/agents.sessions.rename
- Events: https://docs.slack.dev/reference/events/agent_session_stopped,
  https://docs.slack.dev/reference/events/agent_session_title_changed
