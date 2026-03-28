---
name: share-point
version: "1.0.0"
description: SharePoint API — Microsoft SharePoint is a web-based collaboration and content management platfor
activation:
  keywords:
    - "share-point"
    - "sharepoint"
    - "productivity"
  patterns:
    - "(?i)share.?point"
  tags:
    - "productivity"
    - "collaboration"
    - "news"
  max_context_tokens: 1200
---

# SharePoint API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://graph.microsoft.com/v1.0`

## Actions

**List sites:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/sites?search=*&$top=10")
```

**Get site:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/sites/{site_id}")
```

**List lists:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/sites/{site_id}/lists")
```

**List items:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/sites/{site_id}/lists/{list_id}/items?$top=10&$expand=fields")
```

**List files:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/sites/{site_id}/drive/root/children?$top=10")
```

## Notes

- Uses OAuth 2.0 via Microsoft Graph — credentials are auto-injected.
- Site ID format: `{hostname}:/{path}` or GUID.
- Lists contain structured data; drives contain files.
- `$expand=fields` includes column values in list items.
