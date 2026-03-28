---
name: bookingmood
version: "1.0.0"
description: Bookingmood API — A flexible booking platform that lets rental and vacation property owners embed 
activation:
  keywords:
    - "bookingmood"
    - "booking software"
  patterns:
    - "(?i)bookingmood"
  tags:
    - "tools"
    - "booking-software"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BOOKINGMOOD_API_KEY]
---

# Bookingmood API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

> A flexible booking platform that lets rental and vacation property owners embed customizable calendars on their websites, manage availability and reservations, track payments, sync with external calen

## Authentication

This integration uses **Bearer Token** authentication. The token is injected automatically into the `Authorization` header.

## Required Credentials

- `BOOKINGMOOD_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
