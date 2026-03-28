---
name: convex-deployment
version: "1.0.0"
description: Convex Deployment API — Convex Deployment enables developers to deploy backend functions
activation:
  keywords:
    - "convex-deployment"
    - "convex deployment"
    - "tools"
  patterns:
    - "(?i)convex.?deployment"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CONVEX_DEPLOYMENT_DEPLOY_KEY, CONVEX_DEPLOYMENT_BASE_URL]
---

# Convex Deployment API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

> Convex Deployment enables developers to deploy backend functions, database schema, indexes, and configuration for applications built on the Convex platform. It supports development, preview, and produ

## Authentication

This integration uses **API Key** authentication via the `Authorization` header.
Format: `Authorization: Convex ...`

## Required Credentials

- `CONVEX_DEPLOYMENT_DEPLOY_KEY` — API Key
- `CONVEX_DEPLOYMENT_BASE_URL` — Deployment URL

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
