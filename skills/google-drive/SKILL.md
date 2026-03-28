---
name: google-drive
version: "1.0.0"
description: Google Drive API — files, folders, search, permissions, upload
activation:
  keywords:
    - "google drive"
    - "drive"
    - "gdrive"
  exclude_keywords:
    - "dropbox"
    - "onedrive"
    - "box"
  patterns:
    - "(?i)(upload|download|share|search|list).*drive"
    - "(?i)google drive.*(file|folder|share)"
  tags:
    - "file-storage"
    - "google"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [GOOGLE_ACCESS_TOKEN]
---

# Google Drive API

Use the `http` tool. Credentials are automatically injected for `googleapis.com`.

## Base URL

`https://www.googleapis.com/drive/v3`

## Actions

**List files:**
```
http(method="GET", url="https://www.googleapis.com/drive/v3/files?pageSize=20&fields=files(id,name,mimeType,modifiedTime,size,parents)&orderBy=modifiedTime%20desc")
```

**Search files:**
```
http(method="GET", url="https://www.googleapis.com/drive/v3/files?q=name%20contains%20'report'%20and%20mimeType%20%21%3D%20'application/vnd.google-apps.folder'&pageSize=10&fields=files(id,name,mimeType,webViewLink)")
```

**Get file metadata:**
```
http(method="GET", url="https://www.googleapis.com/drive/v3/files/<file_id>?fields=id,name,mimeType,size,modifiedTime,webViewLink,parents")
```

**Download file content:**
```
http(method="GET", url="https://www.googleapis.com/drive/v3/files/<file_id>?alt=media", save_to="/tmp/downloaded_file")
```

**Create folder:**
```
http(method="POST", url="https://www.googleapis.com/drive/v3/files", body={"name": "New Folder", "mimeType": "application/vnd.google-apps.folder", "parents": ["<parent_folder_id>"]})
```

**Move file:**
```
http(method="PATCH", url="https://www.googleapis.com/drive/v3/files/<file_id>?addParents=<new_folder_id>&removeParents=<old_folder_id>")
```

**Share file (add permission):**
```
http(method="POST", url="https://www.googleapis.com/drive/v3/files/<file_id>/permissions", body={"role": "reader", "type": "user", "emailAddress": "alice@example.com"})
```

**Delete file:**
```
http(method="DELETE", url="https://www.googleapis.com/drive/v3/files/<file_id>")
```

## Search Query Syntax

- `name contains 'report'` — filename search
- `mimeType = 'application/vnd.google-apps.spreadsheet'` — by type
- `'<folder_id>' in parents` — files in folder
- `modifiedTime > '2026-01-01T00:00:00'` — date filter
- `trashed = false` — exclude trash

## Google MIME Types

| Type | MIME |
|------|------|
| Folder | `application/vnd.google-apps.folder` |
| Doc | `application/vnd.google-apps.document` |
| Sheet | `application/vnd.google-apps.spreadsheet` |
| Slide | `application/vnd.google-apps.presentation` |

## Notes

- Use `fields` param to request specific properties (saves quota).
- `parents` is an array of folder IDs. Root folder is `"root"`.
- Permission roles: `reader`, `commenter`, `writer`, `organizer`.
- Pagination: use `pageToken` from `nextPageToken`.
