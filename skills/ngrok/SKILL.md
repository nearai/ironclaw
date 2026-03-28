---
name: ngrok
version: "1.0.0"
description: Ngrok API — Ngrok is a secure connectivity platform that enables developers to expose local 
activation:
  keywords:
    - "ngrok"
    - "connectivity"
  patterns:
    - "(?i)ngrok"
  tags:
    - "tools"
    - "connectivity"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [NGROK_API_KEY]
---

# Ngrok API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

## Base URL

`https://api.ngrok.com`

## Actions

**List tunnels:**
```
http(method="GET", url="https://api.ngrok.com/tunnels")
```

**List endpoints:**
```
http(method="GET", url="https://api.ngrok.com/endpoints")
```

**List domains:**
```
http(method="GET", url="https://api.ngrok.com/reserved_domains")
```

**Create reserved domain:**
```
http(method="POST", url="https://api.ngrok.com/reserved_domains", body={"name": "myapp","region": "us"})
```

## Notes

- Auth via `Authorization: Bearer {token}` or `Ngrok-Version: 2` header.
- Tunnels expose local services to the internet.
- Reserved domains persist across tunnel restarts.
- Regions: `us`, `eu`, `ap`, `au`, `sa`, `jp`, `in`.
