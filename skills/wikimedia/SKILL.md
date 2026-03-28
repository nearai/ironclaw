---
name: wikimedia
version: "1.0.0"
description: Wikimedia API — Wikimedia is a global nonprofit organization that hosts and supports free knowle
activation:
  keywords:
    - "wikimedia"
    - "productivity"
  patterns:
    - "(?i)wikimedia"
  tags:
    - "productivity"
    - "collaboration"
    - "news"
    - "productiv"
  max_context_tokens: 1200
---

# Wikimedia API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Wikimedia is a global nonprofit organization that hosts and supports free knowledge platforms like Wikipedia, offering collaborative, open-content resources maintained by a worldwide community of volu

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
