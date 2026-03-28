---
name: ship-bob
version: "1.0.0"
description: ShipBob API — ShipBob is a comprehensive e-commerce fulfillment and logistics platform that ma
activation:
  keywords:
    - "ship-bob"
    - "shipbob"
    - "logistics"
  patterns:
    - "(?i)ship.?bob"
  tags:
    - "tools"
    - "logistics"
  max_context_tokens: 1200
---

# ShipBob API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> ShipBob is a comprehensive e-commerce fulfillment and logistics platform that manages order processing, inventory distribution across 60+ global warehouses, and two-day shipping with built-in analytic

## Authentication

This integration uses **OAuth 2.0**. The token is managed automatically — no manual auth setup required in API calls.

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
