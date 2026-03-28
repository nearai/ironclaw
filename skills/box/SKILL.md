---
name: box
version: "1.0.0"
description: Box API — files, folders, collaborations, search, comments
activation:
  keywords:
    - "box"
    - "box.com"
    - "box file"
  exclude_keywords:
    - "dropbox"
    - "google drive"
  patterns:
    - "(?i)box\\.com.*(file|folder|share)"
    - "(?i)(upload|download|share).*box"
  tags:
    - "file-storage"
    - "cloud-storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BOX_ACCESS_TOKEN]
---

# Box Content API

Use the `http` tool. Credentials are automatically injected for `api.box.com`.

## Base URL

`https://api.box.com/2.0`

## Actions

**Get folder items:**
```
http(method="GET", url="https://api.box.com/2.0/folders/<folder_id>/items?fields=id,name,type,modified_at,size&limit=20")
```
Root folder ID is `"0"`.

**Get file info:**
```
http(method="GET", url="https://api.box.com/2.0/files/<file_id>?fields=id,name,size,modified_at,shared_link")
```

**Search:**
```
http(method="GET", url="https://api.box.com/2.0/search?query=report&type=file&limit=20")
```

**Create folder:**
```
http(method="POST", url="https://api.box.com/2.0/folders", body={"name": "New Folder", "parent": {"id": "0"}})
```

**Copy file:**
```
http(method="POST", url="https://api.box.com/2.0/files/<file_id>/copy", body={"parent": {"id": "<folder_id>"}})
```

**Create shared link:**
```
http(method="PUT", url="https://api.box.com/2.0/files/<file_id>?fields=shared_link", body={"shared_link": {"access": "open"}})
```

**Add collaboration:**
```
http(method="POST", url="https://api.box.com/2.0/collaborations", body={"item": {"type": "folder", "id": "<folder_id>"}, "accessible_by": {"type": "user", "login": "user@example.com"}, "role": "editor"})
```

**Add comment:**
```
http(method="POST", url="https://api.box.com/2.0/comments", body={"item": {"type": "file", "id": "<file_id>"}, "message": "Please review"})
```

## Notes

- Root folder is always `"0"`.
- Collaboration roles: `editor`, `viewer`, `previewer`, `uploader`, `co-owner`.
- Shared link access: `open` (anyone), `company` (organization), `collaborators` (invited only).
- Use `fields` param to request specific properties.
- Pagination: `offset` + `limit`. Check `total_count`.
- File uploads use `https://upload.box.com/api/2.0/files/content` (different host).
