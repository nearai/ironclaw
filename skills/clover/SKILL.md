---
name: clover
version: "1.0.0"
description: Clover API — Clover is a point-of-sale and business management system that helps businesses a
activation:
  keywords:
    - "clover"
    - "pos"
  patterns:
    - "(?i)clover"
  tags:
    - "tools"
    - "POS"
  max_context_tokens: 1200
---

# Clover API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Clover is a point-of-sale and business management system that helps businesses accept payments, track sales, manage inventory, and run operations from one integrated platform.

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
