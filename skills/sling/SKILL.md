---
name: sling
version: "1.0.0"
description: Sling API — Sling simplifies workforce management by combining employee shift scheduling
activation:
  keywords:
    - "sling"
    - "hr"
  patterns:
    - "(?i)sling"
  tags:
    - "hr"
    - "recruitment"
    - "workforce"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SLING_AUTHORIZATION_KEY]
---

# Sling API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

## Base URL

`https://api.getsling.com/v1`

## Actions

**List users:**
```
http(method="GET", url="https://api.getsling.com/v1/{org_id}/users")
```

**List shifts:**
```
http(method="GET", url="https://api.getsling.com/v1/{org_id}/calendar/{from}/{to}")
```

**List locations:**
```
http(method="GET", url="https://api.getsling.com/v1/{org_id}/locations")
```

## Notes

- Sling is a workforce scheduling platform.
- Date range format: `YYYY-MM-DD`.
- Org ID required in most paths.
- Auth via `Authorization` header (auto-injected).
