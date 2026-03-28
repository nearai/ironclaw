---
name: byte-forms
version: "1.0.0"
description: ByteForms API — A no-code form creation tool that enables businesses to design, deploy
activation:
  keywords:
    - "byte-forms"
    - "byteforms"
    - "form builder"
  patterns:
    - "(?i)byte.?forms"
  tags:
    - "tools"
    - "form-builder"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BYTE_FORMS_API_KEY]
---

# ByteForms API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

> A no-code form creation tool that enables businesses to design, deploy, and embed customizable online forms for data collection, surveys, and customer feedback with optional logic and integrations.

## Authentication

This integration uses **API Key** authentication via the `Authorization` header.
Format: `Authorization: ...`

## Required Credentials

- `BYTE_FORMS_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
