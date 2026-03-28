---
name: dropbox
version: "1.0.0"
description: Dropbox API v2 — files, folders, sharing, search
activation:
  keywords:
    - "dropbox"
    - "dropbox file"
  exclude_keywords:
    - "google drive"
    - "box"
    - "onedrive"
  patterns:
    - "(?i)dropbox.*(file|folder|share|upload|download)"
  tags:
    - "file-storage"
    - "cloud-storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [DROPBOX_ACCESS_TOKEN]
---

# Dropbox API v2

Use the `http` tool. Credentials are automatically injected for `api.dropboxapi.com`.

## Base URL

`https://api.dropboxapi.com/2`

## Actions

**List folder contents:**
```
http(method="POST", url="https://api.dropboxapi.com/2/files/list_folder", body={"path": "", "limit": 20})
```
Use `""` for root, `/path/to/folder` for subfolders.

**Search files:**
```
http(method="POST", url="https://api.dropboxapi.com/2/files/search_v2", body={"query": "report", "options": {"max_results": 20, "file_status": "active"}})
```

**Get file metadata:**
```
http(method="POST", url="https://api.dropboxapi.com/2/files/get_metadata", body={"path": "/path/to/file.pdf"})
```

**Create folder:**
```
http(method="POST", url="https://api.dropboxapi.com/2/files/create_folder_v2", body={"path": "/New Folder", "autorename": false})
```

**Move file/folder:**
```
http(method="POST", url="https://api.dropboxapi.com/2/files/move_v2", body={"from_path": "/old/path.txt", "to_path": "/new/path.txt"})
```

**Delete file/folder:**
```
http(method="POST", url="https://api.dropboxapi.com/2/files/delete_v2", body={"path": "/path/to/delete"})
```

**Create shared link:**
```
http(method="POST", url="https://api.dropboxapi.com/2/sharing/create_shared_link_with_settings", body={"path": "/path/to/file.pdf", "settings": {"requested_visibility": "public"}})
```

**Get account info:**
```
http(method="POST", url="https://api.dropboxapi.com/2/users/get_current_account")
```

## Notes

- Dropbox API v2 uses POST for everything (even reads) with JSON bodies.
- Paths are lowercase, start with `/`. Root is empty string `""`.
- `list_folder` returns `has_more` + `cursor`. Use `list_folder/continue` with `cursor` for pagination.
- File downloads use `content.dropboxapi.com/2/files/download` with path in `Dropbox-API-Arg` header (not this API base).
- Errors: `{"error_summary": "path/not_found/...", "error": {...}}`.
