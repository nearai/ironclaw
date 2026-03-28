---
name: kommo
version: "1.0.0"
description: Kommo API — Kommo is a sales-CRM platform that unifies messaging apps
activation:
  keywords:
    - "kommo"
    - "crm"
  patterns:
    - "(?i)kommo"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SUB_DOMAIN]
---

# Kommo API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Kommo is a sales-CRM platform that unifies messaging apps, live chat and pipeline management into one inbox, enabling teams to nurture leads, automate outreach and close deals more efficiently.

## Authentication

This integration uses **OAuth 2.0**. The token is managed automatically — no manual auth setup required in API calls.

## Required Credentials

- `SUB_DOMAIN` — Domain

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
