---
name: slack
version: "1.0.0"
description: Send messages, list channels, read history, add reactions, and get user info in Slack via HTTP tool with automatic credential injection
activation:
  keywords:
    - "slack"
    - "channel"
    - "message"
  exclude_keywords:
    - "slack channel" 
  patterns:
    - "(?i)(send|post|write).*(slack|channel)"
    - "(?i)(list|show).*(slack|channels)"
    - "(?i)slack"
  tags:
    - "communication"
    - "slack"
  max_context_tokens: 2000
credentials:
  - name: slack_bot_token
    provider: slack
    location:
      type: header
      name: Authorization
      prefix: "Bearer "
    hosts:
      - "slack.com"
    oauth:
      authorization_url: "https://slack.com/oauth/v2/authorize"
      token_url: "https://slack.com/api/oauth.v2.access"
      client_id_env: SLACK_OAUTH_CLIENT_ID
      client_secret_env: SLACK_OAUTH_CLIENT_SECRET
      scopes:
        - "chat:write"
        - "channels:read"
        - "channels:history"
        - "groups:read"
        - "groups:history"
        - "reactions:write"
        - "users:read"
    setup_instructions: "Create a Slack App at https://api.slack.com/apps, add Bot Token Scopes (chat:write, channels:read, channels:history, groups:read, groups:history, reactions:write, users:read), install to workspace, copy the Bot User OAuth Token (starts with xoxb-)"
http:
  allowed_hosts:
    - "slack.com"
---

# Slack Skill

You have access to the Slack API via the `http` tool. Credentials are automatically injected — **never construct `Authorization` headers manually**. When the URL host is `slack.com`, the system injects `Authorization: Bearer {slack_bot_token}` transparently.

## API Patterns

All endpoints use `https://slack.com/api/` as the base URL. All responses include `"ok": true` on success.

### Send a message

```
http(method="POST", url="https://slack.com/api/chat.postMessage", body={"channel": "C01234567", "text": "Hello from IronClaw"})
```

- `channel`: Channel ID or encoded name (e.g. `#general`)
- `text`: Message text (supports mrkdwn)
- `thread_ts`: Optional — reply in thread

### List channels

```
http(method="GET", url="https://slack.com/api/conversations.list?limit=100")
```

Returns `channels` array with `id`, `name`, `is_private`, `is_member`, `topic`, `purpose`.

### Read channel history

```
http(method="GET", url="https://slack.com/api/conversations.history?channel=C01234567&limit=20")
```

Returns `messages` array with `ts`, `text`, `user`.

### Add a reaction

```
http(method="POST", url="https://slack.com/api/reactions.add", body={"channel": "C01234567", "timestamp": "1234567890.123456", "name": "thumbsup"})
```

- `name`: Emoji name without colons (e.g. `thumbsup`, not `:thumbsup:`)

### Get user info

```
http(method="GET", url="https://slack.com/api/users.info?user=U01234567")
```

Returns `user` object with `id`, `name`, `real_name`, `display_name`, `email`.

## Common Mistakes

- Do NOT add an `Authorization` header — it is injected automatically.
- Channel IDs start with `C` (public), `G` (private/group), `D` (DM). Use the ID, not the display name, for API calls.
- The bot token starts with `xoxb-`. Never expose it.
- For threaded replies, always include `thread_ts` from the parent message.
