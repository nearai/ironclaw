---
name: sindri
version: "1.0.0"
description: Sindri API — Sindri is a zero-knowledge developer cloud platform that provides serverless inf
activation:
  keywords:
    - "sindri"
    - "tools"
  patterns:
    - "(?i)sindri"
  tags:
    - "tools"
    - "utility"
    - "tool"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SINDRI_API_KEY]
---

# Sindri API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Sindri is a zero-knowledge developer cloud platform that provides serverless infrastructure for zero-knowledge (ZK) applications. It enables teams to go from idea to production quickly and securely.

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `SINDRI_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
