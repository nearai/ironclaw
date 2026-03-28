---
name: people-data-labs
version: "1.0.0"
description: PeopleDataLabs API — People Data Labs provides B2B data enrichment services with access to global dat
activation:
  keywords:
    - "people-data-labs"
    - "peopledatalabs"
    - "ai"
  patterns:
    - "(?i)people.?data.?labs"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [PEOPLEDATALABS_API_KEY]
---

# PeopleDataLabs API

Use the `http` tool. API key is automatically injected via `X-Api-Key` header — **never construct auth headers manually**.

> People Data Labs provides B2B data enrichment services with access to global datasets of professionals and companies. It helps teams improve lead scoring, segmentation, and personalization.

## Authentication

This integration uses **API Key** authentication via the `X-Api-Key` header.
Format: `X-Api-Key: ...`

## Required Credentials

- `PEOPLEDATALABS_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
