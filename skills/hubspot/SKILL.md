---
name: hubspot
version: "1.0.0"
description: HubSpot CRM API — contacts, companies, deals, tickets, notes
activation:
  keywords:
    - "hubspot"
    - "hubspot contact"
    - "hubspot deal"
    - "crm"
  exclude_keywords:
    - "salesforce"
    - "pipedrive"
  patterns:
    - "(?i)hubspot.*(contact|company|deal|ticket)"
    - "(?i)(create|update|search).*hubspot"
  tags:
    - "crm"
    - "sales"
    - "marketing"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [HUBSPOT_ACCESS_TOKEN]
---

# HubSpot CRM API

Use the `http` tool. Credentials are automatically injected for `api.hubapi.com`.

## Base URL

`https://api.hubapi.com`

## Actions

**Search contacts:**
```
http(method="POST", url="https://api.hubapi.com/crm/v3/objects/contacts/search", body={"filterGroups": [{"filters": [{"propertyName": "email", "operator": "CONTAINS_TOKEN", "value": "acme.com"}]}], "properties": ["firstname", "lastname", "email", "phone"], "limit": 10})
```

**Get contact:**
```
http(method="GET", url="https://api.hubapi.com/crm/v3/objects/contacts/<contact_id>?properties=firstname,lastname,email,phone,company")
```

**Create contact:**
```
http(method="POST", url="https://api.hubapi.com/crm/v3/objects/contacts", body={"properties": {"firstname": "John", "lastname": "Doe", "email": "john@acme.com", "company": "Acme Corp"}})
```

**Update contact:**
```
http(method="PATCH", url="https://api.hubapi.com/crm/v3/objects/contacts/<contact_id>", body={"properties": {"phone": "+1234567890"}})
```

**List deals:**
```
http(method="GET", url="https://api.hubapi.com/crm/v3/objects/deals?properties=dealname,amount,dealstage,closedate&limit=20")
```

**Create deal:**
```
http(method="POST", url="https://api.hubapi.com/crm/v3/objects/deals", body={"properties": {"dealname": "Big Deal", "amount": "50000", "dealstage": "qualifiedtobuy", "closedate": "2026-06-30"}})
```

**Create note:**
```
http(method="POST", url="https://api.hubapi.com/crm/v3/objects/notes", body={"properties": {"hs_note_body": "Called John, discussed pricing", "hs_timestamp": "2026-03-27T10:00:00.000Z"}, "associations": [{"to": {"id": "<contact_id>"}, "types": [{"associationCategory": "HUBSPOT_DEFINED", "associationTypeId": 202}]}]})
```

**List companies:**
```
http(method="GET", url="https://api.hubapi.com/crm/v3/objects/companies?properties=name,domain,industry&limit=20")
```

## Notes

- All CRM objects follow the same pattern: `/crm/v3/objects/{objectType}`.
- Object types: `contacts`, `companies`, `deals`, `tickets`, `notes`.
- Properties are always passed as flat string key-value pairs.
- Search operators: `EQ`, `NEQ`, `LT`, `GT`, `CONTAINS_TOKEN`, `HAS_PROPERTY`.
- Pagination: use `after` from `paging.next.after` in response.
- Association type IDs: 1=Contact→Company, 3=Deal→Contact, 202=Note→Contact.
