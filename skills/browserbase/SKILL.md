---
name: browserbase
version: "1.0.0"
description: Browserbase API — Browserbase is a cloud platform that lets developers run headless browsers at sc
activation:
  keywords:
    - "browserbase"
    - "ai"
  patterns:
    - "(?i)browserbase"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BROWSERBASE_API_KEY]
---

# Browserbase API

Use the `http` tool. API key is automatically injected via `X-BB-API-Key` header — **never construct auth headers manually**.

## Base URL

`https://api.browserbase.com/v1`

## Actions

**Create session:**
```
http(method="POST", url="https://api.browserbase.com/v1/sessions", body={"projectId": "<project_id>"})
```

**List sessions:**
```
http(method="GET", url="https://api.browserbase.com/v1/sessions?limit=10")
```

**Get session:**
```
http(method="GET", url="https://api.browserbase.com/v1/sessions/{session_id}")
```

## Notes

- Sessions provide a headless browser instance.
- Connect via CDP (Chrome DevTools Protocol) using the session's debug URL.
- Project ID is required for session creation.
