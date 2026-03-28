---
name: open-phone
version: "1.0.0"
description: OpenPhone API — OpenPhone is a modern business phone system that unifies VoIP calls
activation:
  keywords:
    - "open-phone"
    - "openphone"
    - "other"
  patterns:
    - "(?i)open.?phone"
  tags:
    - "tools"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [OPENPHONE_API_KEY]
---

# OpenPhone API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

## Base URL

`https://api.openphone.com/v1`

## Actions

**List phone numbers:**
```
http(method="GET", url="https://api.openphone.com/v1/phone-numbers")
```

**Send message:**
```
http(method="POST", url="https://api.openphone.com/v1/messages", body={"from": "+1234567890","to": ["+0987654321"],"content": "Hello!"})
```

**List messages:**
```
http(method="GET", url="https://api.openphone.com/v1/messages?phoneNumberId={phone_number_id}&maxResults=10")
```

**List calls:**
```
http(method="GET", url="https://api.openphone.com/v1/calls?phoneNumberId={phone_number_id}&maxResults=10")
```

## Notes

- Phone numbers in E.164 format: `+1234567890`.
- Messages can be SMS or MMS.
- Calls include direction, duration, and recording URLs.
- Pagination: `pageToken` from response.
