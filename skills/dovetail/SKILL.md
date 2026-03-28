---
name: dovetail
version: "1.0.0"
description: Dovetail API — Dovetail is an AI-native customer intelligence platform that centralises and syn
activation:
  keywords:
    - "dovetail"
    - "cip"
  patterns:
    - "(?i)dovetail"
  tags:
    - "tools"
    - "cip"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [DOVETAIL_API_TOKEN]
---

# Dovetail API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Dovetail is an AI-native customer intelligence platform that centralises and synthesises feedback from interviews, support tickets, reviews and research data—helping teams uncover themes, drive produc

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `DOVETAIL_API_TOKEN` — API Token

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
