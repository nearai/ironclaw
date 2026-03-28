---
name: one-drive
version: "1.0.0"
description: OneDrive API — A cloud storage service that allows users to store, sync
activation:
  keywords:
    - "one-drive"
    - "onedrive"
    - "ai"
  patterns:
    - "(?i)one.?drive"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
---

# OneDrive API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://graph.microsoft.com/v1.0`

## Actions

**List root files:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/me/drive/root/children?$top=10")
```

**Get file metadata:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/me/drive/items/{item_id}")
```

**Search files:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/me/drive/root/search(q='report')?$top=10")
```

**Create folder:**
```
http(method="POST", url="https://graph.microsoft.com/v1.0/me/drive/root/children", body={"name": "New Folder","folder": {},"@microsoft.graph.conflictBehavior": "rename"})
```

**Upload small file:**
```
http(method="PUT", url="https://graph.microsoft.com/v1.0/me/drive/root:/Documents/file.txt:/content", body="file content here")
```

## Notes

- Uses OAuth 2.0 via Microsoft Graph — credentials are auto-injected.
- Item paths use colon syntax: `/me/drive/root:/path/to/file:`.
- Small files (<4MB) use simple PUT upload; larger need upload session.
- `$select`, `$filter`, `$orderby` OData query params supported.
