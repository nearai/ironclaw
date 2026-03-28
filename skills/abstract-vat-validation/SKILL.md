---
name: abstract-vat-validation
version: "1.0.0"
description: Abstract Vat Validation API — An API that validates VAT numbers against official registries
activation:
  keywords:
    - "abstract-vat-validation"
    - "abstract vat validation"
    - "vat validation"
  patterns:
    - "(?i)abstract.?vat.?validation"
  tags:
    - "tools"
    - "vat-validation"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ABSTRACT_VAT_VALIDATION_API_KEY]
---

# Abstract Vat Validation API

Use the `http` tool. API key is automatically injected as `api_key` query parameter.

> An API that validates VAT numbers against official registries, confirms company details, and helps businesses ensure tax compliance, reduce fraud risk, and automate cross-border invoicing workflows.

## Authentication

This integration uses **query parameter** authentication via `api_key`.

## Required Credentials

- `ABSTRACT_VAT_VALIDATION_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
