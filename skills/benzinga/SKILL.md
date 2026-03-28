---
name: benzinga
version: "1.0.0"
description: Benzinga API — A real-time financial news and market data service that delivers breaking market
activation:
  keywords:
    - "benzinga"
    - "financial data"
  patterns:
    - "(?i)benzinga"
  tags:
    - "tools"
    - "financial-data"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BENZINGA_API_KEY]
---

# Benzinga API

Use the `http` tool. API key is automatically injected as `token` query parameter.

> A real-time financial news and market data service that delivers breaking market updates, analysis, earnings, and trading insights to investors, traders, and finance professionals.

## Authentication

This integration uses **query parameter** authentication via `token`.

## Required Credentials

- `BENZINGA_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
