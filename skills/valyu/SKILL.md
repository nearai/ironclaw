---
name: valyu
version: "1.0.0"
description: Valyu API — Valyu is a multimodal retrieval API built to enrich your AI’s context with reran
activation:
  keywords:
    - "valyu"
    - "ai"
  patterns:
    - "(?i)valyu"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
    - "communication"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [VALYU_API_KEY]
---

# Valyu API

Use the `http` tool. API key is automatically injected via `x-api-key` header — **never construct auth headers manually**.

> Valyu is a multimodal retrieval API built to enrich your AI’s context with reranked knowledge from scholarly literature, real-time news, market feeds, and fresh web data.

## Authentication

This integration uses **API Key** authentication via the `x-api-key` header.
Format: `x-api-key: ...`

## Required Credentials

- `VALYU_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
