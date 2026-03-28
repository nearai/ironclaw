---
name: better-proposals
version: "1.0.0"
description: Better Proposals API — A web-based platform that enables businesses to create, customize, send
activation:
  keywords:
    - "better-proposals"
    - "better proposals"
    - "proposal software"
  patterns:
    - "(?i)better.?proposals"
  tags:
    - "tools"
    - "proposal-software"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BETTER_PROPOSALS_API_TOKEN]
---

# Better Proposals API

Use the `http` tool. API key is automatically injected via `Bptoken` header — **never construct auth headers manually**.

> A web-based platform that enables businesses to create, customize, send, and track professional sales proposals and contracts with interactive elements, e-signatures, and analytics to streamline clien

## Authentication

This integration uses **API Key** authentication via the `Bptoken` header.
Format: `Bptoken: ...`

## Required Credentials

- `BETTER_PROPOSALS_API_TOKEN` — API Token 

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
