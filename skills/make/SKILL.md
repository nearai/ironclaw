---
name: make
version: "1.0.0"
description: Make API — Make is a no-code automation platform that allows users to visually build workfl
activation:
  keywords:
    - "make"
    - "ai"
  patterns:
    - "(?i)make"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [MAKE_ZONE_URL, MAKE_API_TOKEN]
---

# Make API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

## Base URL

`https://{MAKE_ZONE}.make.com/api/v2`

## Actions

**List scenarios:**
```
http(method="GET", url="https://{MAKE_ZONE}.make.com/api/v2/scenarios?pg[limit]=10")
```

**Get scenario:**
```
http(method="GET", url="https://{MAKE_ZONE}.make.com/api/v2/scenarios/{scenario_id}")
```

**Run scenario:**
```
http(method="POST", url="https://{MAKE_ZONE}.make.com/api/v2/scenarios/{scenario_id}/run")
```

**List organizations:**
```
http(method="GET", url="https://{MAKE_ZONE}.make.com/api/v2/organizations?pg[limit]=10")
```

## Notes

- Zone: `us1`, `eu1`, `eu2` depending on your account region.
- Scenarios are automations/workflows.
- Pagination: `pg[offset]` and `pg[limit]`.
- Scenario states: `active`, `inactive`.
