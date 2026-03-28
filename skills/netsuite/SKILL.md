---
name: netsuite
version: "1.0.0"
description: Netsuite API — NetSuite is a cloud-based ERP platform that helps businesses manage finance
activation:
  keywords:
    - "netsuite"
    - "accounting"
  patterns:
    - "(?i)netsuite"
  tags:
    - "accounting"
    - "finance"
    - "Accounting"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [NETSUITE_ACCOUNT_ID, CONSUMER_KEY, CONSUMER_SECRET, ACCESS_TOKEN_ID, ACCESS_TOKEN_SECRET]
---

# Netsuite API

Use the `http` tool. Credentials are automatically injected.

> NetSuite is a cloud-based ERP platform that helps businesses manage finance, operations, customer relationships, and eCommerce in a unified system for better efficiency and scalability.

## Authentication


## Required Credentials

- `NETSUITE_ACCOUNT_ID` — Account ID
- `CONSUMER_KEY` — Consumer Key
- `CONSUMER_SECRET` — Consumer Secret
- `ACCESS_TOKEN_ID` — Access Token ID
- `ACCESS_TOKEN_SECRET` — Access Token Secret

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
