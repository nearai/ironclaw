---
name: contentstack-content-management
version: "1.0.0"
description: Contentstack Content Management API — Contentstack’s Content Management API is a powerful
activation:
  keywords:
    - "contentstack-content-management"
    - "contentstack content management"
    - "tools"
  patterns:
    - "(?i)contentstack.?content.?management"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CONTENTSTACK_CONTENT_MANAGEMENT_STACK_API_KEY]
---

# Contentstack Content Management API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Contentstack’s Content Management API is a powerful, API-first REST interface that enables developers to programmatically create, update, delete, and structure content—supporting secure token-based ac

## Authentication

This integration uses **OAuth 2.0**. The token is managed automatically — no manual auth setup required in API calls.

## Required Credentials

- `CONTENTSTACK_CONTENT_MANAGEMENT_STACK_API_KEY` — Stack API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
