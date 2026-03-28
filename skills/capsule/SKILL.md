---
name: capsule
version: "1.0.0"
description: Capsule API — Capsule CRM is a streamlined cloud-based CRM designed for small businesses and s
activation:
  keywords:
    - "capsule"
    - "crm"
  patterns:
    - "(?i)capsule"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
---

# Capsule API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Capsule CRM is a streamlined cloud-based CRM designed for small businesses and sales teams, furnishing contact and organisation management, sales pipelines, tasks, projects, and analytics in one place

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
