---
name: posthog
version: "1.0.0"
description: PostHog API — PostHog is a product analytics suite that provides session recording
activation:
  keywords:
    - "posthog"
    - "ai"
  patterns:
    - "(?i)posthog"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [POSTHOG_INSTANCE_URL, POSTHOG_API_KEY]
---

# PostHog API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://us.posthog.com/api`

## Actions

**Capture event:**
```
http(method="POST", url="https://us.posthog.com/api/capture/", body={"api_key": "{POSTHOG_PROJECT_API_KEY}","event": "page_view","distinct_id": "user123","properties": {"url": "/home"}})
```

**List persons:**
```
http(method="GET", url="https://us.posthog.com/api/projects/{project_id}/persons/?limit=10")
```

**List events:**
```
http(method="GET", url="https://us.posthog.com/api/projects/{project_id}/events/?limit=10")
```

**Get insights:**
```
http(method="GET", url="https://us.posthog.com/api/projects/{project_id}/insights/?limit=10")
```

## Notes

- Capture endpoint uses project API key in body.
- Management endpoints use personal API key in `Authorization` header.
- EU region: `eu.posthog.com` instead of `us.posthog.com`.
- Events have `distinct_id` (user identifier) and `properties`.
