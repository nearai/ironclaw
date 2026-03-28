---
name: octave
version: "1.0.0"
description: Octave API — OctaveHQ is a generative AI platform for Go‑to‑Market teams
activation:
  keywords:
    - "octave"
    - "ai"
  patterns:
    - "(?i)octave"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [OCTAVE_API_KEY]
---

# Octave API

Use the `http` tool. API key is automatically injected via `api_key` header — **never construct auth headers manually**.

> OctaveHQ is a generative AI platform for Go‑to‑Market teams, automatically refining ideal customer profiles and outbound messaging in real time—providing dynamic playbooks, personalized outreach, and 

## Authentication

This integration uses **API Key** authentication via the `api_key` header.
Format: `api_key: ...`

## Required Credentials

- `OCTAVE_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
