---
name: unipile
version: "1.0.0"
description: Unipile API — Unipile is a unified communication and productivity platform that brings togethe
activation:
  keywords:
    - "unipile"
    - "tools"
  patterns:
    - "(?i)unipile"
  tags:
    - "tools"
    - "utility"
    - "tool"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [UNIPILE_DSN, UNIPILE_API_KEY]
---

# Unipile API

Use the `http` tool. API key is automatically injected via `X-API-KEY` header — **never construct auth headers manually**.

> Unipile is a unified communication and productivity platform that brings together emails, messages, calendars, and task management into a single, streamlined interface to help individuals and teams st

## Authentication

This integration uses **API Key** authentication via the `X-API-KEY` header.
Format: `X-API-KEY: ...`

## Required Credentials

- `UNIPILE_DSN` — DSN
- `UNIPILE_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
