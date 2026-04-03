---
name: github
version: "1.0.0"
description: GitHub API integration via HTTP tool with automatic credential injection
activation:
  keywords:
    - "github"
    - "issues"
    - "pull request"
    - "repository"
    - "commit"
    - "branch"
  exclude_keywords:
    - "gitlab"
    - "bitbucket"
  patterns:
    - "(?i)(list|show|get|fetch|open|close|create|file|merge)\\s.*(issue|PR|pull request|repo)"
    - "(?i)github\\.com"
  tags:
    - "git"
    - "code-review"
    - "devops"
  max_context_tokens: 2000
credentials:
  - name: github_token
    provider: github
    location:
      type: bearer
    hosts:
      - "api.github.com"
    oauth:
      authorization_url: "https://github.com/login/oauth/authorize"
      token_url: "https://github.com/login/oauth/access_token"
      scopes:
        - "repo"
        - "read:org"
      refresh:
        strategy: reauthorize_only
    setup_instructions: "Create a personal access token at https://github.com/settings/tokens"
---

# GitHub API Skill

You have access to the GitHub REST API via the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**. When the URL host is `api.github.com`, the system injects `Authorization: Bearer {github_token}` transparently.

## API Patterns

All endpoints use `https://api.github.com` as the base URL. Common headers are injected automatically.

### Issues

**List issues:**
```
http(method="GET", url="https://api.github.com/repos/{owner}/{repo}/issues?state=open&sort=created&direction=desc&per_page=30")
```

**Get single issue:**
```
http(method="GET", url="https://api.github.com/repos/{owner}/{repo}/issues/{number}")
```

**Create issue:**
```
http(method="POST", url="https://api.github.com/repos/{owner}/{repo}/issues", body={"title": "...", "body": "...", "labels": ["bug"]})
```

**Add comment:**
```
http(method="POST", url="https://api.github.com/repos/{owner}/{repo}/issues/{number}/comments", body={"body": "..."})
```

### Pull Requests

**List PRs:**
```
http(method="GET", url="https://api.github.com/repos/{owner}/{repo}/pulls?state=open&sort=created&direction=desc&per_page=30")
```

**Create PR:**
```
http(method="POST", url="https://api.github.com/repos/{owner}/{repo}/pulls", body={"title": "...", "body": "...", "head": "feature-branch", "base": "main", "draft": true})
```

**Get PR diff:**
```
http(method="GET", url="https://api.github.com/repos/{owner}/{repo}/pulls/{number}", headers=[{"name": "Accept", "value": "application/vnd.github.v3.diff"}])
```

### Repository

**Get repo info:**
```
http(method="GET", url="https://api.github.com/repos/{owner}/{repo}")
```

**List branches:**
```
http(method="GET", url="https://api.github.com/repos/{owner}/{repo}/branches")
```

**List recent commits:**
```
http(method="GET", url="https://api.github.com/repos/{owner}/{repo}/commits?per_page=10")
```

## Response Handling

- GitHub returns JSON. Parse the response to extract relevant fields.
- For list endpoints, check the `Link` header for pagination.
- Rate limit: 5000 req/hour authenticated. Check `X-RateLimit-Remaining` header if doing bulk operations.
- Errors return `{"message": "..."}` — always check for error responses.

## Common Mistakes

- Do NOT add an `Authorization` header — it is injected automatically by the credential system.
- Always use HTTPS URLs (HTTP is blocked by the security layer).
- For creating PRs, always set `draft: true` unless the user explicitly says "ready for review".
- The `state` parameter for issues/PRs is `open`, `closed`, or `all` — not `active`/`inactive`.
- Use `per_page` to control result count (max 100). Default is 30.
