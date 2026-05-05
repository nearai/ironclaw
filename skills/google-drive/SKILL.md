---
name: google-drive
version: "1.0.0"
description: Search, access, upload, share, and organize files and folders in Google Drive via HTTP tool with automatic OAuth credential injection
activation:
  keywords:
    - "drive"
    - "google drive"
    - "file"
    - "folder"
    - "upload"
    - "share file"
  patterns:
    - "(?i)(upload|share|find|search).*(drive|file|folder)"
    - "(?i)google drive"
  tags:
    - "productivity"
    - "storage"
    - "google"
  max_context_tokens: 2000
credentials:
  - name: google_oauth_token
    provider: google
    location:
      type: bearer
    hosts:
      - "www.googleapis.com"
    oauth:
      authorization_url: "https://accounts.google.com/o/oauth2/v2/auth"
      token_url: "https://oauth2.googleapis.com/token"
      client_id_env: GOOGLE_OAUTH_CLIENT_ID
      client_secret_env: GOOGLE_OAUTH_CLIENT_SECRET
      scopes:
        - "https://www.googleapis.com/auth/drive"
      extra_params:
        access_type: "offline"
        prompt: "consent"
    setup_instructions: "Configure Google OAuth credentials at console.cloud.google.com/apis/credentials"
http:
  allowed_hosts:
    - "www.googleapis.com"
---

# Google Drive Skill

You have access to the Google Drive API via the `http` tool. Credentials are automatically injected — **never construct `Authorization` headers manually**.

All Google tools share the same `google_oauth_token`.

## API Patterns

Base URL: `https://www.googleapis.com/drive/v3`

### List/search files

```
http(method="GET", url="https://www.googleapis.com/drive/v3/files?q=name contains 'report' and trashed = false&pageSize=25&fields=files(id,name,mimeType,webViewLink)")
```

- `q`: Drive query syntax — `name contains 'text'`, `mimeType = 'application/vnd.google-apps.document'`, `'folderId' in parents`, `starred = true`, `trashed = false`
- `pageSize`: default 25, max 1000
- `fields`: Partial response to reduce bandwidth
- `orderBy`: `modifiedTime desc`, `name`, `recency`
- `corpora`: `user` (default), `drive`, `domain`, `allDrives`
- `includeItemsFromAllDrives=true`: Include shared drives
- `pageToken`: For pagination

### Get file metadata

```
http(method="GET", url="https://www.googleapis.com/drive/v3/files/{fileId}?fields=id,name,mimeType,size,webViewLink,parents")
```

### Download a file

For Google Docs formats, export first:
```
http(method="GET", url="https://www.googleapis.com/drive/v3/files/{fileId}/export?mimeType=text/plain")
```

For non-Google files (PDFs, images, etc.):
```
http(method="GET", url="https://www.googleapis.com/drive/v3/files/{fileId}?alt=media")
```

Use `save_to` parameter on the `http` tool for binary downloads.

### Upload a file

Simple upload (files < 5MB):
```
http(method="POST", url="https://www.googleapis.com/upload/drive/v3/files?uploadType=media", headers=[{"name": "Content-Type", "value": "text/plain"}], body={"name": "notes.txt", "content": "file content here", "parents": ["folderId"]})
```

For metadata + content, use multipart. For large files, use resumable uploads.

### Create a folder

```
http(method="POST", url="https://www.googleapis.com/drive/v3/files", body={"name": "New Folder", "mimeType": "application/vnd.google-apps.folder", "parents": ["parentFolderId"]})
```

### Update file metadata

```
http(method="PATCH", url="https://www.googleapis.com/drive/v3/files/{fileId}", body={"name": "Renamed File", "starred": true})
```

### Trash/delete a file

**Trash (recoverable):**
```
http(method="PATCH", url="https://www.googleapis.com/drive/v3/files/{fileId}", body={"trashed": true})
```

**Permanent delete:**
```
http(method="DELETE", url="https://www.googleapis.com/drive/v3/files/{fileId}")
```

### Share a file

```
http(method="POST", url="https://www.googleapis.com/drive/v3/files/{fileId}/permissions", body={"role": "reader", "type": "user", "emailAddress": "user@example.com"})
```

- `role`: `reader`, `commenter`, `writer`, `organizer`
- Send a message with the share: add `"sendNotificationEmail": true, "emailMessage": "Here's the file"`

### List shared drives

```
http(method="GET", url="https://www.googleapis.com/drive/v3/drives?pageSize=25")
```

## Common Mistakes

- Do NOT add an `Authorization` header — it is injected automatically.
- Always include `trashed = false` in search queries to exclude trashed files.
- `parents` is an array — use `["folderId"]` not just `"folderId"`.
- Google Docs have special MIME types: `application/vnd.google-apps.document` (Docs), `application/vnd.google-apps.spreadsheet` (Sheets), `application/vnd.google-apps.presentation` (Slides). These must be exported, not downloaded directly.
