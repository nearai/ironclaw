---
name: meet-geek
version: "1.0.0"
description: MeetGeek API — MeetGeek is an AI-powered meeting assistant that records, transcribes
activation:
  keywords:
    - "meet-geek"
    - "meetgeek"
    - "ai"
  patterns:
    - "(?i)meet.?geek"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [MEET_GEEK_BASE_URL, MEET_GEEK_API_KEY]
---

# MeetGeek API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> MeetGeek is an AI-powered meeting assistant that records, transcribes, and summarizes meetings, helping teams stay aligned, follow up on action items, and boost productivity.

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `MEET_GEEK_BASE_URL` — Region Endpoint
- `MEET_GEEK_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
