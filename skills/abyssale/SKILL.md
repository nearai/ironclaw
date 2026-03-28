---
name: abyssale
version: "1.0.0"
description: Abyssale API — A cloud-based creative automation solution that helps teams design, generate
activation:
  keywords:
    - "abyssale"
    - "creative automation platform"
  patterns:
    - "(?i)abyssale"
  tags:
    - "tools"
    - "creative-automation-platform"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ABYSSALE_API_KEY]
---

# Abyssale API

Use the `http` tool. API key is automatically injected via `x-api-key` header — **never construct auth headers manually**.

> A cloud-based creative automation solution that helps teams design, generate, and scale thousands of on-brand visual assets in minutes from a single template, accelerates production through APIs and i

## Authentication

This integration uses **API Key** authentication via the `x-api-key` header.
Format: `x-api-key: ...`

## Required Credentials

- `ABYSSALE_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
