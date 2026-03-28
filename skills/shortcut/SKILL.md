---
name: shortcut
version: "1.0.0"
description: Shortcut API — Shortcut is a project-management platform tailored for software teams
activation:
  keywords:
    - "shortcut"
    - "product management"
  patterns:
    - "(?i)shortcut"
  tags:
    - "tools"
    - "product-management"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SHORTCUT_API_KEY]
---

# Shortcut API

Use the `http` tool. API key is automatically injected via `Shortcut-Token` header — **never construct auth headers manually**.

## Base URL

`https://api.app.shortcut.com/api/v3`

## Actions

**Search stories:**
```
http(method="GET", url="https://api.app.shortcut.com/api/v3/search/stories?query=state:started&page_size=10")
```

**Get story:**
```
http(method="GET", url="https://api.app.shortcut.com/api/v3/stories/{story_id}")
```

**Create story:**
```
http(method="POST", url="https://api.app.shortcut.com/api/v3/stories", body={"name": "Implement feature","story_type": "feature","workflow_state_id": 500000000})
```

**List projects:**
```
http(method="GET", url="https://api.app.shortcut.com/api/v3/projects")
```

**List workflows:**
```
http(method="GET", url="https://api.app.shortcut.com/api/v3/workflows")
```

## Notes

- Auth via `Shortcut-Token` header (auto-injected).
- Story types: `feature`, `bug`, `chore`.
- Search query syntax: `state:started`, `owner:username`, `label:"bug"`.
- Workflow states define the kanban board columns.
