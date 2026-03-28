---
name: coda
version: "1.0.0"
description: Coda API — Coda is an all-in-one collaborative workspace that blends the flexibility of doc
activation:
  keywords:
    - "coda"
    - "productivity"
  patterns:
    - "(?i)coda"
  tags:
    - "productivity"
    - "collaboration"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CODA_ACCESS_TOKEN]
---

# Coda API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://coda.io/apis/v1`

## Actions

**List docs:**
```
http(method="GET", url="https://coda.io/apis/v1/docs?limit=10")
```

**Get doc:**
```
http(method="GET", url="https://coda.io/apis/v1/docs/{doc_id}")
```

**List tables:**
```
http(method="GET", url="https://coda.io/apis/v1/docs/{doc_id}/tables")
```

**List rows:**
```
http(method="GET", url="https://coda.io/apis/v1/docs/{doc_id}/tables/{table_id}/rows?limit=20")
```

**Insert row:**
```
http(method="POST", url="https://coda.io/apis/v1/docs/{doc_id}/tables/{table_id}/rows", body={"rows": [{"cells": [{"column": "Name","value": "John"},{"column": "Email","value": "john@example.com"}]}]})
```

## Notes

- Doc IDs are alphanumeric strings.
- Tables can be referenced by name or ID.
- Row values reference columns by name or ID.
- Pagination: `pageToken` from response's `nextPageToken`.
