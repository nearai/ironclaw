---
name: box-hero
version: "1.0.0"
description: BoxHero API — A cloud-based inventory and stock management platform that enables businesses to
activation:
  keywords:
    - "box-hero"
    - "boxhero"
    - "inventory management"
  patterns:
    - "(?i)box.?hero"
  tags:
    - "tools"
    - "inventory-management"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BOX_HERO_API_TOKEN]
---

# BoxHero API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> A cloud-based inventory and stock management platform that enables businesses to track products, manage warehouses, monitor stock levels in real time, and automate order fulfillment across sales chann

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `BOX_HERO_API_TOKEN` — API Token

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
