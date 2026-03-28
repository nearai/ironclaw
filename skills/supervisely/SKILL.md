---
name: supervisely
version: "1.0.0"
description: Supervisely API — A web-based computer vision platform that enables teams to annotate and manage d
activation:
  keywords:
    - "supervisely"
    - "computer vision"
  patterns:
    - "(?i)supervisely"
  tags:
    - "tools"
    - "computer-vision"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SUPERVISELY_API_KEY]
---

# Supervisely API

Use the `http` tool. API key is automatically injected via `x-api-key` header — **never construct auth headers manually**.

> A web-based computer vision platform that enables teams to annotate and manage datasets (images, video, 3D, medical), build and train neural networks, automate labeling with AI, and collaborate on mod

## Authentication

This integration uses **API Key** authentication via the `x-api-key` header.
Format: `x-api-key: ...`

## Required Credentials

- `SUPERVISELY_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
