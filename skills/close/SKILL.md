---
name: close
version: "1.0.0"
description: Close API — Close is a sales engagement CRM designed to help inside sales teams close more d
activation:
  keywords:
    - "close"
    - "crm"
  patterns:
    - "(?i)close"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
---

# Close API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.close.com/api/v1`

## Actions

**List leads:**
```
http(method="GET", url="https://api.close.com/api/v1/lead/?_limit=10")
```

**Get lead:**
```
http(method="GET", url="https://api.close.com/api/v1/lead/{lead_id}/")
```

**Create lead:**
```
http(method="POST", url="https://api.close.com/api/v1/lead/", body={"name": "Acme Corp","contacts": [{"name": "John Doe","emails": [{"email": "john@acme.com","type": "office"}]}]})
```

**Search leads:**
```
http(method="POST", url="https://api.close.com/api/v1/data/search/", body={"query": {"type": "and","queries": [{"type": "field_condition","field": {"field_name": "lead_name"},"condition": {"type": "text","mode": "full_words","value": "Acme"}}]},"results_limit": 10})
```

**List activities:**
```
http(method="GET", url="https://api.close.com/api/v1/activity/?_limit=10")
```

## Notes

- Uses API key as Basic auth username (no password).
- Lead IDs start with `lead_`.
- Activities: `EmailThread`, `Call`, `SMS`, `Note`, `Meeting`.
- Pagination: `_skip` and `_limit` params.
