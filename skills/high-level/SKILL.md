---
name: high-level
version: "1.0.0"
description: HighLevel API — HighLevel is an all-in-one sales and marketing automation platform that helps ag
activation:
  keywords:
    - "high-level"
    - "highlevel"
    - "crm"
  patterns:
    - "(?i)high.?level"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
---

# HighLevel API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> HighLevel is an all-in-one sales and marketing automation platform that helps agencies and businesses manage CRM, funnels, messaging, bookings, and campaigns from a single system, with extensible inte

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
