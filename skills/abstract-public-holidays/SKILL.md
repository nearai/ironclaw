---
name: abstract-public-holidays
version: "1.0.0"
description: Abstract Public Holidays API — An API that delivers official public holiday data by country and year
activation:
  keywords:
    - "abstract-public-holidays"
    - "abstract public holidays"
    - "public holidays"
  patterns:
    - "(?i)abstract.?public.?holidays"
  tags:
    - "tools"
    - "public-holidays"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ABSTRACT_PUBLIC_HOLIDAYS_API_KEY]
---

# Abstract Public Holidays API

Use the `http` tool. API key is automatically injected as `api_key` query parameter.

> An API that delivers official public holiday data by country and year, enabling applications to access national and regional holiday calendars for scheduling, localization, and compliance use cases.

## Authentication

This integration uses **query parameter** authentication via `api_key`.

## Required Credentials

- `ABSTRACT_PUBLIC_HOLIDAYS_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
