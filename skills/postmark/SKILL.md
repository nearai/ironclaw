---
name: postmark
version: "1.0.0"
description: Postmark API — send transactional emails, templates, message streams
activation:
  keywords:
    - "postmark"
    - "postmark email"
  exclude_keywords:
    - "resend"
    - "sendgrid"
    - "gmail"
  patterns:
    - "(?i)postmark.*(email|send|template)"
  tags:
    - "email"
    - "transactional"
  max_context_tokens: 1000
metadata:
  openclaw:
    requires:
      env: [POSTMARK_SERVER_TOKEN]
---

# Postmark API

Use the `http` tool. Include the server token as `X-Postmark-Server-Token` header.

## Base URL

`https://api.postmarkapp.com`

## Actions

**Send email:**
```
http(method="POST", url="https://api.postmarkapp.com/email", headers=[{"name": "X-Postmark-Server-Token", "value": "{POSTMARK_SERVER_TOKEN}"}], body={"From": "sender@yourdomain.com", "To": "recipient@example.com", "Subject": "Hello", "HtmlBody": "<p>Body</p>", "MessageStream": "outbound"})
```

**Send with template:**
```
http(method="POST", url="https://api.postmarkapp.com/email/withTemplate", headers=[{"name": "X-Postmark-Server-Token", "value": "{POSTMARK_SERVER_TOKEN}"}], body={"From": "sender@yourdomain.com", "To": "recipient@example.com", "TemplateId": 12345, "TemplateModel": {"name": "John", "action_url": "https://example.com/confirm"}, "MessageStream": "outbound"})
```

**Get delivery stats:**
```
http(method="GET", url="https://api.postmarkapp.com/deliverystats", headers=[{"name": "X-Postmark-Server-Token", "value": "{POSTMARK_SERVER_TOKEN}"}])
```

**Search outbound messages:**
```
http(method="GET", url="https://api.postmarkapp.com/messages/outbound?count=20&offset=0&recipient=user@example.com", headers=[{"name": "X-Postmark-Server-Token", "value": "{POSTMARK_SERVER_TOKEN}"}])
```

**Get bounces:**
```
http(method="GET", url="https://api.postmarkapp.com/bounces?count=20&offset=0", headers=[{"name": "X-Postmark-Server-Token", "value": "{POSTMARK_SERVER_TOKEN}"}])
```

## Notes

- MessageStream: `outbound` (transactional) or `broadcast` (bulk).
- Send response includes `MessageID` for tracking.
- Error response: `{"ErrorCode": 300, "Message": "..."}`.
- `From` must be a verified sender signature.
- Pagination: `count` + `offset` (0-based).
