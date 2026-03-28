---
name: bouncer
version: "1.0.0"
description: Bouncer API — A cloud-based platform that verifies and cleans email lists by identifying inval
activation:
  keywords:
    - "bouncer"
    - "email verification"
  patterns:
    - "(?i)bouncer"
  tags:
    - "tools"
    - "email-verification"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BOUNCER_API_KEY]
---

# Bouncer API

Use the `http` tool. API key is automatically injected via `x-api-key` header — **never construct auth headers manually**.

> A cloud-based platform that verifies and cleans email lists by identifying invalid, risky, or disposable email addresses to improve deliverability, reduce bounce rates, and enhance email campaign perf

## Authentication

This integration uses **API Key** authentication via the `x-api-key` header.
Format: `x-api-key: ...`

## Required Credentials

- `BOUNCER_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
