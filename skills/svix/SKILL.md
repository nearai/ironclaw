---
name: svix
version: "1.0.0"
description: Svix API — Svix is a webhook delivery service that enables developers to send, receive
activation:
  keywords:
    - "svix"
    - "developer-tools"
  patterns:
    - "(?i)svix"
  tags:
    - "tools"
    - "developer-tools"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SVIX_BASE_URL, SVIX_TOKEN]
---

# Svix API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

## Base URL

`https://api.svix.com`

## Actions

**List applications:**
```
http(method="GET", url="https://api.svix.com/api/v1/app/?limit=10")
```

**Create application:**
```
http(method="POST", url="https://api.svix.com/api/v1/app/", body={"name": "My App","uid": "my-app"})
```

**Create message:**
```
http(method="POST", url="https://api.svix.com/api/v1/app/{app_id}/msg/", body={"eventType": "order.completed","payload": {"orderId": "123"}})
```

**List endpoints:**
```
http(method="GET", url="https://api.svix.com/api/v1/app/{app_id}/endpoint/?limit=10")
```

## Notes

- Svix is a webhooks-as-a-service platform.
- Applications group endpoints and messages.
- Messages are webhook payloads sent to all matching endpoints.
- Event types define the schema of webhook payloads.
