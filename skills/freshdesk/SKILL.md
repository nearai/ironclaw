---
name: freshdesk
version: "1.0.0"
description: Freshdesk API — Freshdesk is a cloud-based customer support platform that offers ticketing
activation:
  keywords:
    - "freshdesk"
    - "hr & payroll"
  patterns:
    - "(?i)freshdesk"
  tags:
    - "tools"
    - "Customer Support"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [FRESHDESK_DOMAIN, FRESHDESK_API_KEY]
---

# Freshdesk API

Use the `http` tool. Credentials are automatically injected.

## Base URL

`https://{FRESHDESK_DOMAIN}.freshdesk.com/api/v2`

## Actions

**List tickets:**
```
http(method="GET", url="https://{FRESHDESK_DOMAIN}.freshdesk.com/api/v2/tickets?per_page=10")
```

**Get ticket:**
```
http(method="GET", url="https://{FRESHDESK_DOMAIN}.freshdesk.com/api/v2/tickets/{ticket_id}")
```

**Create ticket:**
```
http(method="POST", url="https://{FRESHDESK_DOMAIN}.freshdesk.com/api/v2/tickets", body={"subject": "Issue with login","description": "Cannot log in since morning","email": "customer@example.com","priority": 2,"status": 2})
```

**Update ticket:**
```
http(method="PUT", url="https://{FRESHDESK_DOMAIN}.freshdesk.com/api/v2/tickets/{ticket_id}", body={"status": 4,"priority": 3})
```

**List contacts:**
```
http(method="GET", url="https://{FRESHDESK_DOMAIN}.freshdesk.com/api/v2/contacts?per_page=10")
```

## Notes

- Priority: 1=Low, 2=Medium, 3=High, 4=Urgent.
- Status: 2=Open, 3=Pending, 4=Resolved, 5=Closed.
- Uses API key as Basic auth username with `X` as password.
- Pagination: `page` param, 30 items per page by default.
