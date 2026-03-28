---
name: abstract-avatars
version: "1.0.0"
description: Abstract Avatars API — An API that generates customizable user avatars based on names or unique identif
activation:
  keywords:
    - "abstract-avatars"
    - "abstract avatars"
    - "avatar generation"
  patterns:
    - "(?i)abstract.?avatars"
  tags:
    - "avatar"
    - "images"
    - "avatar-generation"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ABSTRACT_AVATARS_API_KEY]
---

# Abstract Avatars API

Use the `http` tool. API key is automatically injected as `api_key` query parameter.

> An API that generates customizable user avatars based on names or unique identifiers, enabling applications to automatically create consistent, visually distinct profile images without requiring users

## Authentication

This integration uses **query parameter** authentication via `api_key`.

## Required Credentials

- `ABSTRACT_AVATARS_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
