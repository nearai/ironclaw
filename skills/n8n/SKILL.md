---
name: n8n
version: "1.0.0"
description: n8n API — n8n is an open-source workflow automation platform that lets you connect apps an
activation:
  keywords:
    - "n8n"
    - "ai"
  patterns:
    - "(?i)n8n"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [N8N_HOST_URL, N8N_API_KEY]
---

# n8n API

Use the `http` tool. API key is automatically injected via `X-N8N-API-KEY` header — **never construct auth headers manually**.

## Base URL

`https://{N8N_HOST}/api/v1`

## Actions

**List workflows:**
```
http(method="GET", url="https://{N8N_HOST}/api/v1/workflows?limit=10")
```

**Get workflow:**
```
http(method="GET", url="https://{N8N_HOST}/api/v1/workflows/{workflow_id}")
```

**Activate workflow:**
```
http(method="PATCH", url="https://{N8N_HOST}/api/v1/workflows/{workflow_id}", body={"active": true})
```

**List executions:**
```
http(method="GET", url="https://{N8N_HOST}/api/v1/executions?limit=10")
```

## Notes

- Self-hosted: `{N8N_HOST}` is your instance URL.
- Auth via API key in `X-N8N-API-KEY` header.
- Workflow states: `active`, `inactive`.
- Executions track workflow runs with status and timing.
