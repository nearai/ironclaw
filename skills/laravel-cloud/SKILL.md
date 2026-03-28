---
name: laravel-cloud
version: "1.0.0"
description: Laravel Cloud API — A fully managed cloud platform for deploying, scaling
activation:
  keywords:
    - "laravel-cloud"
    - "laravel cloud"
    - "cloud hosting"
  patterns:
    - "(?i)laravel.?cloud"
  tags:
    - "tools"
    - "cloud-hosting"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [LARAVEL_CLOUD_API_TOKEN]
---

# Laravel Cloud API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> A fully managed cloud platform for deploying, scaling, and operating Laravel applications with built-in autoscaling, managed databases, caching, storage, security, and zero-server management so develo

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `LARAVEL_CLOUD_API_TOKEN` — API Token

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
