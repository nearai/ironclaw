---
name: outlook-mail
version: "1.0.0"
description: Outlook Mail API — An email platform that offers robust inbox organization
activation:
  keywords:
    - "outlook-mail"
    - "outlook mail"
    - "ai"
  patterns:
    - "(?i)outlook.?mail"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
---

# Outlook Mail API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://graph.microsoft.com/v1.0`

## Actions

**List messages:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/me/messages?$top=10&$orderby=receivedDateTime+desc")
```

**Get message:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/me/messages/{message_id}")
```

**Send email:**
```
http(method="POST", url="https://graph.microsoft.com/v1.0/me/sendMail", body={"message": {"subject": "Hello","body": {"contentType": "HTML","content": "<p>Email body</p>"},"toRecipients": [{"emailAddress": {"address": "recipient@example.com"}}]}})
```

**Search messages:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/me/messages?$search="keyword"&$top=10")
```

**List folders:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/me/mailFolders")
```

## Notes

- Uses OAuth 2.0 via Microsoft Graph — credentials are auto-injected.
- Content types: `HTML` or `Text`.
- Use `$search` for KQL search queries.
- `$filter` supports OData expressions: `from/emailAddress/address eq 'john@example.com'`.
