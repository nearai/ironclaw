---
name: fathom
version: "1.0.0"
description: Fathom API — Fathom.ai is an AI-powered meeting assistant that automatically records
activation:
  keywords:
    - "fathom"
    - "ai"
  patterns:
    - "(?i)fathom"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
---

# Fathom API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Fathom.ai is an AI-powered meeting assistant that automatically records, transcribes, and summarizes virtual meetings across platforms like Zoom, Google Meet, and Teams, extracting key points, decisio

## Authentication

This integration uses **OAuth 2.0**. The token is managed automatically — no manual auth setup required in API calls.

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
