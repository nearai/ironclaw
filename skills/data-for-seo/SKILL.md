---
name: data-for-seo
version: "1.0.0"
description: Data For SEO API — DataForSEO provides a comprehensive suite of RESTful APIs and data services that
activation:
  keywords:
    - "data-for-seo"
    - "data for seo"
    - "seo"
  patterns:
    - "(?i)data.?for.?seo"
  tags:
    - "tools"
    - "seo"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [DATA_FOR_SEO_BASE64_CREDENTIALS]
---

# Data For SEO API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

> DataForSEO provides a comprehensive suite of RESTful APIs and data services that deliver search-engine results (SERP), keyword metrics, backlinks, app store info, site technical audits, and domain ana

## Authentication

This integration uses **API Key** authentication via the `Authorization` header.
Format: `Authorization: Basic ...`

## Required Credentials

- `DATA_FOR_SEO_BASE64_CREDENTIALS` — Basic Credential (Base64)

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
