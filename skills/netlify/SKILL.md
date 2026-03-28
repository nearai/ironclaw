---
name: netlify
version: "1.0.0"
description: Netlify API — Netlify is a frontend-first cloud platform that lets developers seamlessly build
activation:
  keywords:
    - "netlify"
    - "tools"
  patterns:
    - "(?i)netlify"
  tags:
    - "tools"
    - "utility"
    - "tool"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [NETLIFY_ACCESS_TOKEN]
---

# Netlify API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.netlify.com/api/v1`

## Actions

**List sites:**
```
http(method="GET", url="https://api.netlify.com/api/v1/sites?per_page=10")
```

**Get site:**
```
http(method="GET", url="https://api.netlify.com/api/v1/sites/{site_id}")
```

**List deploys:**
```
http(method="GET", url="https://api.netlify.com/api/v1/sites/{site_id}/deploys?per_page=10")
```

**Trigger deploy:**
```
http(method="POST", url="https://api.netlify.com/api/v1/sites/{site_id}/builds")
```

**List forms:**
```
http(method="GET", url="https://api.netlify.com/api/v1/sites/{site_id}/forms")
```

## Notes

- Site IDs are UUIDs or subdomain names.
- Deploy states: `new`, `uploading`, `uploaded`, `preparing`, `prepared`, `ready`, `error`.
- Build hooks can trigger deploys via POST to a URL.
- Pagination: `page` and `per_page` params.
