---
name: agiled
version: "1.0.0"
description: Agiled API — Agiled is an all-in-one business management platform that helps freelancers and 
activation:
  keywords:
    - "agiled"
    - "crm"
  patterns:
    - "(?i)agiled"
  tags:
    - "crm"
    - "sales"
    - "contacts"
  max_context_tokens: 1200
---

# Agiled API

Use the `http` tool. API key is automatically injected as `api_token` query parameter.

> Agiled is an all-in-one business management platform that helps freelancers and small businesses manage CRM, projects, finances, contracts, invoicing and client portals within a unified workspace.

## Authentication

This integration uses **query parameter** authentication via `api_token`.

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
