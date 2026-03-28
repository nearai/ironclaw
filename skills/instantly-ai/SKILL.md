---
name: instantly-ai
version: "1.0.0"
description: Instantly.ai API — Instantly is an AI-powered cold email platform that automates outreach with unli
activation:
  keywords:
    - "instantly-ai"
    - "instantly.ai"
    - "ai"
  patterns:
    - "(?i)instantly.?ai"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [INSTANTLY_AI_API_KEY]
---

# Instantly.ai API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Instantly is an AI-powered cold email platform that automates outreach with unlimited inboxes, built-in warm-up, personalization, and analytics—designed to help teams scale email campaigns efficiently

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `INSTANTLY_AI_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
