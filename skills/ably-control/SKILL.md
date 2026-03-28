---
name: ably-control
version: "1.0.0"
description: Ably Control API — Ably Control API is a RESTful interface that enables developers and DevOps teams
activation:
  keywords:
    - "ably-control"
    - "ably control"
    - "tools"
  patterns:
    - "(?i)ably.?control"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ABLY_CONTROL_ACCESS_TOKEN]
---

# Ably Control API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Ably Control API is a RESTful interface that enables developers and DevOps teams to programmatically provision, configure, and manage real-time infrastructure—such as apps, API keys, namespaces, queue

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `ABLY_CONTROL_ACCESS_TOKEN` — Access Token

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
