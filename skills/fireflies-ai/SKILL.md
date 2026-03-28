---
name: fireflies-ai
version: "1.0.0"
description: Fireflies.ai API — Fireflies.ai is an AI meeting assistant that transcribes, summarizes
activation:
  keywords:
    - "fireflies-ai"
    - "fireflies.ai"
    - "ai"
  patterns:
    - "(?i)fireflies.?ai"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [FIREFLIESAI_API_KEY]
---

# Fireflies.ai API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Fireflies.ai is an AI meeting assistant that transcribes, summarizes, and analyzes conversations across video conferencing platforms to improve productivity and collaboration.

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `FIREFLIESAI_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
