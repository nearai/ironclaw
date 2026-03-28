---
name: blaze-meter-functional
version: "1.0.0"
description: BlazeMeter Functional API — BlazeMeter is a testing platform for web and API applications
activation:
  keywords:
    - "blaze-meter-functional"
    - "blazemeter functional"
    - "developer tool"
  patterns:
    - "(?i)blaze.?meter.?functional"
  tags:
    - "tools"
    - "developer-tool"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BLAZE_METER_FUNCTIONAL_API_KEY, BLAZE_METER_FUNCTIONAL_API_SECRET, BLAZE_METER_ACCOUNT_ID]
---

# BlazeMeter Functional API

Use the `http` tool. Credentials are automatically injected.

> BlazeMeter is a testing platform for web and API applications, and its Functional API enables developers to programmatically create, manage, and run automated API tests to validate responses, workflow

## Authentication


## Required Credentials

- `BLAZE_METER_FUNCTIONAL_API_KEY` — API Key
- `BLAZE_METER_FUNCTIONAL_API_SECRET` — API Secret
- `BLAZE_METER_ACCOUNT_ID` — Account ID

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
