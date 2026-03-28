---
name: freshbooks
version: "1.0.0"
description: FreshBooks API — FreshBooks is an accounting software designed for small businesses and freelance
activation:
  keywords:
    - "freshbooks"
    - "accounting"
  patterns:
    - "(?i)freshbooks"
  tags:
    - "accounting"
    - "finance"
  max_context_tokens: 1200
---

# FreshBooks API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.freshbooks.com`

## Actions

**Get current user:**
```
http(method="GET", url="https://api.freshbooks.com/auth/api/v1/users/me")
```

**List clients:**
```
http(method="GET", url="https://api.freshbooks.com/accounting/account/{account_id}/users/clients?per_page=10")
```

**Create invoice:**
```
http(method="POST", url="https://api.freshbooks.com/accounting/account/{account_id}/invoices/invoices", body={"invoice": {"customerid": 123,"create_date": "2026-03-27","lines": [{"name": "Consulting","qty": 10,"unit_cost": {"amount": "150.00","code": "USD"}}]}})
```

**List invoices:**
```
http(method="GET", url="https://api.freshbooks.com/accounting/account/{account_id}/invoices/invoices?per_page=10")
```

## Notes

- Uses OAuth 2.0 — credentials are auto-injected.
- Account ID from `/auth/api/v1/users/me` response's `business_memberships`.
- Money fields use `{amount, code}` objects.
- Invoice statuses: `draft`, `sent`, `viewed`, `paid`, `overdue`.
