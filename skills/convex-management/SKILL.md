---
name: convex-management
version: "1.0.0"
description: Convex Management API — Convex Management provides a developer console for managing applications built o
activation:
  keywords:
    - "convex-management"
    - "convex management"
    - "baas"
  patterns:
    - "(?i)convex.?management"
  tags:
    - "tools"
    - "baas"
  max_context_tokens: 1200
---

# Convex Management API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Convex Management provides a developer console for managing applications built on the Convex backend platform, including database data, serverless functions, deployments, logs, and configuration, help

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
