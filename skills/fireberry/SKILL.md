---
name: fireberry
version: "1.0.0"
description: Fireberry API — A unified CRM platform designed to bring sales, marketing
activation:
  keywords:
    - "fireberry"
    - "crm"
  patterns:
    - "(?i)fireberry"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [FIREBERRY_ACCESS_TOKEN]
---

# Fireberry API

Use the `http` tool. API key is automatically injected via `tokenid` header — **never construct auth headers manually**.

> A unified CRM platform designed to bring sales, marketing, and service data into one customizable hub, Fireberry offers businesses of all sizes AI-assisted automation, real-time insights, and flexible

## Authentication

This integration uses **API Key** authentication via the `tokenid` header.
Format: `tokenid: ...`

## Required Credentials

- `FIREBERRY_ACCESS_TOKEN` — Access Token

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
