---
name: anchor-browser
version: "1.0.0"
description: Anchor Browser API — Anchor Browser is a cloud-hosted automation platform that lets AI agents interac
activation:
  keywords:
    - "anchor-browser"
    - "anchor browser"
    - "web automation"
  patterns:
    - "(?i)anchor.?browser"
  tags:
    - "tools"
    - "web-automation"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ANCHOR_BROWSER_API_KEY]
---

# Anchor Browser API

Use the `http` tool. API key is automatically injected via `anchor-api-key` header — **never construct auth headers manually**.

> Anchor Browser is a cloud-hosted automation platform that lets AI agents interact with web pages like a human—navigating sites, clicking, typing, submitting forms and extracting data—so teams can auto

## Authentication

This integration uses **API Key** authentication via the `anchor-api-key` header.
Format: `anchor-api-key: ...`

## Required Credentials

- `ANCHOR_BROWSER_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
