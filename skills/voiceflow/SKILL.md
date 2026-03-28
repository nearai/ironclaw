---
name: voiceflow
version: "1.0.0"
description: Voiceflow API — Voiceflow is a collaborative platform that enables teams to design, prototype
activation:
  keywords:
    - "voiceflow"
    - "ai"
  patterns:
    - "(?i)voiceflow"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
    - "communication"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [VOICEFLOW_API_KEY]
---

# Voiceflow API

Use the `http` tool. API key is automatically injected via `Authorization` header — **never construct auth headers manually**.

> Voiceflow is a collaborative platform that enables teams to design, prototype, and build conversational AI experiences like voice assistants and chatbots without writing code.

## Authentication

This integration uses **API Key** authentication via the `Authorization` header.
Format: `Authorization: ...`

## Required Credentials

- `VOICEFLOW_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
