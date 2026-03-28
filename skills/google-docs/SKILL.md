---
name: google-docs
version: "1.0.0"
description: Google Docs API — Google Docs is a cloud-based word processing platform that allows users to creat
activation:
  keywords:
    - "google-docs"
    - "google docs"
    - "tools"
  patterns:
    - "(?i)google.?docs"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
---

# Google Docs API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://docs.googleapis.com/v1`

## Actions

**Get document:**
```
http(method="GET", url="https://docs.googleapis.com/v1/documents/{document_id}")
```

**Create document:**
```
http(method="POST", url="https://docs.googleapis.com/v1/documents", body={"title": "My Document"})
```

**Update document:**
```
http(method="POST", url="https://docs.googleapis.com/v1/documents/{document_id}:batchUpdate", body={"requests": [{"insertText": {"location": {"index": 1},"text": "Hello World"}}]})
```

## Notes

- Uses OAuth 2.0 — credentials are auto-injected.
- Document ID from URL: `docs.google.com/document/d/{id}/edit`.
- Updates use batch requests with operations like `insertText`, `deleteContentRange`, `updateTextStyle`.
- Index positions are 1-based (body starts at index 1).
