---
name: kernel
version: "1.0.0"
description: Kernel API — Kernel is a serverless browser automation and web agent platform that enables de
activation:
  keywords:
    - "kernel"
    - "tools"
  patterns:
    - "(?i)kernel"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [KERNEL_API_KEY]
---

# Kernel API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Kernel is a serverless browser automation and web agent platform that enables developers to deploy, scale, and run headless or full-browser tasks as APIs without managing infrastructure, offering para

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `KERNEL_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
