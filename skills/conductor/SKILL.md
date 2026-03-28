---
name: conductor
version: "1.0.0"
description: Conductor API — Conductor is an enterprise-scale website optimization and AI-powered intelligenc
activation:
  keywords:
    - "conductor"
    - "tools"
  patterns:
    - "(?i)conductor"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CONDUCTOR_API_KEY]
---

# Conductor API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Conductor is an enterprise-scale website optimization and AI-powered intelligence platform that combines SEO, AI-generated content suggestions, and continuous site health monitoring—empowering marketi

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `CONDUCTOR_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
