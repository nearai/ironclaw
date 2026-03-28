---
name: bright-data
version: "1.0.0"
description: BrightData API — Bright Data (formerly Luminati) is a comprehensive web data platform offering a 
activation:
  keywords:
    - "bright-data"
    - "brightdata"
    - "tools"
  patterns:
    - "(?i)bright.?data"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BRIGHT_DATA_API_KEY]
---

# BrightData API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Bright Data (formerly Luminati) is a comprehensive web data platform offering a global proxy network (residential, mobile, ISP, data center), browser-based scraping, SERP APIs, and managed data pipeli

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `BRIGHT_DATA_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
