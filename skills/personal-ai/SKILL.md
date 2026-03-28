---
name: personal-ai
version: "1.0.0"
description: Personal AI API — Personal AI allows users to create a digital memory by recording their thoughts
activation:
  keywords:
    - "personal-ai"
    - "personal ai"
    - "ai"
  patterns:
    - "(?i)personal.?ai"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [PERSONAL_AI_API_KEY, PERSONAL_AI_DOMAIN_NAME]
---

# Personal AI API

Use the `http` tool. API key is automatically injected via `x-api-key` header — **never construct auth headers manually**.

> Personal AI allows users to create a digital memory by recording their thoughts, interactions, and decisions, enabling a more personalized and context-aware assistant experience.

## Authentication

This integration uses **API Key** authentication via the `x-api-key` header.
Format: `x-api-key: ...`

## Required Credentials

- `PERSONAL_AI_API_KEY` — API Key
- `PERSONAL_AI_DOMAIN_NAME` — Domain Name

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
