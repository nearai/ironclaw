---
name: nylas
version: "1.0.0"
description: Nylas API — Nylas provides a unified communications API platform enabling developers to embe
activation:
  keywords:
    - "nylas"
    - "integration"
  patterns:
    - "(?i)nylas"
  tags:
    - "tools"
    - "integration"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [NYLAS_BASE_URL, NYLAS_API_KEY]
---

# Nylas API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.us.nylas.com/v3`

## Actions

**List messages:**
```
http(method="GET", url="https://api.us.nylas.com/v3/grants/{grant_id}/messages?limit=10")
```

**Get message:**
```
http(method="GET", url="https://api.us.nylas.com/v3/grants/{grant_id}/messages/{message_id}")
```

**Send message:**
```
http(method="POST", url="https://api.us.nylas.com/v3/grants/{grant_id}/messages/send", body={"to": [{"email": "recipient@example.com","name": "Jane"}],"subject": "Hello","body": "Message body"})
```

**List events:**
```
http(method="GET", url="https://api.us.nylas.com/v3/grants/{grant_id}/events?calendar_id=primary&limit=10")
```

**Create event:**
```
http(method="POST", url="https://api.us.nylas.com/v3/grants/{grant_id}/events?calendar_id=primary", body={"title": "Meeting","when": {"start_time": 1711540800,"end_time": 1711544400}})
```

## Notes

- Grant ID represents a connected account.
- Supports email (Gmail, Outlook, IMAP) and calendar.
- Times are Unix timestamps.
- Calendar ID `primary` for the default calendar.
