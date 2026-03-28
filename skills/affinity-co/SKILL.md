---
name: affinity-co
version: "1.0.0"
description: Affinity.co API — Affinity is a relationship intelligence CRM that automatically captures and anal
activation:
  keywords:
    - "affinity-co"
    - "affinity.co"
    - "crm"
  patterns:
    - "(?i)affinity.?co"
  tags:
    - "crm"
    - "sales"
    - "contacts"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [AFFINITY_CO_API_KEY]
---

# Affinity.co API

Use the `http` tool. Credentials are automatically injected.

> Affinity is a relationship intelligence CRM that automatically captures and analyzes your team's communication data to surface valuable connections, streamline deal management, and help you close more

## Authentication


## Required Credentials

- `AFFINITY_CO_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
