---
name: reducto
version: "1.0.0"
description: Reducto API — Reducto is an AI-powered document ingestion API that transforms complex
activation:
  keywords:
    - "reducto"
    - "ai"
  patterns:
    - "(?i)reducto"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [REDUCTO_API_KEY]
---

# Reducto API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> Reducto is an AI-powered document ingestion API that transforms complex, unstructured documents—such as PDFs, images, and spreadsheets—into structured, LLM-ready data with exceptional accuracy.

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `REDUCTO_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
