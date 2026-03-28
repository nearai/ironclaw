---
name: autodesk
version: "1.0.0"
description: Autodesk API — Autodesk is a global leader in design and engineering software
activation:
  keywords:
    - "autodesk"
    - "software"
  patterns:
    - "(?i)autodesk"
  tags:
    - "software"
    - "development"
    - "tools"
  max_context_tokens: 1200
---

# Autodesk API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> Autodesk is a global leader in design and engineering software, offering a cloud‑connected Design and Make Platform—including AutoCAD, Revit, Fusion 360, and Autodesk Platform Services—that connects d

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
