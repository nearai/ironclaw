---
name: vitally
version: "1.0.0"
description: Vitally API — Vitally is a customer success platform that helps B2B teams monitor product usag
activation:
  keywords:
    - "vitally"
    - "csp"
  patterns:
    - "(?i)vitally"
  tags:
    - "tools"
    - "csp"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [VITALLY_API_KEY, VITALLY_BASE_URL]
---

# Vitally API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

> Vitally is a customer success platform that helps B2B teams monitor product usage, segment accounts, automate workflows, and drive retention and expansion using data-driven insights and playbooks.

## Authentication

This integration uses **API Key** authentication via the `Authorization` header.
Format: `Authorization: Basic ...`

## Required Credentials

- `VITALLY_API_KEY` — API Key
- `VITALLY_BASE_URL` — Base URL

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
