---
name: one-page-crm
version: "1.0.0"
description: OnePageCRM API — OnePageCRM is an action-focused CRM designed to help small and medium-sized busi
activation:
  keywords:
    - "one-page-crm"
    - "onepagecrm"
    - "crm"
  patterns:
    - "(?i)one.?page.?crm"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ONE_PAGE_CRM_USER_ID, ONE_PAGE_CRM_API_KEY]
---

# OnePageCRM API

Use the `http` tool. Credentials are automatically injected.

> OnePageCRM is an action-focused CRM designed to help small and medium-sized businesses manage contacts, track follow-ups and drive deals by turning each lead into a prioritized next action.

## Authentication


## Required Credentials

- `ONE_PAGE_CRM_USER_ID` — User ID
- `ONE_PAGE_CRM_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
