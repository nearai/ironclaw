---
name: linear
version: "1.0.0"
description: Linear issue tracker API integration
activation:
  keywords:
    - "linear"
    - "ticket"
    - "sprint"
    - "backlog"
    - "roadmap"
  exclude_keywords:
    - "jira"
    - "asana"
  patterns:
    - "(?i)(create|list|show|assign|close|update)\\s.*(issue|ticket|task|bug)"
    - "(?i)linear\\.app"
  tags:
    - "project-management"
    - "issue-tracking"
  max_context_tokens: 2000
credentials:
  - name: linear_api_key
    provider: linear
    location:
      type: bearer
    hosts:
      - "api.linear.app"
    setup_instructions: "Create an API key at https://linear.app/settings/api"
---

# Linear API Skill

You have access to the Linear GraphQL API via the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**. When the URL host is `api.linear.app`, the system injects `Authorization: Bearer {linear_api_key}` transparently.

## API Patterns

Linear uses a single GraphQL endpoint: `https://api.linear.app/graphql`

All requests are `POST` with a JSON body containing `query` and optional `variables`.

### List Issues

```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "{ issues(first: 20, orderBy: updatedAt) { nodes { id identifier title state { name } assignee { name } priority priorityLabel createdAt } } }"})
```

### Get Issue by Identifier

```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "query($id: String!) { issue(id: $id) { id identifier title description state { name } assignee { name } labels { nodes { name } } comments { nodes { body user { name } createdAt } } } }", "variables": {"id": "ISSUE_ID"}})
```

### Search Issues

```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "query($term: String!) { issueSearch(query: $term, first: 10) { nodes { id identifier title state { name } priorityLabel } } }", "variables": {"term": "SEARCH_TERM"}})
```

### Create Issue

```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "mutation($input: IssueCreateInput!) { issueCreate(input: $input) { success issue { id identifier title url } } }", "variables": {"input": {"title": "...", "description": "...", "teamId": "TEAM_ID", "priority": 2}}})
```

### List Teams (to get teamId for issue creation)

```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "{ teams { nodes { id name key } } }"})
```

### Update Issue State

```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "mutation($id: String!, $stateId: String!) { issueUpdate(id: $id, input: { stateId: $stateId }) { success issue { id identifier title state { name } } } }", "variables": {"id": "ISSUE_UUID", "stateId": "STATE_UUID"}})
```

## Response Handling

- Linear returns `{"data": {...}}` on success, `{"errors": [...]}` on failure.
- Issue identifiers look like `ENG-123` (team key + number).
- Always check for `errors` in the response before processing `data`.
- GraphQL errors include a `message` and optional `extensions` with error codes.

## Common Mistakes

- Do NOT add an `Authorization` header — it is injected automatically.
- Always use `POST` method — Linear's API is GraphQL only.
- The `id` field is a UUID, the `identifier` field is human-readable (e.g., `ENG-42`).
- Use `issueSearch` for text search, not `issues` with a filter (text search is separate).
- When creating issues, you MUST provide `teamId`. List teams first if unknown.
