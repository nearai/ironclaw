---
name: blaze-meter-performance
version: "1.0.0"
description: BlazeMeter Performance API — BlazeMeter is a testing platform for web and API applications
activation:
  keywords:
    - "blaze-meter-performance"
    - "blazemeter performance"
    - "developer tool"
  patterns:
    - "(?i)blaze.?meter.?performance"
  tags:
    - "tools"
    - "developer-tool"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BLAZE_METER_PERFORMANCE_API_KEY, BLAZE_METER_PERFORMANCE_API_SECRET]
---

# BlazeMeter Performance API

Use the `http` tool. Credentials are automatically injected.

> BlazeMeter is a testing platform for web and API applications, and its Performance API enables developers to programmatically create, configure, run, and retrieve results from large-scale performance 

## Authentication


## Required Credentials

- `BLAZE_METER_PERFORMANCE_API_KEY` — API Key
- `BLAZE_METER_PERFORMANCE_API_SECRET` — API Secret

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
