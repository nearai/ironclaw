---
name: zoho
version: "1.0.0"
description: Zoho API — Zoho offers a suite of cloud-based applications for businesses, including CRM
activation:
  keywords:
    - "zoho"
    - "crm"
  patterns:
    - "(?i)zoho"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
---

# Zoho API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://www.zohoapis.com/crm/v2`

## Actions

**List records:**
```
http(method="GET", url="https://www.zohoapis.com/crm/v2/Leads?per_page=10")
```

**Get record:**
```
http(method="GET", url="https://www.zohoapis.com/crm/v2/Leads/{record_id}")
```

**Create record:**
```
http(method="POST", url="https://www.zohoapis.com/crm/v2/Leads", body={"data": [{"Last_Name": "Doe","First_Name": "John","Email": "john@example.com","Company": "Acme"}]})
```

**Search records:**
```
http(method="GET", url="https://www.zohoapis.com/crm/v2/Leads/search?criteria=(Email:equals:john@example.com)")
```

## Notes

- Uses OAuth 2.0 — credentials are auto-injected.
- Modules: `Leads`, `Contacts`, `Accounts`, `Deals`, `Tasks`, `Meetings`.
- Data wrapped in `{"data": [...]}`.
- Search criteria: `(Field:operator:value)`. Operators: `equals`, `starts_with`, `contains`.
