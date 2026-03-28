---
name: front
version: "1.0.0"
description: Front API — Front is a shared inbox and communication platform that brings email, apps
activation:
  keywords:
    - "front"
    - "ticketing"
  patterns:
    - "(?i)front"
  tags:
    - "tools"
    - "Customer Support"
  max_context_tokens: 1200
---

# Front API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api2.frontapp.com`

## Actions

**List conversations:**
```
http(method="GET", url="https://api2.frontapp.com/conversations?limit=10")
```

**Get conversation:**
```
http(method="GET", url="https://api2.frontapp.com/conversations/{conversation_id}")
```

**Send message:**
```
http(method="POST", url="https://api2.frontapp.com/channels/{channel_id}/messages", body={"to": ["recipient@example.com"],"body": "Hello!","subject": "Re: Your request"})
```

**List inboxes:**
```
http(method="GET", url="https://api2.frontapp.com/inboxes")
```

**List teammates:**
```
http(method="GET", url="https://api2.frontapp.com/teammates")
```

## Notes

- Conversations aggregate messages across channels.
- Channel types: email, SMS, chat, social.
- Conversation IDs start with `cnv_`.
- Pagination: `_links.next` URL in response.
