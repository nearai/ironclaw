---
name: ordinal
version: "1.0.0"
description: Ordinal API — Ordinal is a social media management platform that enables teams to draft, plan
activation:
  keywords:
    - "ordinal"
    - "social media management"
  patterns:
    - "(?i)ordinal"
  tags:
    - "tools"
    - "social-media-management"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ORDINAL_API_KEY]
---

# Ordinal API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

> Ordinal is a social media management platform that enables teams to draft, plan, schedule, publish, automate engagement (likes, comments, reposts), and track analytics across major networks from a uni

## Authentication

This integration uses **API Key** authentication via the `Authorization` header.
Format: `Authorization: Bearer ...`

## Required Credentials

- `ORDINAL_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
