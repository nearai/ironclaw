---
name: salesmate
version: "1.0.0"
description: Salesmate API — Salesmate is an AI-powered CRM platform that helps businesses automate sales
activation:
  keywords:
    - "salesmate"
    - "crm"
  patterns:
    - "(?i)salesmate"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SALESMATE_ACCESS_TOKEN, SALESMATE_DOMAIN]
---

# Salesmate API

Use the `http` tool. API key is automatically injected via `accessToken` header — **never construct auth headers manually**.

## Base URL

`https://{SALESMATE_DOMAIN}.salesmate.io/apis/v3`

## Actions

**List contacts:**
```
http(method="GET", url="https://{SALESMATE_DOMAIN}.salesmate.io/apis/v3/contacts?rows=10")
```

**Create contact:**
```
http(method="POST", url="https://{SALESMATE_DOMAIN}.salesmate.io/apis/v3/contacts", body={"firstName": "John","lastName": "Doe","email": "john@example.com"})
```

**List deals:**
```
http(method="GET", url="https://{SALESMATE_DOMAIN}.salesmate.io/apis/v3/deals?rows=10")
```

**Create deal:**
```
http(method="POST", url="https://{SALESMATE_DOMAIN}.salesmate.io/apis/v3/deals", body={"title": "Big Deal","primaryContact": "contact_id","pipeline": "default"})
```

## Notes

- Auth via `sessionToken` header (auto-injected).
- Entities: contacts, companies, deals, activities.
- Pagination: `rows` and `from` params.
- Custom fields available on all entities.
