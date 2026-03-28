---
name: folk-app
version: "1.0.0"
description: Folk.app API — Folk is a simple, AI-powered CRM designed for service businesses
activation:
  keywords:
    - "folk-app"
    - "folk.app"
    - "crm"
  patterns:
    - "(?i)folk.?app"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [FOLK_APP_API_KEY]
---

# Folk.app API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Folk is a simple, AI-powered CRM designed for service businesses. It features spreadsheet-like interface, automated contact management, Chrome extension for one-click imports, Gmail integration, and p

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `FOLK_APP_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
