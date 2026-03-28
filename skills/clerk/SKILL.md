---
name: clerk
version: "1.0.0"
description: Clerk API — Clerk provides user authentication and user management solutions for modern web 
activation:
  keywords:
    - "clerk"
    - "crm"
  patterns:
    - "(?i)clerk"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CLERK_API_KEY]
---

# Clerk API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.clerk.com/v1`

## Actions

**List users:**
```
http(method="GET", url="https://api.clerk.com/v1/users?limit=10&order_by=-created_at")
```

**Get user:**
```
http(method="GET", url="https://api.clerk.com/v1/users/{user_id}")
```

**Create user:**
```
http(method="POST", url="https://api.clerk.com/v1/users", body={"email_address": ["john@example.com"],"first_name": "John","last_name": "Doe","password": "SecureP@ss1"})
```

**Update user:**
```
http(method="PATCH", url="https://api.clerk.com/v1/users/{user_id}", body={"first_name": "Jane"})
```

**Delete user:**
```
http(method="DELETE", url="https://api.clerk.com/v1/users/{user_id}")
```

**List organizations:**
```
http(method="GET", url="https://api.clerk.com/v1/organizations?limit=10")
```

## Notes

- User IDs start with `user_`.
- Email addresses are arrays — users can have multiple.
- Use `?query=` for fuzzy search across name/email.
- Pagination: `limit` + `offset`.
