---
name: gorgias
version: "1.0.0"
description: Gorgias API — Gorgias is a conversational AI platform built for e-commerce brands that central
activation:
  keywords:
    - "gorgias"
    - "crm"
  patterns:
    - "(?i)gorgias"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [GORGIAS_DOMAIN]
---

# Gorgias API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://{GORGIAS_DOMAIN}.gorgias.com/api`

## Actions

**List tickets:**
```
http(method="GET", url="https://{GORGIAS_DOMAIN}.gorgias.com/api/tickets?limit=10&order_by=created_datetime")
```

**Get ticket:**
```
http(method="GET", url="https://{GORGIAS_DOMAIN}.gorgias.com/api/tickets/{ticket_id}")
```

**Create ticket:**
```
http(method="POST", url="https://{GORGIAS_DOMAIN}.gorgias.com/api/tickets", body={"customer": {"email": "customer@example.com"},"messages": [{"channel": "email","via": "api","body_text": "Need help with order"}]})
```

**List customers:**
```
http(method="GET", url="https://{GORGIAS_DOMAIN}.gorgias.com/api/customers?limit=10")
```

## Notes

- Uses Basic auth with email and API key.
- Ticket channels: `email`, `chat`, `facebook`, `instagram`, `phone`.
- Messages are nested within tickets.
- Pagination: `cursor` param from response.
