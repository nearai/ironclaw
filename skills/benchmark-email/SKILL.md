---
name: benchmark-email
version: "1.0.0"
description: Benchmark Email API — Benchmark Email is an intuitive email-marketing platform that enables marketers 
activation:
  keywords:
    - "benchmark-email"
    - "benchmark email"
    - "marketing"
  patterns:
    - "(?i)benchmark.?email"
  tags:
    - "marketing"
    - "email"
    - "campaigns"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BENCHMARK_EMAIL_AUTH_TOKEN]
---

# Benchmark Email API

Use the `http` tool. API key is automatically injected via `AuthToken` header — **never construct auth headers manually**.

> Benchmark Email is an intuitive email-marketing platform that enables marketers to design mobile-responsive campaigns with drag-and-drop ease, segment contacts for targeted sends, and track real-time 

## Authentication

This integration uses **API Key** authentication via the `AuthToken` header.
Format: `AuthToken: ...`

## Required Credentials

- `BENCHMARK_EMAIL_AUTH_TOKEN` — Auth Token

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
