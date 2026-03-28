---
name: teams
version: "1.0.0"
description: Teams API — A collaboration and communication platform that offers chat, meetings
activation:
  keywords:
    - "teams"
    - "ai"
  patterns:
    - "(?i)teams"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
    - "communication"
  max_context_tokens: 1200
---

# Teams API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://graph.microsoft.com/v1.0`

## Actions

**List my teams:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/me/joinedTeams")
```

**List channels:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/teams/{team_id}/channels")
```

**Send message:**
```
http(method="POST", url="https://graph.microsoft.com/v1.0/teams/{team_id}/channels/{channel_id}/messages", body={"body": {"content": "Hello Teams!"}})
```

**List messages:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/teams/{team_id}/channels/{channel_id}/messages?$top=10")
```

**List chats:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/me/chats?$top=10")
```

## Notes

- Uses OAuth 2.0 via Microsoft Graph — credentials are auto-injected.
- Message body supports HTML: `{"contentType": "html", "content": "<b>Bold</b>"}`.
- Team and channel IDs are GUIDs.
- Pagination: `@odata.nextLink` URL in response.
