---
name: q2
version: "1.0.0"
description: Q2 API — Q2 offers an integrated digital banking platform enabling financial institutions
activation:
  keywords:
    - "q2"
    - "finance"
  patterns:
    - "(?i)q2"
  tags:
    - "tools"
    - "finance"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [Q2_CLIENT_ID, Q2_CLIENT_SECRET, Q2_SCOPES, Q2_ENVIRONMENT]
---

# Q2 API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Q2 offers an integrated digital banking platform enabling financial institutions to manage consumer, small-business, and commercial banking operations, including onboarding, payments, risk/fraud manag

## Authentication

This integration uses **OAuth 2.0**. The token is managed automatically — no manual auth setup required in API calls.

## Required Credentials

- `Q2_CLIENT_ID` — Client ID
- `Q2_CLIENT_SECRET` — Client Secret
- `Q2_SCOPES` — Scopes (space separated)
- `Q2_ENVIRONMENT` — Environment

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
