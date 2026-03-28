---
name: scrapingdog
version: "1.0.0"
description: Scrapingdog API — Scrapingdog is an all‑in‑one web scraping API that handles rotating proxies
activation:
  keywords:
    - "scrapingdog"
    - "tools"
  patterns:
    - "(?i)scrapingdog"
  tags:
    - "tools"
    - "utility"
    - "tool"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SCRAPINGDOG_API_KEY]
---

# Scrapingdog API

Use the `http` tool. API key is automatically injected as `api_key` query parameter.

> Scrapingdog is an all‑in‑one web scraping API that handles rotating proxies, headless browser rendering, CAPTCHA solving, and offers dedicated endpoints (e.g., Google SERP, Amazon, LinkedIn), deliveri

## Authentication

This integration uses **query parameter** authentication via `api_key`.

## Required Credentials

- `SCRAPINGDOG_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
