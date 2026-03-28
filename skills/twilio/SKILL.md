---
name: twilio
version: "1.0.0"
description: Twilio API — Twilio is a cloud communications platform that enables developers to build SMS
activation:
  keywords:
    - "twilio"
    - "tools"
  patterns:
    - "(?i)twilio"
  tags:
    - "tools"
    - "utility"
    - "tool"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [TWILIO_ACCOUNT_SID, TWILIO_AUTH_TOKEN]
---

# Twilio API

Use the `http` tool. Credentials are automatically injected.

## Base URL

`https://api.twilio.com/2010-04-01/Accounts/{TWILIO_ACCOUNT_SID}`

**Content-Type**: `application/x-www-form-urlencoded` for POST/PUT requests.

## Actions

**Send SMS:**
```
http(method="POST", url="https://api.twilio.com/2010-04-01/Accounts/{TWILIO_ACCOUNT_SID}/Messages.json", headers=[{"name": "Content-Type", "value": "application/x-www-form-urlencoded"}], body="To=%2B1234567890&From=%2B0987654321&Body=Hello+from+Twilio")
```

**List messages:**
```
http(method="GET", url="https://api.twilio.com/2010-04-01/Accounts/{TWILIO_ACCOUNT_SID}/Messages.json?PageSize=10")
```

**Make call:**
```
http(method="POST", url="https://api.twilio.com/2010-04-01/Accounts/{TWILIO_ACCOUNT_SID}/Calls.json", headers=[{"name": "Content-Type", "value": "application/x-www-form-urlencoded"}], body="To=%2B1234567890&From=%2B0987654321&Url=http://demo.twilio.com/docs/voice.xml")
```

**List calls:**
```
http(method="GET", url="https://api.twilio.com/2010-04-01/Accounts/{TWILIO_ACCOUNT_SID}/Calls.json?PageSize=10")
```

## Notes

- Uses Basic auth with Account SID and Auth Token.
- POST bodies are form-encoded.
- Phone numbers in E.164 format (URL-encoded `+` → `%2B`).
- `.json` suffix returns JSON (default is XML).
- Pagination: `NextPageUri` in response.
