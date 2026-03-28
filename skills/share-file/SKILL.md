---
name: share-file
version: "1.0.0"
description: ShareFile API — ShareFile is a secure file sharing and content collaboration solution that suppo
activation:
  keywords:
    - "share-file"
    - "sharefile"
    - "storage"
  patterns:
    - "(?i)share.?file"
  tags:
    - "storage"
    - "files"
    - "documents"
  max_context_tokens: 1200
---

# ShareFile API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

> ShareFile is a secure file sharing and content collaboration solution that supports workflows for document storage, e-signatures, and encrypted file transfer in professional environments.

## Authentication

This integration uses **OAuth 2.0**. The token is managed automatically — no manual auth setup required in API calls.

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
