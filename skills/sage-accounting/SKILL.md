---
name: sage-accounting
version: "1.0.0"
description: Sage Accounting API — Sage Accounting is cloud-based software for managing invoices, expenses
activation:
  keywords:
    - "sage-accounting"
    - "sage accounting"
    - "accounting"
  patterns:
    - "(?i)sage.?accounting"
  tags:
    - "accounting"
    - "finance"
    - "Accounting"
    - "ai"
  max_context_tokens: 1200
---

# Sage Accounting API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.accounting.sage.com/v3.1`

## Actions

**List contacts:**
```
http(method="GET", url="https://api.accounting.sage.com/v3.1/contacts?items_per_page=10")
```

**List invoices:**
```
http(method="GET", url="https://api.accounting.sage.com/v3.1/sales_invoices?items_per_page=10")
```

**Create invoice:**
```
http(method="POST", url="https://api.accounting.sage.com/v3.1/sales_invoices", body={"contact_id": "contact_id","date": "2026-03-27","invoice_lines": [{"description": "Consulting","quantity": 10,"unit_price": 150.0,"ledger_account_id": "ledger_id"}]})
```

## Notes

- Uses OAuth 2.0 — credentials are auto-injected.
- Resources: contacts, sales_invoices, purchase_invoices, ledger_accounts, bank_accounts.
- Pagination: `items_per_page` and `page` params.
- Dates in `YYYY-MM-DD` format.
