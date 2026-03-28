---
name: job-nimbus
version: "1.0.0"
description: JobNimbus API — JobNimbus is an all-in-one CRM and project management platform tailored for cont
activation:
  keywords:
    - "job-nimbus"
    - "jobnimbus"
    - "crm"
  patterns:
    - "(?i)job.?nimbus"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [JOB_NIMBUS_API_KEY]
---

# JobNimbus API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> JobNimbus is an all-in-one CRM and project management platform tailored for contractors, combining lead tracking, custom job workflows, estimating, invoicing, mobile access, material ordering, integra

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `JOB_NIMBUS_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
