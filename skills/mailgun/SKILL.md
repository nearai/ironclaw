---
name: mailgun
version: "1.0.0"
description: Mailgun API — Mailgun is a powerful email delivery service designed for developers and busines
activation:
  keywords:
    - "mailgun"
    - "tools"
  patterns:
    - "(?i)mailgun"
  tags:
    - "tools"
    - "utility"
    - "tool"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [MAILGUN_BASE_URL, MAILGUN_API_KEY]
---

# Mailgun API

Use the `http` tool. Credentials are automatically injected.

## Base URL

`https://api.mailgun.net/v3`

**Content-Type**: `application/x-www-form-urlencoded` for POST/PUT requests.

## Actions

**Send email:**
```
http(method="POST", url="https://api.mailgun.net/v3/{domain}/messages", headers=[{"name": "Content-Type", "value": "application/x-www-form-urlencoded"}], body="from=sender@yourdomain.com&to=recipient@example.com&subject=Hello&text=Body+text")
```

**List domains:**
```
http(method="GET", url="https://api.mailgun.net/v3/domains")
```

**Get events:**
```
http(method="GET", url="https://api.mailgun.net/v3/{domain}/events?limit=10")
```

**List bounces:**
```
http(method="GET", url="https://api.mailgun.net/v3/{domain}/bounces?limit=10")
```

## Notes

- Uses Basic auth with `api` as username and API key as password.
- Send endpoint uses form-encoded body.
- `{domain}` is your verified sending domain.
- Supports `html` field for HTML emails, `attachment` for files.
