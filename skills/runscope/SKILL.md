---
name: runscope
version: "1.0.0"
description: Runscope API — A cloud-based service that enables developers and QA/DevOps teams to create
activation:
  keywords:
    - "runscope"
    - "api testing"
  patterns:
    - "(?i)runscope"
  tags:
    - "tools"
    - "api-testing"
  max_context_tokens: 1200
---

# Runscope API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> A cloud-based service that enables developers and QA/DevOps teams to create, automate, monitor, and debug API tests, verify performance and uptime, and trigger alerts on issues to ensure reliable API 

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
