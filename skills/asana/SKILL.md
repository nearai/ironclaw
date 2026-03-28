---
name: asana
version: "1.0.0"
description: Asana REST API — tasks, projects, sections, comments, search
activation:
  keywords:
    - "asana"
    - "asana task"
    - "asana project"
  exclude_keywords:
    - "jira"
    - "trello"
  patterns:
    - "(?i)asana.*(task|project|section)"
    - "(?i)(create|list|update|complete).*asana"
  tags:
    - "project-management"
    - "task-management"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [ASANA_ACCESS_TOKEN]
---

# Asana REST API

Use the `http` tool. Credentials are automatically injected for `app.asana.com`.

## Base URL

`https://app.asana.com/api/1.0`

## Actions

**List my tasks:**
```
http(method="GET", url="https://app.asana.com/api/1.0/user_task_lists/me/tasks?workspace=<workspace_gid>&opt_fields=name,completed,due_on,assignee_section.name&limit=20")
```

**Get task:**
```
http(method="GET", url="https://app.asana.com/api/1.0/tasks/<task_gid>?opt_fields=name,notes,completed,due_on,assignee.name,projects.name,tags.name")
```

**Create task:**
```
http(method="POST", url="https://app.asana.com/api/1.0/tasks", body={"data": {"name": "Task title", "notes": "Description", "projects": ["<project_gid>"], "due_on": "2026-04-01", "assignee": "me"}})
```

**Update task:**
```
http(method="PUT", url="https://app.asana.com/api/1.0/tasks/<task_gid>", body={"data": {"completed": true}})
```

**List project tasks:**
```
http(method="GET", url="https://app.asana.com/api/1.0/projects/<project_gid>/tasks?opt_fields=name,completed,due_on,assignee.name&limit=50")
```

**Add comment:**
```
http(method="POST", url="https://app.asana.com/api/1.0/tasks/<task_gid>/stories", body={"data": {"text": "Comment text"}})
```

**Search tasks:**
```
http(method="GET", url="https://app.asana.com/api/1.0/workspaces/<workspace_gid>/tasks/search?text=search+term&opt_fields=name,completed&limit=20")
```

**List projects:**
```
http(method="GET", url="https://app.asana.com/api/1.0/projects?workspace=<workspace_gid>&opt_fields=name,current_status&limit=50")
```

**List workspaces:**
```
http(method="GET", url="https://app.asana.com/api/1.0/workspaces")
```

## Notes

- All response data is wrapped in `{"data": ...}`. Errors: `{"errors": [{"message": "..."}]}`.
- Use `opt_fields` to request specific fields (comma-separated). Without it, you get minimal data.
- GIDs are numeric strings like `"1234567890123456"`.
- Dates are ISO format `YYYY-MM-DD`. Due times use `due_at` (ISO 8601 with timezone).
- Pagination: `offset` token from `next_page.offset`. Check `next_page` for more.
