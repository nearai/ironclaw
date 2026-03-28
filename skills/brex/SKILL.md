---
name: brex
version: "1.0.0"
description: Brex API — Brex combines corporate cards, business accounts, expense management, bill pay
activation:
  keywords:
    - "brex"
    - "finance"
  patterns:
    - "(?i)brex"
  tags:
    - "tools"
    - "finance"
  max_context_tokens: 1200
---

# Brex API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Brex combines corporate cards, business accounts, expense management, bill pay, and travel booking into a single AI-powered financial operations platform—offering unified spend control, real-time visi

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
