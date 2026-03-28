---
name: abstract-phone-intelligence
version: "1.0.0"
description: Abstract Phone Intelligence API — An API that validates and enriches phone numbers with carrier, location
activation:
  keywords:
    - "abstract-phone-intelligence"
    - "abstract phone intelligence"
    - "phone validation"
  patterns:
    - "(?i)abstract.?phone.?intelligence"
  tags:
    - "tools"
    - "phone-validation"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ABSTRACT_PHONE_INTELLIGENCE_API_KEY]
---

# Abstract Phone Intelligence API

Use the `http` tool. API key is automatically injected as `api_key` query parameter.

> An API that validates and enriches phone numbers with carrier, location, and line type data, enabling applications to verify user input, detect fraud risk, and improve communication accuracy globally.

## Authentication

This integration uses **query parameter** authentication via `api_key`.

## Required Credentials

- `ABSTRACT_PHONE_INTELLIGENCE_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
