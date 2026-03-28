---
name: linked-in
version: "1.0.0"
description: LinkedIn API — LinkedIn is a professional networking platform where users can connect
activation:
  keywords:
    - "linked-in"
    - "linkedin"
    - "marketing"
  patterns:
    - "(?i)linked.?in"
  tags:
    - "marketing"
    - "email"
    - "campaigns"
  max_context_tokens: 1200
---

# LinkedIn API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.linkedin.com/v2`

## Actions

**Get current user:**
```
http(method="GET", url="https://api.linkedin.com/v2/userinfo")
```

**Create post:**
```
http(method="POST", url="https://api.linkedin.com/v2/ugcPosts", body={"author": "urn:li:person:{person_id}","lifecycleState": "PUBLISHED","specificContent": {"com.linkedin.ugc.ShareContent": {"shareCommentary": {"text": "Hello LinkedIn!"},"shareMediaCategory": "NONE"}},"visibility": {"com.linkedin.ugc.MemberNetworkVisibility": "PUBLIC"}})
```

**Get company:**
```
http(method="GET", url="https://api.linkedin.com/v2/organizations/{org_id}")
```

## Notes

- Uses OAuth 2.0 — credentials are auto-injected.
- URNs format: `urn:li:person:{id}`, `urn:li:organization:{id}`.
- Scopes needed: `r_liteprofile`, `w_member_social` for posting.
- Rate limit: 100 requests/day for most endpoints.
