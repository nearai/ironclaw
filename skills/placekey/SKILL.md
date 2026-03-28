---
name: placekey
version: "1.0.0"
description: Placekey API — Placekey is a developer-oriented service that provides a universal identifier (P
activation:
  keywords:
    - "placekey"
    - "geospatial"
  patterns:
    - "(?i)placekey"
  tags:
    - "tools"
    - "geospatial"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [PLACEKEY_API_KEY]
---

# Placekey API

Use the `http` tool. API key is automatically injected via `apikey` header — **never construct auth headers manually**.

> Placekey is a developer-oriented service that provides a universal identifier (Placekey) for any physical place by address or geographic coordinates, enabling easier address/POI matching, deduplicatio

## Authentication

This integration uses **API Key** authentication via the `apikey` header.
Format: `apikey: ...`

## Required Credentials

- `PLACEKEY_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
