---
name: jira
version: "1.0.0"
description: Jira Cloud REST API — issues, projects, sprints, transitions, comments
activation:
  keywords:
    - "jira"
    - "jira issue"
    - "sprint"
    - "jira ticket"
    - "story points"
  exclude_keywords:
    - "linear"
    - "github issues"
  patterns:
    - "(?i)(create|list|update|close|assign|move)\\s.*(issue|ticket|story|bug|task).*jira"
    - "(?i)jira.*(issue|board|sprint|project)"
  tags:
    - "project-management"
    - "issue-tracking"
    - "devops"
  max_context_tokens: 2000
metadata:
  openclaw:
    requires:
      env: [JIRA_DOMAIN, JIRA_EMAIL, JIRA_API_TOKEN]
---

# Jira Cloud REST API

Use the `http` tool. Auth uses Basic authentication (email:api_token base64-encoded). Credentials are automatically injected for your Jira domain.

## Base URL

`https://{JIRA_DOMAIN}.atlassian.net/rest/api/3`

## Actions

**Search issues (JQL):**
```
http(method="GET", url="https://{domain}.atlassian.net/rest/api/3/search?jql=project%3DPROJ%20AND%20status%21%3DDone&maxResults=20")
```

**Get issue:**
```
http(method="GET", url="https://{domain}.atlassian.net/rest/api/3/issue/PROJ-123")
```

**Create issue:**
```
http(method="POST", url="https://{domain}.atlassian.net/rest/api/3/issue", body={"fields": {"project": {"key": "PROJ"}, "summary": "Bug title", "description": {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Description"}]}]}, "issuetype": {"name": "Bug"}}})
```

**Update issue:**
```
http(method="PUT", url="https://{domain}.atlassian.net/rest/api/3/issue/PROJ-123", body={"fields": {"summary": "Updated title"}})
```

**Transition issue (change status):**
```
http(method="POST", url="https://{domain}.atlassian.net/rest/api/3/issue/PROJ-123/transitions", body={"transition": {"id": "31"}})
```

**Get available transitions:**
```
http(method="GET", url="https://{domain}.atlassian.net/rest/api/3/issue/PROJ-123/transitions")
```

**Add comment:**
```
http(method="POST", url="https://{domain}.atlassian.net/rest/api/3/issue/PROJ-123/comment", body={"body": {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "My comment"}]}]}})
```

**Assign issue:**
```
http(method="PUT", url="https://{domain}.atlassian.net/rest/api/3/issue/PROJ-123/assignee", body={"accountId": "<account_id>"})
```

**List projects:**
```
http(method="GET", url="https://{domain}.atlassian.net/rest/api/3/project?maxResults=50")
```

**Get board sprints:**
```
http(method="GET", url="https://{domain}.atlassian.net/rest/agile/1.0/board/<board_id>/sprint?state=active")
```

## Notes

- Jira v3 API uses Atlassian Document Format (ADF) for description/comment bodies, not plain text.
- JQL must be URL-encoded. Common: `project=PROJ AND status!=Done ORDER BY created DESC`.
- To change status, first GET transitions to find the transition ID, then POST it.
- Issue types: Bug, Task, Story, Epic, Sub-task (case-sensitive, project-dependent).
- Pagination: `startAt` + `maxResults` params. Check `total` in response.
