---
name: linear
version: "1.2.0"
description: Linear issue tracker API integration. Covers first-use identity bootstrap (viewer + teams cached), raw GraphQL for list/search/create/update, and the rules for handling "my issues" / "assigned to me" requests.
activation:
  keywords:
    - "linear"
    - "my linear"
    - "linear issue"
    - "linear issues"
    - "linear ticket"
    - "linear tickets"
    - "linear backlog"
    - "linear assignments"
    - "my linear issues"
    - "my linear tickets"
    - "assigned in linear"
    - "linear.app"
  exclude_keywords:
    - "jira"
    - "asana"
    - "github issue"
  patterns:
    - "(?i)linear\\.(?:app|com)"
    - "(?i)\\blinear\\b.+(issue|ticket|task|backlog|board)"
    - "(?i)(create|show|list|close|update).+linear\\s+(issue|ticket)"
  tags:
    - "project-management"
    - "issue-tracking"
  max_context_tokens: 1600
credentials:
  - name: linear_api_key
    provider: linear
    location:
      type: header
      name: Authorization
    hosts:
      - "api.linear.app"
    setup_instructions: "Create an API key at https://linear.app/settings/api"
---

# Linear API Skill

You have access to the Linear GraphQL API via the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**. When the URL host is `api.linear.app`, the system injects `Authorization: {linear_api_key}` transparently (no Bearer prefix — Linear API keys are sent raw).

## Identity bootstrap (first use)

Linear's API key does not tell you who the user IS inside Linear. Before running any "my issues" / "assigned to me" / "my tickets" request, make sure the user's Linear identity is cached. This avoids re-fetching `viewer` on every request and makes filter-by-assignee queries deterministic.

### Cache file

Path: `context/intel/linear-identity.md`

Shape:

```yaml
---
type: linear-identity
bootstrapped_at: 2026-04-21
refreshed_at: 2026-04-21
stale_after: 2026-05-21
---
# Linear identity
user_id: 8a7f...-uuid
display_name: Tobias Holenstein
email: tobias@...
timezone: Europe/Zurich

## Teams
- id: team-uuid-a, key: ENG, name: Engineering
- id: team-uuid-b, key: PROD, name: Product

## Default team
ENG
```

### Bootstrap flow

1. `memory_read("context/intel/linear-identity.md")`. If the file exists and `stale_after` is in the future, use it and stop.
2. If missing or stale, run one GraphQL call:
   ```
   query { viewer { id name displayName email } teams(first: 50) { nodes { id key name } } }
   ```
3. Write the cache via `memory_write` with `stale_after` = today + 30 days.
4. If the returned team list has exactly one team, record it as `Default team`. If more than one, ask the user once: *"I see teams ENG, PROD, OPS. Which one do you default to for new issues?"* and store the answer.
5. On HTTP 401 or an `AuthenticationError` GraphQL error, invalidate the cache and re-prompt the user to check their API key — do not silently retry.

### Using the cached identity

- "list my issues" / "what's assigned to me" → filter by `assignee: { id: { eq: "<cached user_id>" } }`, **not** by `assignee: { isMe: true }` (the `isMe` filter is not universally available and `viewer` round-trips are wasteful).
- "create an issue in my team" → use cached `Default team` id without asking.
- "create an issue in <team name>" → match against cached team names; ask only if no match.
- Skills that import external work into Linear must consume this cache rather than re-resolving identity per run.


## API Patterns

Linear uses a single GraphQL endpoint: `https://api.linear.app/graphql`

All requests are `POST` with a JSON body containing `query` and optional `variables`.

### List Issues

```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "{ issues(first: 20, orderBy: updatedAt) { nodes { id identifier title state { name } assignee { name } priority priorityLabel createdAt } } }"})
```

### List Issues Assigned to the User (uses identity cache)

```
http(method="POST", url="https://api.linear.app/graphql", body={"query": "query($uid: ID!) { issues(filter: { assignee: { id: { eq: $uid } }, state: { type: { nin: [completed, canceled] } } }, first: 50, orderBy: updatedAt) { nodes { id identifier title state { name type } priority priorityLabel url updatedAt } } }", "variables": {"uid": "<cached user_id>"}})
```

Never pass `viewer.id` inline from a fresh round-trip when the cache is valid — consult `context/intel/linear-identity.md`.

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
