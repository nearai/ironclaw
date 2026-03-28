---
name: klaviyo
version: "1.0.0"
description: Klaviyo API — Klaviyo is a marketing automation platform built for eCommerce
activation:
  keywords:
    - "klaviyo"
    - "ecommerce"
  patterns:
    - "(?i)klaviyo"
  tags:
    - "tools"
    - "Ecommerce"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [KLAVIYO_API_KEY]
---

# Klaviyo API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

## Base URL

`https://a.klaviyo.com/api`

**Required headers**: `revision: 2024-10-15`

## Actions

**List profiles:**
```
http(method="GET", url="https://a.klaviyo.com/api/profiles/?page[size]=10")
```

**Get profile:**
```
http(method="GET", url="https://a.klaviyo.com/api/profiles/{profile_id}")
```

**Create profile:**
```
http(method="POST", url="https://a.klaviyo.com/api/profiles/", headers=[{"name": "revision","value": "2024-10-15"}], body={"data": {"type": "profile","attributes": {"email": "john@example.com","first_name": "John","last_name": "Doe"}}})
```

**List lists:**
```
http(method="GET", url="https://a.klaviyo.com/api/lists/?page[size]=10")
```

**Create event:**
```
http(method="POST", url="https://a.klaviyo.com/api/events/", headers=[{"name": "revision","value": "2024-10-15"}], body={"data": {"type": "event","attributes": {"profile": {"data": {"type": "profile","attributes": {"email": "john@example.com"}}},"metric": {"data": {"type": "metric","attributes": {"name": "Placed Order"}}},"properties": {"value": 99.99}}}})
```

## Notes

- Auth header: `Authorization: Klaviyo-API-Key {key}` (auto-injected).
- Requires `revision` header with API version date.
- Uses JSON:API format: `{data: {type, id, attributes}}`.
- Pagination: cursor-based via `page[cursor]`.
