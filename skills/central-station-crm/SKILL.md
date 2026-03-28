---
name: central-station-crm
version: "1.0.0"
description: Central Station CRM API — CentralStationCRM is a lightweight CRM designed for small businesses to manage c
activation:
  keywords:
    - "central-station-crm"
    - "central station crm"
    - "crm"
  patterns:
    - "(?i)central.?station.?crm"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CENTRAL_STATION_CRM_API_KEY]
---

# Central Station CRM API

Use the `http` tool. API key is automatically injected via `X-apikey` header — **never construct auth headers manually**.

> CentralStationCRM is a lightweight CRM designed for small businesses to manage contacts, track deals, organize tasks and streamline customer relationships with a simple, user-friendly interface.

## Authentication

This integration uses **API Key** authentication via the `X-apikey` header.
Format: `X-apikey: ...`

## Required Credentials

- `CENTRAL_STATION_CRM_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
