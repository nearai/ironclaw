---
name: quickbooks
version: "1.0.0"
description: QuickBooks Online API — invoices, customers, payments, accounts, reports
activation:
  keywords:
    - "quickbooks"
    - "quickbooks invoice"
    - "qbo"
  exclude_keywords:
    - "xero"
    - "freshbooks"
  patterns:
    - "(?i)quickbooks.*(invoice|customer|payment|account)"
    - "(?i)(create|list).*invoice.*quickbooks"
  tags:
    - "accounting"
    - "finance"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [QUICKBOOKS_ACCESS_TOKEN, QUICKBOOKS_REALM_ID]
---

# QuickBooks Online API

Use the `http` tool. Credentials are automatically injected for `quickbooks.api.intuit.com`.

## Base URL

`https://quickbooks.api.intuit.com/v3/company/{QUICKBOOKS_REALM_ID}`

## Actions

**Query customers:**
```
http(method="GET", url="https://quickbooks.api.intuit.com/v3/company/{realm}/query?query=SELECT+*+FROM+Customer+WHERE+Active+%3D+true+MAXRESULTS+20", headers=[{"name": "Accept", "value": "application/json"}])
```

**Create invoice:**
```
http(method="POST", url="https://quickbooks.api.intuit.com/v3/company/{realm}/invoice", headers=[{"name": "Accept", "value": "application/json"}], body={"CustomerRef": {"value": "<customer_id>"}, "Line": [{"Amount": 150.00, "DetailType": "SalesItemLineDetail", "SalesItemLineDetail": {"ItemRef": {"value": "<item_id>"}, "Qty": 1, "UnitPrice": 150.00}}], "DueDate": "2026-04-30"})
```

**Get invoice:**
```
http(method="GET", url="https://quickbooks.api.intuit.com/v3/company/{realm}/invoice/<invoice_id>", headers=[{"name": "Accept", "value": "application/json"}])
```

**Send invoice (email):**
```
http(method="POST", url="https://quickbooks.api.intuit.com/v3/company/{realm}/invoice/<invoice_id>/send")
```

**Create payment:**
```
http(method="POST", url="https://quickbooks.api.intuit.com/v3/company/{realm}/payment", headers=[{"name": "Accept", "value": "application/json"}], body={"CustomerRef": {"value": "<customer_id>"}, "TotalAmt": 150.00, "Line": [{"Amount": 150.00, "LinkedTxn": [{"TxnId": "<invoice_id>", "TxnType": "Invoice"}]}]})
```

**Profit & Loss report:**
```
http(method="GET", url="https://quickbooks.api.intuit.com/v3/company/{realm}/reports/ProfitAndLoss?start_date=2026-01-01&end_date=2026-03-31", headers=[{"name": "Accept", "value": "application/json"}])
```

**Query items (products/services):**
```
http(method="GET", url="https://quickbooks.api.intuit.com/v3/company/{realm}/query?query=SELECT+*+FROM+Item+WHERE+Active+%3D+true+MAXRESULTS+20", headers=[{"name": "Accept", "value": "application/json"}])
```

## Notes

- QuickBooks uses a SQL-like query language for reads.
- `CustomerRef`, `ItemRef` etc. use `{"value": "<id>"}` wrapper objects.
- Always include `Accept: application/json` header.
- Amounts are decimal numbers (not cents).
- Update requires `SyncToken` from the GET response (optimistic concurrency).
- Sandbox: use `sandbox-quickbooks.api.intuit.com` for testing.
