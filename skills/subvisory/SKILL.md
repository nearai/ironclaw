---
name: subvisory
version: "1.0.0"
description: Subvisory API — A subscription tracking and optimization platform that helps individuals and tea
activation:
  keywords:
    - "subvisory"
    - "subscription management"
  patterns:
    - "(?i)subvisory"
  tags:
    - "tools"
    - "subscription-management"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SUBVISORY_API_KEY]
---

# Subvisory API

Use the `http` tool. API key is automatically injected via `X-API-Key` header — **never construct auth headers manually**.

> A subscription tracking and optimization platform that helps individuals and teams monitor recurring payments, forecast spending, receive billing and trial reminders, and manage subscription details v

## Authentication

This integration uses **API Key** authentication via the `X-API-Key` header.
Format: `X-API-Key: ...`

## Required Credentials

- `SUBVISORY_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
