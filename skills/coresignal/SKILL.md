---
name: coresignal
version: "1.0.0"
description: Coresignal API — Coresignal provides large-scale public web data for investment, HR tech
activation:
  keywords:
    - "coresignal"
    - "ai"
  patterns:
    - "(?i)coresignal"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CORESIGNAL_API_KEY]
---

# Coresignal API

Use the `http` tool. API key is automatically injected via `apikey` header — **never construct auth headers manually**.

> Coresignal provides large-scale public web data for investment, HR tech, and other industries. It offers datasets on companies, jobs, people, and more to power business insights and machine learning m

## Authentication

This integration uses **API Key** authentication via the `apikey` header.
Format: `apikey: ...`

## Required Credentials

- `CORESIGNAL_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
