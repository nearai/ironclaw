---
name: xero
version: "1.0.0"
description: Xero API — Xero is an online accounting software platform designed for small businesses
activation:
  keywords:
    - "xero"
    - "accounting"
  patterns:
    - "(?i)xero"
  tags:
    - "accounting"
    - "finance"
    - "Accounting"
    - "ai"
  max_context_tokens: 1200
---

# Xero API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.xero.com/api.xro/2.0`

## Actions

**List contacts:**
```
http(method="GET", url="https://api.xero.com/api.xro/2.0/Contacts?page=1")
```

**Get contact:**
```
http(method="GET", url="https://api.xero.com/api.xro/2.0/Contacts/{contact_id}")
```

**Create invoice:**
```
http(method="POST", url="https://api.xero.com/api.xro/2.0/Invoices", body={"Type": "ACCREC","Contact": {"ContactID": "contact_id"},"LineItems": [{"Description": "Consulting","Quantity": 10,"UnitAmount": 150.0,"AccountCode": "200"}]})
```

**List invoices:**
```
http(method="GET", url="https://api.xero.com/api.xro/2.0/Invoices?page=1&Statuses=AUTHORISED")
```

**List accounts:**
```
http(method="GET", url="https://api.xero.com/api.xro/2.0/Accounts")
```

## Notes

- Uses OAuth 2.0 — credentials are auto-injected.
- Requires `Xero-tenant-id` header for multi-org access.
- Invoice types: `ACCREC` (receivable/sales), `ACCPAY` (payable/bills).
- Invoice statuses: `DRAFT`, `SUBMITTED`, `AUTHORISED`, `PAID`, `VOIDED`.
