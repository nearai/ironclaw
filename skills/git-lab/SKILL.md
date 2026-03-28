---
name: git-lab
version: "1.0.0"
description: GitLab API — GitLab is a complete DevOps platform that provides a single application for sour
activation:
  keywords:
    - "git-lab"
    - "gitlab"
    - "productivity"
  patterns:
    - "(?i)git.?lab"
  tags:
    - "productivity"
    - "collaboration"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [GITLAB_HOSTNAME, GITLAB_PERSONAL_ACCESS_TOKEN]
---

# GitLab API

Use the `http` tool. API key is automatically injected via `PRIVATE-TOKEN` header — **never construct auth headers manually**.

## Base URL

`https://gitlab.com/api/v4`

## Actions

**List projects:**
```
http(method="GET", url="https://gitlab.com/api/v4/projects?membership=true&per_page=10")
```

**Get project:**
```
http(method="GET", url="https://gitlab.com/api/v4/projects/{project_id}")
```

**List issues:**
```
http(method="GET", url="https://gitlab.com/api/v4/projects/{project_id}/issues?state=opened&per_page=10")
```

**Create issue:**
```
http(method="POST", url="https://gitlab.com/api/v4/projects/{project_id}/issues", body={"title": "Bug report","description": "Details here","labels": "bug"})
```

**List merge requests:**
```
http(method="GET", url="https://gitlab.com/api/v4/projects/{project_id}/merge_requests?state=opened&per_page=10")
```

**Create merge request:**
```
http(method="POST", url="https://gitlab.com/api/v4/projects/{project_id}/merge_requests", body={"source_branch": "feature","target_branch": "main","title": "Feature implementation"})
```

**List pipelines:**
```
http(method="GET", url="https://gitlab.com/api/v4/projects/{project_id}/pipelines?per_page=10")
```

## Notes

- Project IDs are numeric or URL-encoded paths: `namespace%2Fproject`.
- Auth via `PRIVATE-TOKEN` header (auto-injected).
- Issue/MR states: `opened`, `closed`, `merged` (MR only), `all`.
- Pagination: `page` and `per_page` params; check `X-Total` header.
