---
name: beehiiv
version: "1.0.0"
description: Beehiiv API — Beehiiv is a newsletter platform designed for creators and publishers to grow
activation:
  keywords:
    - "beehiiv"
    - "tools"
  patterns:
    - "(?i)beehiiv"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BEEHIIV_API_KEY]
---

# Beehiiv API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.beehiiv.com/v2`

## Actions

**List publications:**
```
http(method="GET", url="https://api.beehiiv.com/v2/publications")
```

**List subscribers:**
```
http(method="GET", url="https://api.beehiiv.com/v2/publications/{pub_id}/subscriptions?limit=10")
```

**Create subscriber:**
```
http(method="POST", url="https://api.beehiiv.com/v2/publications/{pub_id}/subscriptions", body={"email": "reader@example.com","reactivate_existing": true})
```

**List posts:**
```
http(method="GET", url="https://api.beehiiv.com/v2/publications/{pub_id}/posts?status=confirmed&limit=10")
```

## Notes

- Publication IDs start with `pub_`.
- Post status: `draft`, `confirmed` (published), `archived`.
- Subscriber status: `active`, `inactive`, `validating`.
