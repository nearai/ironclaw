---
name: contentstack-content-delivery
version: "1.0.0"
description: Contentstack Content Delivery API — Contentstack Content Delivery API is a high-performance
activation:
  keywords:
    - "contentstack-content-delivery"
    - "contentstack content delivery"
    - "tools"
  patterns:
    - "(?i)contentstack.?content.?delivery"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CONTENTSTACK_CONTENT_DELIVERY_BASE_URL, CONTENTSTACK_CONTENT_DELIVERY_STACK_API_KEY, CONTENTSTACK_CONTENT_DELIVERY_DELIVERY_TOKEN]
---

# Contentstack Content Delivery API

Use the `http` tool. API key is automatically injected via `api_key` header — **never construct auth headers manually**.

> Contentstack Content Delivery API is a high-performance, read-only REST and GraphQL service that retrieves published content from your headless CMS via a global CDN—supporting efficient queries, cachi

## Authentication

This integration uses **API Key** authentication via the `api_key` header.
Format: `api_key: ...`

## Required Credentials

- `CONTENTSTACK_CONTENT_DELIVERY_BASE_URL` — Base URL
- `CONTENTSTACK_CONTENT_DELIVERY_STACK_API_KEY` — Stack API Key
- `CONTENTSTACK_CONTENT_DELIVERY_DELIVERY_TOKEN` — Delivery Token

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
