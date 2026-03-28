---
name: slack
version: "1.0.0"
description: Slack Web API — post messages, manage channels, search, react, user info
activation:
  keywords:
    - "slack"
    - "post message"
    - "channel"
    - "slack message"
    - "reaction"
  exclude_keywords:
    - "discord"
    - "teams"
  patterns:
    - "(?i)send.*slack"
    - "(?i)slack.*(message|channel|thread|react)"
    - "(?i)post.*(to|in|on).*#"
  tags:
    - "messaging"
    - "communication"
    - "chat"
  max_context_tokens: 1800
metadata:
  openclaw:
    requires:
      env: [SLACK_BOT_TOKEN]
---

# Slack Web API

Use the `http` tool to call Slack. Credentials are automatically injected — **never construct Authorization headers manually**. When the URL host is `slack.com`, the system injects `Authorization: Bearer {SLACK_BOT_TOKEN}` transparently.

## Base URL

`https://slack.com/api`

All POST bodies use `Content-Type: application/json`.

## Actions

**Post message:**
```
http(method="POST", url="https://slack.com/api/chat.postMessage", body={"channel": "<channel_id>", "text": "Hello!"})
```

**Reply in thread:**
```
http(method="POST", url="https://slack.com/api/chat.postMessage", body={"channel": "<channel_id>", "text": "Reply", "thread_ts": "<message_ts>"})
```

**List channels:**
```
http(method="GET", url="https://slack.com/api/conversations.list?types=public_channel&limit=100")
```

**Channel history:**
```
http(method="GET", url="https://slack.com/api/conversations.history?channel=<channel_id>&limit=20")
```

**Search messages:**
```
http(method="GET", url="https://slack.com/api/search.messages?query=<search_text>&count=10")
```

**Add reaction:**
```
http(method="POST", url="https://slack.com/api/reactions.add", body={"channel": "<channel_id>", "timestamp": "<message_ts>", "name": "thumbsup"})
```

**Get user info:**
```
http(method="GET", url="https://slack.com/api/users.info?user=<user_id>")
```

**List users:**
```
http(method="GET", url="https://slack.com/api/users.list?limit=100")
```

**Update message:**
```
http(method="POST", url="https://slack.com/api/chat.update", body={"channel": "<channel_id>", "ts": "<message_ts>", "text": "Updated text"})
```

**Upload file:**
```
http(method="POST", url="https://slack.com/api/files.uploadV2", body={"channel_id": "<channel_id>", "content": "file content", "filename": "file.txt", "title": "My File"})
```

## Response Pattern

All responses: `{"ok": true, ...}` or `{"ok": false, "error": "<code>"}`.
Paginate with `cursor` param when `response_metadata.next_cursor` is non-empty.

## Common Mistakes

- Do NOT add Authorization headers — automatically injected.
- Channel IDs look like `C01234ABCDE`, not `#channel-name`. Use `conversations.list` to find IDs.
- Message timestamps (`ts`) are strings like `"1234567890.123456"` — not numbers.
- The `name` in `reactions.add` omits colons: use `thumbsup` not `:thumbsup:`.
