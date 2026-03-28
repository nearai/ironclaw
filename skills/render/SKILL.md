---
name: render
version: "1.0.0"
description: Render API — Render is a cloud-hosting platform that enables developers to build, deploy
activation:
  keywords:
    - "render"
    - "cloud"
  patterns:
    - "(?i)render"
  tags:
    - "cloud"
    - "infrastructure"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [RENDER_API_KEY]
---

# Render API

Use the `http` tool. API key is automatically injected via `authorization` header — **never construct auth headers manually**.

## Base URL

`https://api.render.com/v1`

## Actions

**List services:**
```
http(method="GET", url="https://api.render.com/v1/services?limit=10")
```

**Get service:**
```
http(method="GET", url="https://api.render.com/v1/services/{service_id}")
```

**List deploys:**
```
http(method="GET", url="https://api.render.com/v1/services/{service_id}/deploys?limit=10")
```

**Trigger deploy:**
```
http(method="POST", url="https://api.render.com/v1/services/{service_id}/deploys")
```

**List environments:**
```
http(method="GET", url="https://api.render.com/v1/services/{service_id}/env-vars")
```

## Notes

- Service types: `web_service`, `static_site`, `private_service`, `background_worker`, `cron_job`.
- Deploy statuses: `created`, `build_in_progress`, `update_in_progress`, `live`, `deactivated`.
- Pagination: `cursor` param from response.
