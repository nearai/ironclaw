---
name: linear
version: "1.0.0"
description: Linear GraphQL API — issues, projects, cycles, teams, labels
activation:
  keywords:
    - "linear"
    - "linear issue"
    - "linear ticket"
    - "cycle"
  exclude_keywords:
    - "jira"
    - "github issues"
  patterns:
    - "(?i)linear.*(issue|project|cycle|team)"
    - "(?i)(create|list|update|close).*linear"
  tags:
    - "project-management"
    - "issue-tracking"
  max_context_tokens: 1800
metadata:
  openclaw:
    requires:
      env: [LINEAR_API_KEY]
---

# Linear API

Linear uses a **GraphQL API**. Use the `http` tool to POST queries. Credentials are automatically injected — the system adds `Authorization: {LINEAR_API_KEY}` for `api.linear.app`.

## Endpoint

All requests: `POST https://api.linear.app/graphql` with `Content-Type: application/json`.

## Actions

**List my issues:**
```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "{ viewer { assignedIssues(first: 20, orderBy: updatedAt) { nodes { id identifier title state { name } priority priorityLabel assignee { name } } } } }"})
```

**Search issues:**
```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "{ issueSearch(query: \"search text\", first: 10) { nodes { id identifier title state { name } team { key } } } }"})
```

**Create issue:**
```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "mutation { issueCreate(input: { teamId: \"<team_id>\", title: \"Bug title\", description: \"Details\", priority: 2 }) { success issue { id identifier url } } }"})
```

**Update issue:**
```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "mutation { issueUpdate(id: \"<issue_id>\", input: { stateId: \"<state_id>\", priority: 1 }) { success issue { id identifier state { name } } } }"})
```

**List teams:**
```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "{ teams { nodes { id name key } } }"})
```

**List workflow states (for a team):**
```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "{ workflowStates(filter: { team: { id: { eq: \"<team_id>\" } } }) { nodes { id name type } } }"})
```

**Add comment:**
```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "mutation { commentCreate(input: { issueId: \"<issue_id>\", body: \"Comment text\" }) { success comment { id } } }"})
```

**List cycles:**
```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "{ cycles(filter: { isActive: { eq: true } }) { nodes { id name startsAt endsAt issues { nodes { identifier title state { name } } } } } }"})
```

## Notes

- Priority: 0=No priority, 1=Urgent, 2=High, 3=Medium, 4=Low.
- State types: `triage`, `backlog`, `unstarted`, `started`, `completed`, `cancelled`.
- Identifiers look like `ENG-123`. UUIDs are used for `id` fields.
- Pagination: use `first`/`after` with `pageInfo { hasNextPage endCursor }`.
- All mutations return `{ success ... }` — check `success` field.
