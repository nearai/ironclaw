---
name: riveter
version: "1.0.0"
description: Riveter API — Riveter is a no-code AI platform that automates web research and data enrichment
activation:
  keywords:
    - "riveter"
    - "scraper"
  patterns:
    - "(?i)riveter"
  tags:
    - "tools"
    - "scraper"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [RIVETER_API_KEY]
---

# Riveter API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Riveter is a no-code AI platform that automates web research and data enrichment by using intelligent agents to browse, scrape, read documents, and return structured, auditable outputs for large datas

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `RIVETER_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
