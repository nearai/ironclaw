---
name: puzzle-io
version: "1.0.0"
description: Puzzle.io API — Puzzle.io is an AI‑native accounting platform that automates bookkeeping tasks l
activation:
  keywords:
    - "puzzle-io"
    - "puzzle.io"
    - "accounting"
  patterns:
    - "(?i)puzzle.?io"
  tags:
    - "accounting"
    - "finance"
    - "Accounting"
    - "ai"
  max_context_tokens: 1200
---

# Puzzle.io API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Puzzle.io is an AI‑native accounting platform that automates bookkeeping tasks like transaction categorization, reconciliations, and accruals, while providing real‑time financial insights, anomaly det

## Authentication

This integration uses **OAuth 2.0**. The token is managed automatically — no manual auth setup required in API calls.

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
