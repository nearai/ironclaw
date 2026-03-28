---
name: bug-herd
version: "1.0.0"
description: BugHerd API — A web-based issue and feedback tracking tool that lets teams capture visual bugs
activation:
  keywords:
    - "bug-herd"
    - "bugherd"
    - "bug tracking"
  patterns:
    - "(?i)bug.?herd"
  tags:
    - "tools"
    - "bug-tracking"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BUG_HERD_USERNAME, BUG_HERD_PASSWORD]
---

# BugHerd API

Use the `http` tool. Credentials are automatically injected.

> A web-based issue and feedback tracking tool that lets teams capture visual bugs directly on web pages, annotate problems, manage tickets, and collaborate on fixes for faster design and development cy

## Authentication


## Required Credentials

- `BUG_HERD_USERNAME` — API Key
- `BUG_HERD_PASSWORD` — Password

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
