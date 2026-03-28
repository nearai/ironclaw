---
name: sage-sales-management
version: "1.0.0"
description: Sage Sales Management API — Sage Sales Management (formerly ForceManager) is a mobile-first CRM built for fi
activation:
  keywords:
    - "sage-sales-management"
    - "sage sales management"
    - "crm"
  patterns:
    - "(?i)sage.?sales.?management"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SAGE_SALES_MANAGEMENT_API_KEY, SAGE_SALES_MANAGEMENT_BASE_URL]
---

# Sage Sales Management API

Use the `http` tool. API key is automatically injected via `X-Session-Key` header — **never construct auth headers manually**.

> Sage Sales Management (formerly ForceManager) is a mobile-first CRM built for field sales teams, offering tools like route planning, visit tracking, pipeline management and AI-driven insights to help 

## Authentication

This integration uses **API Key** authentication via the `X-Session-Key` header.
Format: `X-Session-Key: ...`

## Required Credentials

- `SAGE_SALES_MANAGEMENT_API_KEY` — API Key
- `SAGE_SALES_MANAGEMENT_BASE_URL` — Base URL

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
