---
name: quick-chart
version: "1.0.0"
description: QuickChart API — QuickChart is a web service that generates chart images using Chart.js
activation:
  keywords:
    - "quick-chart"
    - "quickchart"
    - "tools"
  patterns:
    - "(?i)quick.?chart"
  tags:
    - "tools"
    - "utility"
    - "tool"
  max_context_tokens: 1200
---

# QuickChart API

Use the `http` tool. Credentials are automatically injected.

> QuickChart is a web service that generates chart images using Chart.js. Ideal for emails, reports, or static sites, it supports various chart types, QR codes, and more. Charts are created via simple U

## Authentication


## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
