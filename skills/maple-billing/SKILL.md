---
name: maple-billing
version: "1.0.0"
description: Maple Billing API — Maple Billing is an all-in-one revenue management platform that empowers SaaS co
activation:
  keywords:
    - "maple-billing"
    - "maple billing"
    - "payments"
  patterns:
    - "(?i)maple.?billing"
  tags:
    - "payments"
    - "billing"
    - "finance"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [MAPLE_BILLING_API_KEY, MAPLE_BILLING_COMPANY_ID]
---

# Maple Billing API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Maple Billing is an all-in-one revenue management platform that empowers SaaS companies to streamline billing, invoicing, and contract workflows across usage-based, seat-based, and hybrid pricing mode

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `MAPLE_BILLING_API_KEY` — API Key
- `MAPLE_BILLING_COMPANY_ID` — Company ID

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
