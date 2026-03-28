---
name: bluebeam
version: "1.0.0"
description: Bluebeam API — Bluebeam is a PDF-centric collaboration platform tailored for architecture
activation:
  keywords:
    - "bluebeam"
    - "software"
  patterns:
    - "(?i)bluebeam"
  tags:
    - "software"
    - "development"
    - "tools"
  max_context_tokens: 1200
---

# Bluebeam API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Bluebeam is a PDF-centric collaboration platform tailored for architecture, engineering, and construction professionals, offering industry-grade markup, measurement, takeoff, and document management t

## Authentication

This integration uses **OAuth 2.0**. The token is managed automatically — no manual auth setup required in API calls.

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
