---
name: deliveroo
version: "1.0.0"
description: Deliveroo API — Deliveroo operates a technology-driven online food delivery marketplace that con
activation:
  keywords:
    - "deliveroo"
    - "food delivery"
  patterns:
    - "(?i)deliveroo"
  tags:
    - "tools"
    - "food-delivery"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [DELIVEROO_CLIENT_ID, DELIVEROO_CLIENT_SECRET]
---

# Deliveroo API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Deliveroo operates a technology-driven online food delivery marketplace that connects consumers with local restaurants, grocery and retail partners through its app, enabling on-demand ordering and rea

## Authentication

This integration uses **OAuth 2.0**. The token is managed automatically — no manual auth setup required in API calls.

## Required Credentials

- `DELIVEROO_CLIENT_ID` — Client ID
- `DELIVEROO_CLIENT_SECRET` — Client Secret

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
