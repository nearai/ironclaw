---
name: parsehub
version: "1.0.0"
description: Parsehub API — ParseHub is a visual, no-code web scraping tool that enables users to extract st
activation:
  keywords:
    - "parsehub"
    - "scraper"
  patterns:
    - "(?i)parsehub"
  tags:
    - "tools"
    - "scraper"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [PARSEHUB_API_KEY]
---

# Parsehub API

Use the `http` tool. API key is automatically injected as `api_key` query parameter.

> ParseHub is a visual, no-code web scraping tool that enables users to extract structured data from dynamic websites—including those built with JavaScript or AJAX—and export the results via JSON, CSV/E

## Authentication

This integration uses **query parameter** authentication via `api_key`.

## Required Credentials

- `PARSEHUB_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
