---
name: resend
version: "1.0.0"
description: Resend API — send transactional emails, manage domains, API keys
activation:
  keywords:
    - "resend"
    - "transactional email"
    - "send email"
  exclude_keywords:
    - "gmail"
    - "postmark"
    - "sendgrid"
  patterns:
    - "(?i)resend.*(email|send|domain)"
    - "(?i)send.*email.*resend"
  tags:
    - "email"
    - "transactional"
  max_context_tokens: 1000
metadata:
  openclaw:
    requires:
      env: [RESEND_API_KEY]
---

# Resend API

Use the `http` tool. Credentials are automatically injected for `api.resend.com`.

## Base URL

`https://api.resend.com`

## Actions

**Send email:**
```
http(method="POST", url="https://api.resend.com/emails", body={"from": "noreply@yourdomain.com", "to": ["recipient@example.com"], "subject": "Hello", "html": "<p>Body text</p>"})
```

**Send with cc/bcc/reply_to:**
```
http(method="POST", url="https://api.resend.com/emails", body={"from": "Team <team@yourdomain.com>", "to": ["alice@example.com"], "cc": ["bob@example.com"], "bcc": ["archive@yourdomain.com"], "reply_to": "support@yourdomain.com", "subject": "Update", "html": "<h1>News</h1><p>Details here</p>"})
```

**Get email status:**
```
http(method="GET", url="https://api.resend.com/emails/<email_id>")
```

**List domains:**
```
http(method="GET", url="https://api.resend.com/domains")
```

**Add domain:**
```
http(method="POST", url="https://api.resend.com/domains", body={"name": "yourdomain.com"})
```

## Notes

- `from` must use a verified domain. Format: `"Name <email@domain.com>"` or just `"email@domain.com"`.
- `to`, `cc`, `bcc` are arrays of email strings.
- Use `html` for HTML content or `text` for plain text.
- Response: `{"id": "email_id"}`. Status is trackable via GET.
- Email statuses: `sent`, `delivered`, `bounced`, `complained`.
