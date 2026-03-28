---
name: auth0-management
version: "1.0.0"
description: Auth0 Management API — Auth0 delivers a flexible, drop-in authentication and authorization platform tha
activation:
  keywords:
    - "auth0-management"
    - "auth0 management"
    - "authentication"
  patterns:
    - "(?i)auth0.?management"
  tags:
    - "tools"
    - "authentication"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [AUTH0_API_TOKEN, AUTH0_MANAGEMENT_TENANT_DOMAIN]
---

# Auth0 Management API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://{AUTH0_DOMAIN}/api/v2`

## Actions

**List users:**
```
http(method="GET", url="https://{AUTH0_DOMAIN}/api/v2/users?page=0&per_page=10")
```

**Get user:**
```
http(method="GET", url="https://{AUTH0_DOMAIN}/api/v2/users/{user_id}")
```

**Create user:**
```
http(method="POST", url="https://{AUTH0_DOMAIN}/api/v2/users", body={"email": "john@example.com","password": "SecureP@ss1","connection": "Username-Password-Authentication"})
```

**Update user:**
```
http(method="PATCH", url="https://{AUTH0_DOMAIN}/api/v2/users/{user_id}", body={"name": "John Doe"})
```

**List roles:**
```
http(method="GET", url="https://{AUTH0_DOMAIN}/api/v2/roles")
```

## Notes

- `user_id` is URL-encoded: `auth0|abc123` → `auth0%7Cabc123`.
- Connections: `Username-Password-Authentication`, `google-oauth2`, etc.
- Use `q` param for Lucene query syntax search: `?q=email:"*@acme.com"`.
