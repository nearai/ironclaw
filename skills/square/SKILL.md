---
name: square
version: "1.0.0"
description: Square API — Square offers payment processing and business tools for merchants
activation:
  keywords:
    - "square"
    - "payments"
  patterns:
    - "(?i)square"
  tags:
    - "payments"
    - "billing"
    - "finance"
  max_context_tokens: 1200
---

# Square API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://connect.squareup.com/v2`

## Actions

**List payments:**
```
http(method="GET", url="https://connect.squareup.com/v2/payments?limit=10")
```

**Create payment:**
```
http(method="POST", url="https://connect.squareup.com/v2/payments", body={"source_id": "cnon:card-nonce-ok","idempotency_key": "unique-key","amount_money": {"amount": 1000,"currency": "USD"},"location_id": "loc_id"})
```

**List customers:**
```
http(method="GET", url="https://connect.squareup.com/v2/customers?limit=10")
```

**Create customer:**
```
http(method="POST", url="https://connect.squareup.com/v2/customers", body={"given_name": "John","family_name": "Doe","email_address": "john@example.com"})
```

**List catalog items:**
```
http(method="GET", url="https://connect.squareup.com/v2/catalog/list?types=ITEM&limit=10")
```

## Notes

- Amounts in smallest currency unit: $10.00 = `1000`.
- `idempotency_key` required for create operations (use UUIDs).
- Location ID required for most transaction operations.
- Sandbox base URL: `https://connect.squareupsandbox.com/v2`.
