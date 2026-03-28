---
name: click-send
version: "1.0.0"
description: ClickSend API — A cloud-based messaging and communication service that enables businesses to sen
activation:
  keywords:
    - "click-send"
    - "clicksend"
    - "communication"
  patterns:
    - "(?i)click.?send"
  tags:
    - "messaging"
    - "communication"
    - "chat"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CLICK_SEND_USERNAME, CLICK_SEND_PASSWORD]
---

# ClickSend API

Use the `http` tool. Credentials are automatically injected.

## Base URL

`https://rest.clicksend.com/v3`

## Actions

**Send SMS:**
```
http(method="POST", url="https://rest.clicksend.com/v3/sms/send", body={"messages": [{"to": "+1234567890","body": "Hello!","source": "sdk"}]})
```

**Get SMS history:**
```
http(method="GET", url="https://rest.clicksend.com/v3/sms/history?page=1&limit=10")
```

**Send email:**
```
http(method="POST", url="https://rest.clicksend.com/v3/email/send", body={"to": [{"email": "recipient@example.com","name": "John"}],"from": {"email": "sender@example.com"},"subject": "Hello","body": "<p>Content</p>"})
```

## Notes

- Uses Basic auth with username and API key.
- SMS `to` must include country code (e.g., `+1234567890`).
- Supports SMS, MMS, email, voice, fax, and postal mail.
