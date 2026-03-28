---
name: reply-io
version: "1.0.0"
description: Reply.io API — Reply.io enables sales and marketing teams to automate and scale multichannel ou
activation:
  keywords:
    - "reply-io"
    - "reply.io"
    - "sales"
  patterns:
    - "(?i)reply.?io"
  tags:
    - "tools"
    - "sales"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [REPLY_IO_API_KEY]
---

# Reply.io API

Use the `http` tool. API key is automatically injected via `x-api-key` header — **never construct auth headers manually**.

> Reply.io enables sales and marketing teams to automate and scale multichannel outreach by orchestrating personalized sequences via email, LinkedIn, SMS, calls, and WhatsApp. It incorporates AI-generat

## Authentication

This integration uses **API Key** authentication via the `x-api-key` header.
Format: `x-api-key: ...`

## Required Credentials

- `REPLY_IO_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
