---
name: neon
version: "1.0.0"
description: Neon API — Neon is a cloud-native, serverless PostgreSQL platform that provides instant dat
activation:
  keywords:
    - "neon"
  patterns:
    - "(?i)neon"
  tags:
    - "tools"
    - "Cloud"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [NEON_API_KEY]
---

# Neon API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

## Base URL

`https://console.neon.tech/api/v2`

## Actions

**List projects:**
```
http(method="GET", url="https://console.neon.tech/api/v2/projects")
```

**Get project:**
```
http(method="GET", url="https://console.neon.tech/api/v2/projects/{project_id}")
```

**Create project:**
```
http(method="POST", url="https://console.neon.tech/api/v2/projects", body={"project": {"name": "my-project"}})
```

**List branches:**
```
http(method="GET", url="https://console.neon.tech/api/v2/projects/{project_id}/branches")
```

**Create branch:**
```
http(method="POST", url="https://console.neon.tech/api/v2/projects/{project_id}/branches", body={"branch": {"name": "dev"},"endpoints": [{"type": "read_write"}]})
```

## Notes

- Neon is serverless PostgreSQL.
- Branches are copy-on-write database forks.
- Endpoints are compute instances attached to branches.
- Connection strings in endpoint details.
