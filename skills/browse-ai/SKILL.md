---
name: browse-ai
version: "1.0.0"
description: Browse AI API — Browse AI is a no-code, AI-powered web scraping and monitoring platform that ena
activation:
  keywords:
    - "browse-ai"
    - "browse ai"
    - "scraper"
  patterns:
    - "(?i)browse.?ai"
  tags:
    - "tools"
    - "scraper"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BROWSE_AI_API_KEY]
---

# Browse AI API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Browse AI is a no-code, AI-powered web scraping and monitoring platform that enables users to extract structured data from any website, set automated alerts for changes, and funnel the output into spr

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `BROWSE_AI_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
