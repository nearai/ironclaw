---
name: todoist
version: "1.0.0"
description: Todoist REST API — tasks, projects, labels, comments, sections
activation:
  keywords:
    - "todoist"
    - "todoist task"
    - "todoist project"
    - "todo list"
  exclude_keywords:
    - "asana"
    - "trello"
  patterns:
    - "(?i)todoist.*(task|project|label)"
    - "(?i)(add|list|complete).*todoist"
  tags:
    - "task-management"
    - "productivity"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [TODOIST_API_TOKEN]
---

# Todoist REST API

Use the `http` tool. Credentials are automatically injected for `api.todoist.com`.

## Base URL

`https://api.todoist.com/rest/v2`

## Actions

**List active tasks:**
```
http(method="GET", url="https://api.todoist.com/rest/v2/tasks")
```

**Get task:**
```
http(method="GET", url="https://api.todoist.com/rest/v2/tasks/<task_id>")
```

**Create task:**
```
http(method="POST", url="https://api.todoist.com/rest/v2/tasks", body={"content": "Buy groceries", "description": "Milk, eggs, bread", "project_id": "<project_id>", "due_string": "tomorrow at 10am", "priority": 3})
```

**Complete task:**
```
http(method="POST", url="https://api.todoist.com/rest/v2/tasks/<task_id>/close")
```

**Update task:**
```
http(method="POST", url="https://api.todoist.com/rest/v2/tasks/<task_id>", body={"content": "Updated title", "priority": 4})
```

**List projects:**
```
http(method="GET", url="https://api.todoist.com/rest/v2/projects")
```

**Create project:**
```
http(method="POST", url="https://api.todoist.com/rest/v2/projects", body={"name": "Work"})
```

**Add comment:**
```
http(method="POST", url="https://api.todoist.com/rest/v2/comments", body={"task_id": "<task_id>", "content": "Note to self"})
```

## Notes

- Priority: 1=normal, 2=medium, 3=high, 4=urgent (reversed from most systems).
- `due_string` supports natural language: "tomorrow", "every monday", "Jan 5 at 3pm".
- Task IDs are numeric strings. Project IDs are also numeric.
- Filter tasks with `?project_id=X` or `?label=Y` query params.
- Completing a recurring task creates the next occurrence automatically.
