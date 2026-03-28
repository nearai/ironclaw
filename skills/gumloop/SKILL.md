---
name: gumloop
version: "1.0.0"
description: Gumloop API — Gumloop is a no-code AI automation platform that enables users to build and depl
activation:
  keywords:
    - "gumloop"
    - "ai"
  patterns:
    - "(?i)gumloop"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [GUMLOOP_API_KEY, GUMLOOP_USER_ID]
---

# Gumloop API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Gumloop is a no-code AI automation platform that enables users to build and deploy complex workflows using a visual drag-and-drop interface, integrating large language models and third-party tools to 

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `GUMLOOP_API_KEY` — API Key
- `GUMLOOP_USER_ID` — User ID

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
