---
name: activecampaign
version: "1.0.0"
description: ActiveCampaign API — contacts, deals, automations, campaigns, lists
activation:
  keywords:
    - "activecampaign"
    - "active campaign"
    - "marketing automation"
  exclude_keywords:
    - "hubspot"
    - "mailchimp"
  patterns:
    - "(?i)active.?campaign.*(contact|deal|automation|campaign)"
  tags:
    - "marketing"
    - "crm"
    - "automation"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [ACTIVECAMPAIGN_URL, ACTIVECAMPAIGN_API_KEY]
---

# ActiveCampaign API v3

Use the `http` tool. Include `Api-Token` header.

## Base URL

`https://{ACTIVECAMPAIGN_URL}.api-us1.com/api/3`

## Actions

**List contacts:**
```
http(method="GET", url="https://{account}.api-us1.com/api/3/contacts?limit=20", headers=[{"name": "Api-Token", "value": "{ACTIVECAMPAIGN_API_KEY}"}])
```

**Search contacts:**
```
http(method="GET", url="https://{account}.api-us1.com/api/3/contacts?search=john@example.com", headers=[{"name": "Api-Token", "value": "{ACTIVECAMPAIGN_API_KEY}"}])
```

**Create contact:**
```
http(method="POST", url="https://{account}.api-us1.com/api/3/contacts", headers=[{"name": "Api-Token", "value": "{ACTIVECAMPAIGN_API_KEY}"}], body={"contact": {"email": "john@example.com", "firstName": "John", "lastName": "Doe", "phone": "+1234567890"}})
```

**Update contact:**
```
http(method="PUT", url="https://{account}.api-us1.com/api/3/contacts/<contact_id>", headers=[{"name": "Api-Token", "value": "{ACTIVECAMPAIGN_API_KEY}"}], body={"contact": {"firstName": "Updated"}})
```

**List deals:**
```
http(method="GET", url="https://{account}.api-us1.com/api/3/deals?limit=20", headers=[{"name": "Api-Token", "value": "{ACTIVECAMPAIGN_API_KEY}"}])
```

**Create deal:**
```
http(method="POST", url="https://{account}.api-us1.com/api/3/deals", headers=[{"name": "Api-Token", "value": "{ACTIVECAMPAIGN_API_KEY}"}], body={"deal": {"title": "New Deal", "value": 5000, "currency": "usd", "contact": "<contact_id>", "group": "<pipeline_id>", "stage": "<stage_id>"}})
```

**Add contact to automation:**
```
http(method="POST", url="https://{account}.api-us1.com/api/3/contactAutomations", headers=[{"name": "Api-Token", "value": "{ACTIVECAMPAIGN_API_KEY}"}], body={"contactAutomation": {"contact": "<contact_id>", "automation": "<automation_id>"}})
```

**List automations:**
```
http(method="GET", url="https://{account}.api-us1.com/api/3/automations?limit=20", headers=[{"name": "Api-Token", "value": "{ACTIVECAMPAIGN_API_KEY}"}])
```

## Notes

- All request/response bodies wrap data in the resource name: `{"contact": {...}}`, `{"contacts": [...]}`.
- Deal values are in cents (integer).
- Pagination: `limit` + `offset`. Check `meta.total` for count.
- Tags can be added via `/api/3/contactTags` with `{"contactTag": {"contact": "id", "tag": "id"}}`.
