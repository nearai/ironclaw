---
name: sendgrid
version: "1.0.0"
description: SendGrid API — SendGrid is a cloud-based email service that enables businesses to send transact
activation:
  keywords:
    - "sendgrid"
    - "communication"
  patterns:
    - "(?i)sendgrid"
  tags:
    - "messaging"
    - "communication"
    - "chat"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SENDGRID_API_KEY]
---

# SendGrid API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.sendgrid.com/v3`

## Actions

**Send email:**
```
http(method="POST", url="https://api.sendgrid.com/v3/mail/send", body={"personalizations": [{"to": [{"email": "recipient@example.com"}]}],"from": {"email": "sender@example.com"},"subject": "Hello","content": [{"type": "text/html","value": "<p>Body</p>"}]})
```

**List contacts:**
```
http(method="GET", url="https://api.sendgrid.com/v3/marketing/contacts?page_size=10")
```

**Add contacts:**
```
http(method="PUT", url="https://api.sendgrid.com/v3/marketing/contacts", body={"contacts": [{"email": "john@example.com","first_name": "John"}]})
```

**Get stats:**
```
http(method="GET", url="https://api.sendgrid.com/v3/stats?start_date=2026-03-01&end_date=2026-03-27")
```

## Notes

- `personalizations` is required and allows per-recipient customization.
- Content types: `text/plain`, `text/html`.
- Templates: use `template_id` instead of `content`.
- Stats include requests, delivered, opens, clicks, bounces.
