---
name: abstract-email-reputation
version: "1.0.0"
description: Abstract Email Reputation API — An API that evaluates the reputation of email addresses by analyzing risk factor
activation:
  keywords:
    - "abstract-email-reputation"
    - "abstract email reputation"
    - "email verification"
  patterns:
    - "(?i)abstract.?email.?reputation"
  tags:
    - "tools"
    - "email-verification"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ABSTRACT_EMAIL_REPUTATION_API_KEY]
---

# Abstract Email Reputation API

Use the `http` tool. API key is automatically injected as `api_key` query parameter.

> An API that evaluates the reputation of email addresses by analyzing risk factors to help applications improve deliverability, reduce fraud, and enhance email validation accuracy.

## Authentication

This integration uses **query parameter** authentication via `api_key`.

## Required Credentials

- `ABSTRACT_EMAIL_REPUTATION_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
