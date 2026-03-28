---
name: twelve-data
version: "1.0.0"
description: Twelve Data API — Twelve Data provides unified REST and WebSocket APIs delivering real-time and hi
activation:
  keywords:
    - "twelve-data"
    - "twelve data"
    - "finance"
  patterns:
    - "(?i)twelve.?data"
  tags:
    - "tools"
    - "finance"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [TWELVE_DATA_API_KEY]
---

# Twelve Data API

Use the `http` tool. API key is automatically injected as `apikey` query parameter.

> Twelve Data provides unified REST and WebSocket APIs delivering real-time and historical market data—including stocks, forex, crypto, ETFs, commodities, and fundamentals—alongside technical indicators

## Authentication

This integration uses **query parameter** authentication via `apikey`.

## Required Credentials

- `TWELVE_DATA_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
