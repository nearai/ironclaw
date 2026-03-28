---
name: ably
version: "1.0.0"
description: Ably API — Ably Pub/Sub is a global serverless real-time messaging platform that delivers s
activation:
  keywords:
    - "ably"
    - "tools"
  patterns:
    - "(?i)ably"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ABLY_ENCODED_API_KEY]
---

# Ably API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

> Ably Pub/Sub is a global serverless real-time messaging platform that delivers sub‑60 ms latency pub/sub capabilities—including message history, presence detection, exactly‑once delivery, and guarante

## Authentication

This integration uses **API Key** authentication via the `Authorization` header.
Format: `Authorization: Basic ...`

## Required Credentials

- `ABLY_ENCODED_API_KEY` — API Key (RFC 4648 Base64 Encoded)

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
