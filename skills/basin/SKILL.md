---
name: basin
version: "1.0.0"
description: Basin API — Basin is a form backend service that captures form submissions from static sites
activation:
  keywords:
    - "basin"
    - "workflow automation"
  patterns:
    - "(?i)basin"
  tags:
    - "tools"
    - "workflow-automation"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BASIN_API_KEY]
---

# Basin API

Use the `http` tool. API key is automatically injected as `api_token` query parameter.

> Basin is a form backend service that captures form submissions from static sites and forwards data to email, webhooks or integrations—enabling developers to handle form data without building a custom 

## Authentication

This integration uses **query parameter** authentication via `api_token`.

## Required Credentials

- `BASIN_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
