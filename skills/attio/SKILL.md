---
name: attio
version: "1.0.0"
description: Attio API — Attio is a modern CRM platform that offers fully customizable workspaces
activation:
  keywords:
    - "attio"
    - "crm"
  patterns:
    - "(?i)attio"
  tags:
    - "crm"
    - "sales"
    - "contacts"
    - "CRM"
  max_context_tokens: 1200
---

# Attio API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.attio.com/v2`

## Actions

**List records:**
```
http(method="POST", url="https://api.attio.com/v2/objects/{object_slug}/records/query", body={"limit": 20})
```

**Get record:**
```
http(method="GET", url="https://api.attio.com/v2/objects/{object_slug}/records/{record_id}")
```

**Create record:**
```
http(method="POST", url="https://api.attio.com/v2/objects/{object_slug}/records", body={"data": {"values": {"name": [{"value": "Acme Corp"}]}}})
```

**List objects:**
```
http(method="GET", url="https://api.attio.com/v2/objects")
```

**List lists:**
```
http(method="GET", url="https://api.attio.com/v2/lists")
```

## Notes

- Standard objects: `companies`, `people`, `deals`.
- Values are arrays of typed entries: `[{"value": "..."}]`.
- Use `/objects/{slug}/records/query` with filters for searching.
