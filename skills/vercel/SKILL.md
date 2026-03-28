---
name: vercel
version: "1.0.0"
description: Vercel API — deployments, projects, domains, environment variables, logs
activation:
  keywords:
    - "vercel"
    - "vercel deploy"
    - "vercel project"
  exclude_keywords:
    - "netlify"
    - "heroku"
  patterns:
    - "(?i)vercel.*(deploy|project|domain|env)"
    - "(?i)(deploy|rollback).*vercel"
  tags:
    - "deployment"
    - "hosting"
    - "devops"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [VERCEL_TOKEN]
---

# Vercel API

Use the `http` tool. Credentials are automatically injected for `api.vercel.com`.

## Base URL

`https://api.vercel.com`

## Actions

**List projects:**
```
http(method="GET", url="https://api.vercel.com/v9/projects?limit=20")
```

**Get project:**
```
http(method="GET", url="https://api.vercel.com/v9/projects/<project_name_or_id>")
```

**List deployments:**
```
http(method="GET", url="https://api.vercel.com/v6/deployments?projectId=<project_id>&limit=10")
```

**Get deployment:**
```
http(method="GET", url="https://api.vercel.com/v13/deployments/<deployment_id_or_url>")
```

**List environment variables:**
```
http(method="GET", url="https://api.vercel.com/v9/projects/<project_id>/env")
```

**Create environment variable:**
```
http(method="POST", url="https://api.vercel.com/v10/projects/<project_id>/env", body={"key": "API_KEY", "value": "secret123", "type": "encrypted", "target": ["production", "preview"]})
```

**List domains:**
```
http(method="GET", url="https://api.vercel.com/v9/projects/<project_id>/domains")
```

**Add domain:**
```
http(method="POST", url="https://api.vercel.com/v10/projects/<project_id>/domains", body={"name": "app.example.com"})
```

**Promote deployment (alias):**
```
http(method="POST", url="https://api.vercel.com/v10/projects/<project_id>/promote/<deployment_id>")
```

**Get deployment build logs:**
```
http(method="GET", url="https://api.vercel.com/v7/deployments/<deployment_id>/events")
```

## Notes

- Deployment states: `BUILDING`, `READY`, `ERROR`, `CANCELED`, `QUEUED`.
- Env var targets: `production`, `preview`, `development`.
- Env var types: `plain`, `encrypted`, `secret`, `system`.
- Team scope: add `?teamId=<team_id>` to all requests if using team account.
- Pagination: `limit` + `until` (timestamp) or `next` cursor.
