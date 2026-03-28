---
name: alt-text-ai
version: "1.0.0"
description: AltText AI API — An AI-driven service that automatically analyzes images and generates descriptiv
activation:
  keywords:
    - "alt-text-ai"
    - "alttext ai"
    - "generation tool"
  patterns:
    - "(?i)alt.?text.?ai"
  tags:
    - "tools"
    - "generation-tool"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ALT_TEXT_AI_API_KEY]
---

# AltText AI API

Use the `http` tool. API key is automatically injected via `X-API-Key` header — **never construct auth headers manually**.

> An AI-driven service that automatically analyzes images and generates descriptive, SEO-friendly alt text to improve website accessibility, enhance search visibility across languages, and streamline im

## Authentication

This integration uses **API Key** authentication via the `X-API-Key` header.
Format: `X-API-Key: ...`

## Required Credentials

- `ALT_TEXT_AI_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
