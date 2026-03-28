---
name: circle-ci
version: "1.0.0"
description: Circle CI API — CircleCI is a continuous integration and delivery (CI/CD) platform that automate
activation:
  keywords:
    - "circle-ci"
    - "circle ci"
    - "devops"
  patterns:
    - "(?i)circle.?ci"
  tags:
    - "devops"
    - "ci-cd"
    - "deployment"
    - "dev-ops"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CIRCLE_CI_API_KEY]
---

# Circle CI API

Use the `http` tool. API key is automatically injected via `Circle-Token` header — **never construct auth headers manually**.

## Base URL

`https://circleci.com/api/v2`

## Actions

**Get current user:**
```
http(method="GET", url="https://circleci.com/api/v2/me")
```

**List pipelines:**
```
http(method="GET", url="https://circleci.com/api/v2/project/{project_slug}/pipeline?branch=main")
```

**Get pipeline:**
```
http(method="GET", url="https://circleci.com/api/v2/pipeline/{pipeline_id}")
```

**List workflows:**
```
http(method="GET", url="https://circleci.com/api/v2/pipeline/{pipeline_id}/workflow")
```

**Trigger pipeline:**
```
http(method="POST", url="https://circleci.com/api/v2/project/{project_slug}/pipeline", body={"branch": "main","parameters": {}})
```

## Notes

- Project slug format: `gh/{org}/{repo}` or `bb/{org}/{repo}`.
- Pipeline statuses: `created`, `errored`, `setup-pending`, `setup`, `pending`.
- Workflow statuses: `success`, `running`, `not_run`, `failed`, `error`, `failing`, `on_hold`, `canceled`.
