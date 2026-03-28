---
name: chargebee
version: "1.0.0"
description: Chargebee API — Chargebee is a subscription billing and revenue operations platform that helps S
activation:
  keywords:
    - "chargebee"
    - "crm"
  patterns:
    - "(?i)chargebee"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CHARGEBEE_SUBDOMAIN, CHARGEBEE_API_KEY]
---

# Chargebee API

Use the `http` tool. Credentials are automatically injected.

## Base URL

`https://{CHARGEBEE_SITE}.chargebee.com/api/v2`

**Content-Type**: `application/x-www-form-urlencoded` for POST/PUT requests.

## Actions

**List subscriptions:**
```
http(method="GET", url="https://{CHARGEBEE_SITE}.chargebee.com/api/v2/subscriptions?limit=10")
```

**Get subscription:**
```
http(method="GET", url="https://{CHARGEBEE_SITE}.chargebee.com/api/v2/subscriptions/{subscription_id}")
```

**Create subscription:**
```
http(method="POST", url="https://{CHARGEBEE_SITE}.chargebee.com/api/v2/subscriptions", headers=[{"name": "Content-Type", "value": "application/x-www-form-urlencoded"}], body="customer[email]=john@example.com&plan_id=basic-monthly")
```

**List customers:**
```
http(method="GET", url="https://{CHARGEBEE_SITE}.chargebee.com/api/v2/customers?limit=10")
```

**List invoices:**
```
http(method="GET", url="https://{CHARGEBEE_SITE}.chargebee.com/api/v2/invoices?limit=10&sort_by[asc]=date")
```

## Notes

- Uses Basic auth with API key as username and empty password.
- POST bodies are form-encoded with bracket notation: `customer[email]=...`.
- Subscription states: `future`, `in_trial`, `active`, `non_renewing`, `paused`, `cancelled`.
- Pagination: `offset` key in response; pass as `?offset=...` for next page.
