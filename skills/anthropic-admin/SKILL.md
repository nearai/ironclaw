---
name: anthropic-admin
version: "1.0.0"
description: Anthropic Admin API — Anthropic Admin provides administrative tools for managing access, API keys
activation:
  keywords:
    - "anthropic-admin"
    - "anthropic admin"
    - "ai"
  patterns:
    - "(?i)anthropic.?admin"
  tags:
    - "ai"
    - "machine-learning"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ANTHROPIC_ADMIN_API_KEY]
---

# Anthropic Admin API

Use the `http` tool. API key is automatically injected via `X-Api-Key` header — **never construct auth headers manually**.

> Anthropic Admin provides administrative tools for managing access, API keys, usage, billing, and organizational settings for applications built with Claude AI models, allowing teams to control permiss

## Authentication

This integration uses **API Key** authentication via the `X-Api-Key` header.
Format: `X-Api-Key: ...`

## Required Credentials

- `ANTHROPIC_ADMIN_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
