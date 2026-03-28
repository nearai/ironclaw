---
name: phantom-buster
version: "1.0.0"
description: PhantomBuster API — PhantomBuster is a cloud‑based automation platform featuring prebuilt “Phantoms”
activation:
  keywords:
    - "phantom-buster"
    - "phantombuster"
    - "ai"
  patterns:
    - "(?i)phantom.?buster"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [PHANTOM_BUSTER_API_KEY]
---

# PhantomBuster API

Use the `http` tool. API key is automatically injected via `X-Phantombuster-Key-1` header — **never construct auth headers manually**.

> PhantomBuster is a cloud‑based automation platform featuring prebuilt “Phantoms” and visual “Workflows” that extract leads and automate engagement on social platforms like LinkedIn, Instagram, Twitter

## Authentication

This integration uses **API Key** authentication via the `X-Phantombuster-Key-1` header.
Format: `X-Phantombuster-Key-1: ...`

## Required Credentials

- `PHANTOM_BUSTER_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
