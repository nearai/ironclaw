---
name: scrape-do
version: "1.0.0"
description: Scrape.do API — Scrape.do is a web-scraping API that bypasses anti-bot systems (like Cloudflare 
activation:
  keywords:
    - "scrape-do"
    - "scrape.do"
    - "scrapper"
  patterns:
    - "(?i)scrape.?do"
  tags:
    - "tools"
    - "scrapper"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SCRAPE_DO_TOKEN]
---

# Scrape.do API

Use the `http` tool. API key is automatically injected as `token` query parameter.

> Scrape.do is a web-scraping API that bypasses anti-bot systems (like Cloudflare or Akamai) by using rotating residential and mobile proxies, real browser fingerprints, headless browser rendering, and 

## Authentication

This integration uses **query parameter** authentication via `token`.

## Required Credentials

- `SCRAPE_DO_TOKEN` — Token

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
