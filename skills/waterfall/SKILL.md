---
name: waterfall
version: "1.0.0"
description: Waterfall API — Waterfall provides a unified B2B data platform that aggregates multiple vendors 
activation:
  keywords:
    - "waterfall"
    - "data"
  patterns:
    - "(?i)waterfall"
  tags:
    - "tools"
    - "data"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [WATERFALL_API_KEY]
---

# Waterfall API

Use the `http` tool. API key is automatically injected via `x-api-key` header — **never construct auth headers manually**.

> Waterfall provides a unified B2B data platform that aggregates multiple vendors into one API to deliver verified contact details, phone numbers, job changes, and enrichment at scale for sales and GTM 

## Authentication

This integration uses **API Key** authentication via the `x-api-key` header.
Format: `x-api-key: ...`

## Required Credentials

- `WATERFALL_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
