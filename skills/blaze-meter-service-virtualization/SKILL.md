---
name: blaze-meter-service-virtualization
version: "1.0.0"
description: BlazeMeter Service Virtualization API — BlazeMeter is a testing platform for web and API applications
activation:
  keywords:
    - "blaze-meter-service-virtualization"
    - "blazemeter service virtualization"
    - "developer tool"
  patterns:
    - "(?i)blaze.?meter.?service.?virtualization"
  tags:
    - "tools"
    - "developer-tool"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BLAZE_METER_SERVICE_VIRTUALIZATION_API_KEY, BLAZE_METER_SERVICE_VIRTUALIZATION_API_SECRET]
---

# BlazeMeter Service Virtualization API

Use the `http` tool. Credentials are automatically injected.

> BlazeMeter is a testing platform for web and API applications, and its Service Virtualization API enables developers to programmatically create and manage virtual services that simulate APIs and syste

## Authentication


## Required Credentials

- `BLAZE_METER_SERVICE_VIRTUALIZATION_API_KEY` — API Key
- `BLAZE_METER_SERVICE_VIRTUALIZATION_API_SECRET` — API Secret

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
