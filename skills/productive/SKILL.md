---
name: productive
version: "1.0.0"
description: Productive API — Productive is an all-in-one agency management platform that streamlines project 
activation:
  keywords:
    - "productive"
    - "productivity"
  patterns:
    - "(?i)productive"
  tags:
    - "productivity"
    - "collaboration"
    - "news"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [PRODUCTIVE_API_KEY]
---

# Productive API

Use the `http` tool. API key is automatically injected via `X-Auth-Token` header — **never construct auth headers manually**.

> Productive is an all-in-one agency management platform that streamlines project delivery, resource planning, time tracking, budgeting, and profitability reporting—empowering agencies and consultancies

## Authentication

This integration uses **API Key** authentication via the `X-Auth-Token` header.
Format: `X-Auth-Token: ...`

## Required Credentials

- `PRODUCTIVE_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
