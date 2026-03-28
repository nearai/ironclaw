---
name: wttr-in
version: "1.0.0"
description: Wttr.in API — Wttr.in is a console-based weather forecast service that provides location-speci
activation:
  keywords:
    - "wttr-in"
    - "wttr.in"
    - "logistics"
  patterns:
    - "(?i)wttr.?in"
  tags:
    - "tools"
    - "weather"
  max_context_tokens: 1200
---

# Wttr.in API

Use the `http` tool. Credentials are automatically injected.

> Wttr.in is a console-based weather forecast service that provides location-specific weather updates in a minimal, text-based format. It’s especially popular among developers and system administrators 

## Authentication


## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
