---
name: figma
version: "1.0.0"
description: Figma API — files, components, comments, images, projects
activation:
  keywords:
    - "figma"
    - "figma file"
    - "figma component"
    - "design"
  exclude_keywords:
    - "sketch"
    - "canva"
  patterns:
    - "(?i)figma.*(file|component|comment|image|project)"
    - "(?i)(export|inspect).*figma"
  tags:
    - "design"
    - "ui"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [FIGMA_ACCESS_TOKEN]
---

# Figma API

Use the `http` tool. Credentials are automatically injected for `api.figma.com`.

## Base URL

`https://api.figma.com/v1`

## Actions

**Get file:**
```
http(method="GET", url="https://api.figma.com/v1/files/<file_key>")
```

**Get specific nodes:**
```
http(method="GET", url="https://api.figma.com/v1/files/<file_key>/nodes?ids=<node_id1>,<node_id2>")
```

**Export images:**
```
http(method="GET", url="https://api.figma.com/v1/images/<file_key>?ids=<node_id>&format=png&scale=2")
```

**List components:**
```
http(method="GET", url="https://api.figma.com/v1/files/<file_key>/components")
```

**List comments:**
```
http(method="GET", url="https://api.figma.com/v1/files/<file_key>/comments")
```

**Post comment:**
```
http(method="POST", url="https://api.figma.com/v1/files/<file_key>/comments", body={"message": "Looks good!", "client_meta": {"x": 100, "y": 200}})
```

**List project files:**
```
http(method="GET", url="https://api.figma.com/v1/projects/<project_id>/files")
```

**List team projects:**
```
http(method="GET", url="https://api.figma.com/v1/teams/<team_id>/projects")
```

**Get file versions:**
```
http(method="GET", url="https://api.figma.com/v1/files/<file_key>/versions")
```

## Notes

- File key is from URL: `figma.com/file/<file_key>/File-Name`.
- Node IDs use colon format: `"1:23"`. URL-encode as `1%3A23`.
- Export formats: `jpg`, `png`, `svg`, `pdf`. Scale: 0.01 to 4.
- Image export returns URLs (temporary, valid ~14 days).
- Full file response can be very large — use `/nodes` endpoint for specific sections.
- Rate limit: 30 requests/minute.
