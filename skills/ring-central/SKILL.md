---
name: ring-central
version: "1.0.0"
description: Ring Central API — RingCentral offers a cloud‑based unified communications platform that combines b
activation:
  keywords:
    - "ring-central"
    - "ring central"
    - "communication"
  patterns:
    - "(?i)ring.?central"
  tags:
    - "messaging"
    - "communication"
    - "chat"
  max_context_tokens: 1200
---

# Ring Central API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://platform.ringcentral.com/restapi/v1.0`

## Actions

**Get account info:**
```
http(method="GET", url="https://platform.ringcentral.com/restapi/v1.0/account/~")
```

**Send SMS:**
```
http(method="POST", url="https://platform.ringcentral.com/restapi/v1.0/account/~/extension/~/sms", body={"from": {"phoneNumber": "+1234567890"},"to": [{"phoneNumber": "+0987654321"}],"text": "Hello!"})
```

**List messages:**
```
http(method="GET", url="https://platform.ringcentral.com/restapi/v1.0/account/~/extension/~/message-store?messageType=SMS&perPage=10")
```

**List call log:**
```
http(method="GET", url="https://platform.ringcentral.com/restapi/v1.0/account/~/extension/~/call-log?perPage=10")
```

## Notes

- Uses OAuth 2.0 — credentials are auto-injected.
- `~` in path means current account/extension.
- Phone numbers in E.164 format.
- Message types: `SMS`, `Pager`, `Fax`, `VoiceMail`.
