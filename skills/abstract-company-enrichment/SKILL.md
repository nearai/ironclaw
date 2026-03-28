---
name: abstract-company-enrichment
version: "1.0.0"
description: Abstract Company Enrichment API — An API that enriches company records with firmographic data such as industry
activation:
  keywords:
    - "abstract-company-enrichment"
    - "abstract company enrichment"
    - "data enrichment"
  patterns:
    - "(?i)abstract.?company.?enrichment"
  tags:
    - "data-enrichment"
    - "enrichment"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ABSTRACT_COMPANY_ENRICHMENT_API_KEY]
---

# Abstract Company Enrichment API

Use the `http` tool. API key is automatically injected as `api_key` query parameter.

> An API that enriches company records with firmographic data such as industry, size, location, and domain details, enabling businesses to enhance lead profiles, improve segmentation, and power more acc

## Authentication

This integration uses **query parameter** authentication via `api_key`.

## Required Credentials

- `ABSTRACT_COMPANY_ENRICHMENT_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
