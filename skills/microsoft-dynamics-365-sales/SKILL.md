---
name: microsoft-dynamics-365-sales
version: "1.0.0"
description: Microsoft Dynamics 365 Sales API — A sales automation platform that empowers teams to build stronger customer relat
activation:
  keywords:
    - "microsoft-dynamics-365-sales"
    - "microsoft dynamics 365 sales"
    - "crm"
  patterns:
    - "(?i)microsoft.?dynamics.?365.?sales"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [MICROSOFT_DYNAMICS_365_SALES_ORGANIZATION_URI, MICROSOFT_DYNAMICS_365_SALES_TENANT_ID]
---

# Microsoft Dynamics 365 Sales API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://{DYNAMICS_ORG}.crm.dynamics.com/api/data/v9.2`

## Actions

**List accounts:**
```
http(method="GET", url="https://{DYNAMICS_ORG}.crm.dynamics.com/api/data/v9.2/accounts?$top=10&$select=name,revenue")
```

**List contacts:**
```
http(method="GET", url="https://{DYNAMICS_ORG}.crm.dynamics.com/api/data/v9.2/contacts?$top=10&$select=fullname,emailaddress1")
```

**List opportunities:**
```
http(method="GET", url="https://{DYNAMICS_ORG}.crm.dynamics.com/api/data/v9.2/opportunities?$top=10&$select=name,estimatedvalue")
```

**Create contact:**
```
http(method="POST", url="https://{DYNAMICS_ORG}.crm.dynamics.com/api/data/v9.2/contacts", body={"firstname": "John","lastname": "Doe","emailaddress1": "john@example.com"})
```

## Notes

- Uses OAuth 2.0 — credentials are auto-injected.
- OData query params: `$filter`, `$select`, `$expand`, `$top`, `$orderby`.
- Entities: `accounts`, `contacts`, `opportunities`, `leads`, `incidents` (cases).
- Entity names are plural lowercase.
