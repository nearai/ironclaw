---
name: x
version: "1.0.0"
description: X API — X is a real-time social networking platform for sharing short-form content
activation:
  keywords:
    - "x"
    - "tools"
  patterns:
    - "(?i)x"
  tags:
    - "tools"
    - "utility"
    - "tool"
  max_context_tokens: 1200
---

# X API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.x.com/2`

## Actions

**Search tweets:**
```
http(method="GET", url="https://api.x.com/2/tweets/search/recent?query=from:username&max_results=10&tweet.fields=created_at,public_metrics")
```

**Get user by username:**
```
http(method="GET", url="https://api.x.com/2/users/by/username/{username}?user.fields=public_metrics,description")
```

**Post tweet:**
```
http(method="POST", url="https://api.x.com/2/tweets", body={"text": "Hello X!"})
```

**Get user tweets:**
```
http(method="GET", url="https://api.x.com/2/users/{user_id}/tweets?max_results=10&tweet.fields=created_at,public_metrics")
```

## Notes

- Uses OAuth 2.0 — credentials are auto-injected.
- Use `tweet.fields`, `user.fields`, `expansions` to request specific data.
- Search query operators: `from:`, `to:`, `has:media`, `is:retweet`, `-is:reply`.
- Rate limits vary by endpoint and access level.
