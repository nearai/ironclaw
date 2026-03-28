---
name: hoop
version: "1.0.0"
description: Hoop API — Hoop.dev provides a secure access gateway that enables developers to connect to 
activation:
  keywords:
    - "hoop"
    - "security"
  patterns:
    - "(?i)hoop"
  tags:
    - "tools"
    - "security"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [HOOP_API_KEY, HOOP_BASE_URL]
---

# Hoop API

Use the `http` tool. API key is automatically injected via `Api-Key` header — **never construct auth headers manually**.

> Hoop.dev provides a secure access gateway that enables developers to connect to databases, servers and internal tools while automatically masking sensitive data, enforcing command-level guardrails, an

## Authentication

This integration uses **API Key** authentication via the `Api-Key` header.
Format: `Api-Key: ...`

## Required Credentials

- `HOOP_API_KEY` — API Key
- `HOOP_BASE_URL` — Base URL

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
