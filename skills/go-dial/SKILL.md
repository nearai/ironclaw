---
name: go-dial
version: "1.0.0"
description: Go Dial API — Go Dial is an all-in-one mobile CRM and auto-dialer platform enabling businesses
activation:
  keywords:
    - "go-dial"
    - "go dial"
    - "crm"
  patterns:
    - "(?i)go.?dial"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [GO_DIAL_ACCESS_TOKEN]
---

# Go Dial API

Use the `http` tool. API key is automatically injected as `access_token` query parameter.

> Go Dial is an all-in-one mobile CRM and auto-dialer platform enabling businesses to import contacts, manage pipelines, automate outbound calls, record interactions and track team performance from a sm

## Authentication

This integration uses **query parameter** authentication via `access_token`.

## Required Credentials

- `GO_DIAL_ACCESS_TOKEN` — Access Token

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
