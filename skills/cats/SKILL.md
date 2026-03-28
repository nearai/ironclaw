---
name: cats
version: "1.0.0"
description: CATS API — CATS is a web-based applicant tracking system designed for recruiting agencies a
activation:
  keywords:
    - "cats"
    - "ats"
  patterns:
    - "(?i)cats"
  tags:
    - "tools"
    - "ats"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CATS_API_KEY]
---

# CATS API

Use the `http` tool. API key is automatically injected via `authorization` header — **never construct auth headers manually**.

> CATS is a web-based applicant tracking system designed for recruiting agencies and HR teams, offering tools for job posting, resume parsing, candidate tracking, custom workflows, analytics, and integr

## Authentication

This integration uses **API Key** authentication via the `authorization` header.
Format: `authorization: Token ...`

## Required Credentials

- `CATS_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
