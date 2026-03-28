---
name: ayrshare
version: "1.0.0"
description: Ayrshare API — Ayrshare provides a unified REST-based social media API that lets developers pro
activation:
  keywords:
    - "ayrshare"
    - "social media"
  patterns:
    - "(?i)ayrshare"
  tags:
    - "social-media"
    - "social"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [AYRSHARE_API_KEY]
---

# Ayrshare API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

> Ayrshare provides a unified REST-based social media API that lets developers programmatically post, schedule, delete, and analyze content across 13 major networks, manage comments and messages, and au

## Authentication

This integration uses **API Key** authentication via the `Authorization` header.
Format: `Authorization: Bearer ...`

## Required Credentials

- `AYRSHARE_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
