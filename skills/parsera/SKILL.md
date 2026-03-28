---
name: parsera
version: "1.0.0"
description: Parsera API — Parsera is an AI-powered web-data-extraction platform that allows users to input
activation:
  keywords:
    - "parsera"
    - "scraper"
  patterns:
    - "(?i)parsera"
  tags:
    - "tools"
    - "scraper"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [PARSERA_API_KEY]
---

# Parsera API

Use the `http` tool. API key is automatically injected via `X-API-KEY` header — **never construct auth headers manually**.

> Parsera is an AI-powered web-data-extraction platform that allows users to input a URL and natural-language instructions to extract structured data or generate reusable scraping scripts — enabling dev

## Authentication

This integration uses **API Key** authentication via the `X-API-KEY` header.
Format: `X-API-KEY: ...`

## Required Credentials

- `PARSERA_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
