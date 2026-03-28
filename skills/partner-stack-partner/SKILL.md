---
name: partner-stack-partner
version: "1.0.0"
description: PartnerStack Partner API — PartnerStack is a partner relationship management platform that helps companies 
activation:
  keywords:
    - "partner-stack-partner"
    - "partnerstack partner"
    - "prm"
  patterns:
    - "(?i)partner.?stack.?partner"
  tags:
    - "tools"
    - "prm"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [PARTNERSTACK_PARTNER_API_KEY]
---

# PartnerStack Partner API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> PartnerStack is a partner relationship management platform that helps companies scale affiliate, referral, and reseller programs through automated tracking, payouts, and performance insights. The Part

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `PARTNERSTACK_PARTNER_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
