---
name: cockroach-labs
version: "1.0.0"
description: Cockroach Labs API — Cockroach Labs develops a distributed SQL database designed for cloud applicatio
activation:
  keywords:
    - "cockroach-labs"
    - "cockroach labs"
    - "cloud database"
  patterns:
    - "(?i)cockroach.?labs"
  tags:
    - "tools"
    - "cloud-database"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [COCKROACH_LABS_API_KEY]
---

# Cockroach Labs API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

> Cockroach Labs develops a distributed SQL database designed for cloud applications, providing scalable, resilient, strongly consistent data storage with multi-region replication, automatic failover, a

## Authentication

This integration uses **API Key** authentication via the `Authorization` header.
Format: `Authorization: Bearer ...`

## Required Credentials

- `COCKROACH_LABS_API_KEY` — API Keys

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
