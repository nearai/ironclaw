---
name: abstract-iban-validation
version: "1.0.0"
description: Abstract IBAN Validation API — An API that validates IBAN numbers by checking format, bank details
activation:
  keywords:
    - "abstract-iban-validation"
    - "abstract iban validation"
    - "iban validation"
  patterns:
    - "(?i)abstract.?iban.?validation"
  tags:
    - "tools"
    - "iban-validation"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ABSTRACT_IBAN_VALIDATION_API_KEY]
---

# Abstract IBAN Validation API

Use the `http` tool. API key is automatically injected as `api_key` query parameter.

> An API that validates IBAN numbers by checking format, bank details, and country-specific rules to help businesses prevent payment errors, reduce fraud risk, and streamline international transactions.

## Authentication

This integration uses **query parameter** authentication via `api_key`.

## Required Credentials

- `ABSTRACT_IBAN_VALIDATION_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
