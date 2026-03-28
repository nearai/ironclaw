---
name: abstract-ip-intelligence
version: "1.0.0"
description: Abstract IP Intelligence API — An API that provides IP address data including geolocation, ISP
activation:
  keywords:
    - "abstract-ip-intelligence"
    - "abstract ip intelligence"
    - "ip intelligence"
  patterns:
    - "(?i)abstract.?ip.?intelligence"
  tags:
    - "tools"
    - "ip-intelligence"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ABSTRACT_IP_INTELLIGENCE_API_KEY]
---

# Abstract IP Intelligence API

Use the `http` tool. API key is automatically injected as `api_key` query parameter.

> An API that provides IP address data including geolocation, ISP, proxy and VPN detection, and risk signals to help applications enhance security, prevent fraud, and deliver location-aware experiences.

## Authentication

This integration uses **query parameter** authentication via `api_key`.

## Required Credentials

- `ABSTRACT_IP_INTELLIGENCE_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
