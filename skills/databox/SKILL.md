---
name: databox
version: "1.0.0"
description: Databox API — Databox is a business analytics platform that unifies metrics from multiple tool
activation:
  keywords:
    - "databox"
    - "analytics"
  patterns:
    - "(?i)databox"
  tags:
    - "analytics"
    - "data"
    - ""
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [DATABOX_API_KEY]
---

# Databox API

Use the `http` tool. API key is automatically injected via `x-api-key` header — **never construct auth headers manually**.

> Databox is a business analytics platform that unifies metrics from multiple tools into customizable dashboards and alerts, helping teams track performance and make data-driven decisions in real time.

## Authentication

This integration uses **API Key** authentication via the `x-api-key` header.
Format: `x-api-key: ...`

## Required Credentials

- `DATABOX_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
