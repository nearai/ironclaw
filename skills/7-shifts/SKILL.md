---
name: 7-shifts
version: "1.0.0"
description: 7shifts API — 7Shifts is a cloud‑based workforce management platform tailored for restaurants
activation:
  keywords:
    - "7-shifts"
    - "7shifts"
    - "hospitality"
  patterns:
    - "(?i)7.?shifts"
  tags:
    - "hospitality"
    - "scheduling"
  max_context_tokens: 1200
---

# 7shifts API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> 7Shifts is a cloud‑based workforce management platform tailored for restaurants, combining intuitive drag‑and‑drop scheduling, mobile time tracking, automated payroll and tip management, labor complia

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
