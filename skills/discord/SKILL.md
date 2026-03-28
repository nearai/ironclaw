---
name: discord
version: "1.0.0"
description: Discord API — send messages, manage channels, guilds, reactions
activation:
  keywords:
    - "discord"
    - "discord message"
    - "guild"
    - "discord channel"
  exclude_keywords:
    - "slack"
    - "teams"
  patterns:
    - "(?i)send.*discord"
    - "(?i)discord.*(message|channel|server|guild)"
  tags:
    - "messaging"
    - "communication"
    - "chat"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [DISCORD_BOT_TOKEN]
---

# Discord API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**. The system injects `Authorization: Bot {DISCORD_BOT_TOKEN}` for `discord.com` hosts.

## Base URL

`https://discord.com/api/v10`

## Actions

**Send message:**
```
http(method="POST", url="https://discord.com/api/v10/channels/<channel_id>/messages", body={"content": "Hello!"})
```

**Send embed:**
```
http(method="POST", url="https://discord.com/api/v10/channels/<channel_id>/messages", body={"embeds": [{"title": "Title", "description": "Body", "color": 5814783}]})
```

**Get channel messages:**
```
http(method="GET", url="https://discord.com/api/v10/channels/<channel_id>/messages?limit=50")
```

**List guild channels:**
```
http(method="GET", url="https://discord.com/api/v10/guilds/<guild_id>/channels")
```

**Get guild info:**
```
http(method="GET", url="https://discord.com/api/v10/guilds/<guild_id>")
```

**Add reaction:**
```
http(method="PUT", url="https://discord.com/api/v10/channels/<channel_id>/messages/<message_id>/reactions/<emoji>/@me")
```

**Create channel:**
```
http(method="POST", url="https://discord.com/api/v10/guilds/<guild_id>/channels", body={"name": "new-channel", "type": 0})
```

**Get user:**
```
http(method="GET", url="https://discord.com/api/v10/users/@me")
```

## Notes

- Channel types: 0 = text, 2 = voice, 4 = category, 5 = announcement.
- Embeds `color` is decimal (not hex). Convert: `0x58ACFF` → `5814783`.
- Emoji in reactions: URL-encode unicode emoji (`%F0%9F%91%8D` for thumbsup) or use `name:id` for custom.
- Rate limits: 50 requests/second. Check `X-RateLimit-Remaining` header.
- Snowflake IDs are strings of digits (e.g., `"1234567890123456789"`).
