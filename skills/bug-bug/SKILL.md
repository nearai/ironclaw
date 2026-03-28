---
name: bug-bug
version: "1.0.0"
description: BugBug API — A no-code testing platform that automatically crawls websites and web apps to de
activation:
  keywords:
    - "bug-bug"
    - "bugbug"
    - "bug detection"
  patterns:
    - "(?i)bug.?bug"
  tags:
    - "tools"
    - "bug-detection"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BUG_BUG_API_KEY]
---

# BugBug API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

> A no-code testing platform that automatically crawls websites and web apps to detect visual issues, broken links, and UI regressions, helping teams maintain quality, catch bugs early, and streamline Q

## Authentication

This integration uses **API Key** authentication via the `Authorization` header.
Format: `Authorization: Token ...`

## Required Credentials

- `BUG_BUG_API_KEY` — API key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
