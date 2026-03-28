---
name: meta
version: "1.0.0"
description: Meta API — Meta builds technologies that help people connect, find communities
activation:
  keywords:
    - "meta"
    - "marketing"
  patterns:
    - "(?i)meta"
  tags:
    - "marketing"
    - "email"
    - "campaigns"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [META_ACCESS_TOKEN]
---

# Meta API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://graph.facebook.com/v21.0`

## Actions

**Get current user:**
```
http(method="GET", url="https://graph.facebook.com/v21.0/me?fields=id,name,email")
```

**Get page:**
```
http(method="GET", url="https://graph.facebook.com/v21.0/{page_id}?fields=name,fan_count,about")
```

**Post to page:**
```
http(method="POST", url="https://graph.facebook.com/v21.0/{page_id}/feed", body={"message": "Hello from API!"})
```

**Get Instagram business account:**
```
http(method="GET", url="https://graph.facebook.com/v21.0/{page_id}?fields=instagram_business_account")
```

## Notes

- Uses OAuth 2.0 — credentials are auto-injected.
- Always specify `fields` parameter to select returned data.
- Page access token needed for page operations.
- Rate limit: 200 calls/user/hour for most endpoints.
