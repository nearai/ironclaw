---
name: nyne-ai
version: "1.0.0"
description: Nyne.ai API — Nyne.ai surfaces person- and business-level purchase-intent signals by monitorin
activation:
  keywords:
    - "nyne-ai"
    - "nyne.ai"
    - "ai"
  patterns:
    - "(?i)nyne.?ai"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [NYNE_AI_API_KEY, NYNE_AI_API_SECRET]
---

# Nyne.ai API

Use the `http` tool. API key is automatically injected via `X-API-Key` header — **never construct auth headers manually**.

> Nyne.ai surfaces person- and business-level purchase-intent signals by monitoring over 200 million sources for real-time buying events, life-changes and enriched contact data, delivered through APIs a

## Authentication

This integration uses **API Key** authentication via the `X-API-Key` header.
Format: `X-API-Key: ...`

## Required Credentials

- `NYNE_AI_API_KEY` — API Key
- `NYNE_AI_API_SECRET` — API Secret

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
