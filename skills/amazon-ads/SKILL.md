---
name: amazon-ads
version: "1.0.0"
description: Amazon Ads API — Amazon Ads is an advertising platform that enables brands to promote their produ
activation:
  keywords:
    - "amazon-ads"
    - "amazon ads"
    - "marketing"
  patterns:
    - "(?i)amazon.?ads"
  tags:
    - "marketing"
    - "email"
    - "campaigns"
  max_context_tokens: 1200
---

# Amazon Ads API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Amazon Ads is an advertising platform that enables brands to promote their products across Amazon’s ecosystem, helping them reach shoppers through targeted ads, sponsored listings, and display campaig

## Authentication

This integration uses **OAuth 2.0**. The token is managed automatically — no manual auth setup required in API calls.

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
