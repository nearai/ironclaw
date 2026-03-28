---
name: webflow
version: "1.0.0"
description: Webflow API — Webflow is a no-code website builder that enables users to design, build
activation:
  keywords:
    - "webflow"
    - "tools"
  patterns:
    - "(?i)webflow"
  tags:
    - "tools"
    - "utility"
    - "tool"
  max_context_tokens: 1200
---

# Webflow API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.webflow.com/v2`

## Actions

**List sites:**
```
http(method="GET", url="https://api.webflow.com/v2/sites")
```

**Get site:**
```
http(method="GET", url="https://api.webflow.com/v2/sites/{site_id}")
```

**List collections:**
```
http(method="GET", url="https://api.webflow.com/v2/sites/{site_id}/collections")
```

**List items:**
```
http(method="GET", url="https://api.webflow.com/v2/collections/{collection_id}/items?limit=10")
```

**Create item:**
```
http(method="POST", url="https://api.webflow.com/v2/collections/{collection_id}/items", body={"fieldData": {"name": "New Post","slug": "new-post"},"isArchived": false,"isDraft": false})
```

**Publish site:**
```
http(method="POST", url="https://api.webflow.com/v2/sites/{site_id}/publish", body={"domains": ["mysite.webflow.io"]})
```

## Notes

- Collections are CMS content types.
- Items have `fieldData` matching collection field schema.
- Draft items need explicit publishing.
- Pagination: `offset` and `limit` params.
