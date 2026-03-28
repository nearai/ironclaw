---
name: uber-eats
version: "1.0.0"
description: Uber Eats API — Uber Eats is a global online food ordering and delivery platform that lets consu
activation:
  keywords:
    - "uber-eats"
    - "uber eats"
    - "food delivery"
  patterns:
    - "(?i)uber.?eats"
  tags:
    - "tools"
    - "food-delivery"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [UBER_EATS_CLIENT_ID, UBER_EATS_CLIENT_SECRET]
---

# Uber Eats API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Uber Eats is a global online food ordering and delivery platform that lets consumers browse local restaurant menus, place orders for delivery or pickup, and track fulfillment in real time while connec

## Authentication

This integration uses **OAuth 2.0**. The token is managed automatically — no manual auth setup required in API calls.

## Required Credentials

- `UBER_EATS_CLIENT_ID` — Client ID
- `UBER_EATS_CLIENT_SECRET` — Client Secret

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
