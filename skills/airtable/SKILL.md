---
name: airtable
version: "1.0.0"
description: Airtable API — bases, tables, records, fields, views
activation:
  keywords:
    - "airtable"
    - "airtable base"
    - "airtable record"
  exclude_keywords:
    - "google sheets"
    - "notion"
  patterns:
    - "(?i)airtable.*(base|table|record|field)"
    - "(?i)(create|list|update).*airtable"
  tags:
    - "database"
    - "spreadsheet"
    - "productivity"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [AIRTABLE_ACCESS_TOKEN]
---

# Airtable Web API

Use the `http` tool. Credentials are automatically injected for `api.airtable.com`.

## Base URL

`https://api.airtable.com/v0`

## Actions

**List bases:**
```
http(method="GET", url="https://api.airtable.com/v0/meta/bases")
```

**List tables in base:**
```
http(method="GET", url="https://api.airtable.com/v0/meta/bases/<base_id>/tables")
```

**List records:**
```
http(method="GET", url="https://api.airtable.com/v0/<base_id>/<table_name>?maxRecords=20&view=Grid%20view")
```

**List with filter:**
```
http(method="GET", url="https://api.airtable.com/v0/<base_id>/<table_name>?filterByFormula={Status}='Active'&sort[0][field]=Name&sort[0][direction]=asc&maxRecords=20")
```

**Get record:**
```
http(method="GET", url="https://api.airtable.com/v0/<base_id>/<table_name>/<record_id>")
```

**Create records:**
```
http(method="POST", url="https://api.airtable.com/v0/<base_id>/<table_name>", body={"records": [{"fields": {"Name": "Alice", "Email": "alice@example.com", "Status": "Active"}}]})
```

**Update records:**
```
http(method="PATCH", url="https://api.airtable.com/v0/<base_id>/<table_name>", body={"records": [{"id": "<record_id>", "fields": {"Status": "Done"}}]})
```

**Delete records:**
```
http(method="DELETE", url="https://api.airtable.com/v0/<base_id>/<table_name>?records[]=<record_id1>&records[]=<record_id2>")
```

## Notes

- Base IDs start with `app`, table IDs with `tbl`, record IDs with `rec`.
- Table name in URL must be URL-encoded if it contains spaces.
- `filterByFormula` uses Airtable formula syntax: `{Field Name}='value'`, `AND(...)`, `OR(...)`.
- Create/update accept up to 10 records per request.
- Pagination: use `offset` from response. When absent, no more records.
- Rate limit: 5 requests/second per base.
- Field names are case-sensitive and must match exactly.
