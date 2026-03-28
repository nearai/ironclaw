---
name: clickup
version: "1.0.0"
description: ClickUp API v2 — tasks, spaces, lists, comments, time tracking
activation:
  keywords:
    - "clickup"
    - "clickup task"
    - "clickup space"
  exclude_keywords:
    - "jira"
    - "asana"
  patterns:
    - "(?i)clickup.*(task|space|list|folder)"
    - "(?i)(create|list|update).*clickup"
  tags:
    - "project-management"
    - "task-management"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [CLICKUP_API_TOKEN]
---

# ClickUp API v2

Use the `http` tool. Credentials are automatically injected for `api.clickup.com`.

## Base URL

`https://api.clickup.com/api/v2`

## Actions

**List workspaces (teams):**
```
http(method="GET", url="https://api.clickup.com/api/v2/team")
```

**List spaces:**
```
http(method="GET", url="https://api.clickup.com/api/v2/team/<team_id>/space")
```

**List folders in space:**
```
http(method="GET", url="https://api.clickup.com/api/v2/space/<space_id>/folder")
```

**List tasks in a list:**
```
http(method="GET", url="https://api.clickup.com/api/v2/list/<list_id>/task?page=0&subtasks=true&include_closed=false")
```

**Get task:**
```
http(method="GET", url="https://api.clickup.com/api/v2/task/<task_id>")
```

**Create task:**
```
http(method="POST", url="https://api.clickup.com/api/v2/list/<list_id>/task", body={"name": "Task name", "description": "Details", "status": "to do", "priority": 2, "due_date": 1775000000000, "assignees": [123456]})
```

**Update task:**
```
http(method="PUT", url="https://api.clickup.com/api/v2/task/<task_id>", body={"status": "in progress", "priority": 1})
```

**Add comment:**
```
http(method="POST", url="https://api.clickup.com/api/v2/task/<task_id>/comment", body={"comment_text": "My comment"})
```

## Notes

- Priority: 1=Urgent, 2=High, 3=Normal, 4=Low.
- Due dates are Unix timestamps in **milliseconds**.
- Status values are lowercase strings matching your workspace statuses.
- Task IDs are alphanumeric strings like `"abc123"`.
- Hierarchy: Team → Space → Folder → List → Task.
- Pagination: `page` param (0-indexed). Returns empty array when no more.
