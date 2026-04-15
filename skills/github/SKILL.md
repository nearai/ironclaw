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

### Authenticated User & Cross-Repo Queries

When the user says "my PRs", "my issues", or "my repos", they mean the user who owns `github_token`. Don't try to list a single repo, hit the search/user endpoints instead.

**Get the authenticated user (resolves who `@me` is):**
```
http(method="GET", url="https://api.github.com/user")
```

**My latest PRs across all repos:**
```
http(method="GET", url="https://api.github.com/search/issues?q=is:pr+author:%40me+sort:updated-desc&per_page=20")
```

**My open issues across all repos (assigned to me):**
```
http(method="GET", url="https://api.github.com/search/issues?q=is:issue+is:open+assignee:%40me&per_page=20")
```

**PRs that need my review:**
```
http(method="GET", url="https://api.github.com/search/issues?q=is:pr+is:open+review-requested:%40me")
```

**My repos (list all repos accessible to the token):**
```
http(method="GET", url="https://api.github.com/user/repos?sort=updated&per_page=30")
```

### Search

GitHub has three search endpoints. Build queries with the [search syntax](https://docs.github.com/en/search-github/searching-on-github).

**Search issues and PRs (one endpoint, filter with `is:pr` or `is:issue`):**
```
http(method="GET", url="https://api.github.com/search/issues?q=repo:{owner}/{repo}+is:pr+is:open+label:bug")
```

**Search code:**
```
http(method="GET", url="https://api.github.com/search/code?q=fn+main+language:rust+repo:{owner}/{repo}")
```

**Search repositories:**
```
http(method="GET", url="https://api.github.com/search/repositories?q=tetris+language:rust&sort=stars")
```

URL-encode `@` as `%40` and spaces as `+` in `q=` values.

## Response Handling

The `http` tool returns an envelope:

```python
{"status": 200, "headers": {...}, "body": <parsed value>}
```

- **JSON endpoints** — `body` is already a parsed Python dict or list. Do **not** call `json.loads()` on it. Example:
  ```python
  r = await http(method="GET", url="https://api.github.com/repos/nearai/ironclaw/pulls/2483")
  if r["status"] != 200:
      FINAL(f"GitHub returned HTTP {r['status']}: {r['body']}")
  pr = r["body"]          # dict, not a string
  title  = pr["title"]    # use direct indexing; these keys always exist on a 2xx
  state  = pr["state"]
  head   = pr["head"]["ref"]
  base   = pr["base"]["ref"]
  ```
- **Diff / plain text endpoints** (`Accept: application/vnd.github.v3.diff` etc.) — `body` is a `str` containing the raw unified diff; use it as-is.
- **Never** write `body = pr_meta.get("body", pr_meta)` as a "safety net" — it hides real errors. If `status` isn't 2xx, fail fast.
- For list endpoints, check the `Link` header for pagination.
- Rate limit: 5000 req/hour authenticated. Check `X-RateLimit-Remaining` if doing bulk ops.
- Error responses are JSON of the form `{"message": "..."}` with a non-2xx `status` — surface them literally in your FINAL answer.

## Common Mistakes

- Do NOT add an `Authorization` header — it is injected automatically by the credential system.
- Always use HTTPS URLs (HTTP is blocked by the security layer).
- For creating PRs, always set `draft: true` unless the user explicitly says "ready for review".
- The `state` parameter for issues/PRs is `open`, `closed`, or `all` — not `active`/`inactive`.
- Use `per_page` to control result count (max 100). Default is 30.
- For "my PRs / my issues" across all repos, hit `/search/issues?q=...+author:%40me`. Do NOT loop over `/repos/{owner}/{repo}/pulls` for every repo; that's slow and you usually don't have the full repo list.
- The unified endpoint `/search/issues` returns BOTH issues and pull requests, filter with `is:pr` or `is:issue`. There is no `/search/pulls` endpoint.
