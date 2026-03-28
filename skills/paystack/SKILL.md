---
name: paystack
version: "1.0.0"
description: Paystack API — Paystack is a payments platform that enables businesses to accept online and off
activation:
  keywords:
    - "paystack"
    - "payment gateway"
  patterns:
    - "(?i)paystack"
  tags:
    - "tools"
    - "payment-gateway"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [PAYSTACK_API_TOKEN]
---

# Paystack API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.paystack.co`

## Actions

**Initialize transaction:**
```
http(method="POST", url="https://api.paystack.co/transaction/initialize", body={"email": "customer@example.com","amount": 500000})
```

**Verify transaction:**
```
http(method="GET", url="https://api.paystack.co/transaction/verify/{reference}")
```

**List transactions:**
```
http(method="GET", url="https://api.paystack.co/transaction?perPage=10")
```

**List customers:**
```
http(method="GET", url="https://api.paystack.co/customer?perPage=10")
```

**Create plan:**
```
http(method="POST", url="https://api.paystack.co/plan", body={"name": "Monthly Basic","interval": "monthly","amount": 500000})
```

## Notes

- Amounts are in the smallest currency unit (kobo for NGN): ₦5,000 = `500000`.
- Transaction reference is unique per transaction.
- Plan intervals: `hourly`, `daily`, `weekly`, `monthly`, `biannually`, `annually`.
- Currencies: `NGN`, `GHS`, `ZAR`, `USD`.
