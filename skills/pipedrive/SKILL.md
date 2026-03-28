---
name: pipedrive
version: "1.0.0"
description: Pipedrive CRM API — deals, persons, organizations, activities, notes
activation:
  keywords:
    - "pipedrive"
    - "pipedrive deal"
    - "pipeline"
  exclude_keywords:
    - "hubspot"
    - "salesforce"
  patterns:
    - "(?i)pipedrive.*(deal|person|organization|activity)"
  tags:
    - "crm"
    - "sales"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [PIPEDRIVE_API_TOKEN, PIPEDRIVE_DOMAIN]
---

# Pipedrive API

Use the `http` tool. Auth uses `api_token` query parameter.

## Base URL

`https://{PIPEDRIVE_DOMAIN}.pipedrive.com/api/v1`

Append `?api_token={PIPEDRIVE_API_TOKEN}` to all URLs.

## Actions

**List deals:**
```
http(method="GET", url="https://{domain}.pipedrive.com/api/v1/deals?api_token={token}&status=open&sort=add_time%20DESC&limit=20")
```

**Create deal:**
```
http(method="POST", url="https://{domain}.pipedrive.com/api/v1/deals?api_token={token}", body={"title": "New Deal", "value": 5000, "currency": "USD", "person_id": 123, "org_id": 456, "stage_id": 1})
```

**Update deal:**
```
http(method="PUT", url="https://{domain}.pipedrive.com/api/v1/deals/<deal_id>?api_token={token}", body={"status": "won", "won_time": "2026-03-27 10:00:00"})
```

**List persons:**
```
http(method="GET", url="https://{domain}.pipedrive.com/api/v1/persons?api_token={token}&limit=20")
```

**Create person:**
```
http(method="POST", url="https://{domain}.pipedrive.com/api/v1/persons?api_token={token}", body={"name": "John Doe", "email": [{"value": "john@acme.com", "primary": true}], "phone": [{"value": "+1234567890", "primary": true}], "org_id": 456})
```

**Search:**
```
http(method="GET", url="https://{domain}.pipedrive.com/api/v1/itemSearch?api_token={token}&term=acme&item_types=deal,person,organization&limit=10")
```

**Add activity:**
```
http(method="POST", url="https://{domain}.pipedrive.com/api/v1/activities?api_token={token}", body={"subject": "Follow-up call", "type": "call", "due_date": "2026-04-01", "deal_id": 789, "person_id": 123})
```

**Add note:**
```
http(method="POST", url="https://{domain}.pipedrive.com/api/v1/notes?api_token={token}", body={"content": "Discussed pricing", "deal_id": 789})
```

## Notes

- All responses: `{"success": true, "data": {...}}` or `{"success": false, "error": "..."}`.
- Deal status: `open`, `won`, `lost`, `deleted`.
- Email/phone fields are arrays of `{"value": "...", "primary": true/false}`.
- Pagination: `start` + `limit` params. Check `additional_data.pagination.more_items_in_collection`.
- Activity types: `call`, `meeting`, `task`, `deadline`, `email`, `lunch`.
