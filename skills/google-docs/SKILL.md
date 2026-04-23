---
name: google-docs
version: "1.0.0"
description: Create, read, edit, and format Google Docs documents via HTTP tool with automatic OAuth credential injection
activation:
  keywords:
    - "google docs"
    - "document"
    - "doc"
  exclude_keywords:
    - "pdf"
    - "spreadsheet"
  patterns:
    - "(?i)(create|edit|write|format|read).*(google doc|document)"
    - "(?i)google docs"
  tags:
    - "productivity"
    - "docs"
    - "google"
  max_context_tokens: 2500
credentials:
  - name: google_oauth_token
    provider: google
    location:
      type: bearer
    hosts:
      - "docs.googleapis.com"
    oauth:
      authorization_url: "https://accounts.google.com/o/oauth2/v2/auth"
      token_url: "https://oauth2.googleapis.com/token"
      client_id_env: GOOGLE_OAUTH_CLIENT_ID
      client_secret_env: GOOGLE_OAUTH_CLIENT_SECRET
      scopes:
        - "https://www.googleapis.com/auth/documents"
      extra_params:
        access_type: "offline"
        prompt: "consent"
    setup_instructions: "Configure Google OAuth credentials at console.cloud.google.com/apis/credentials"
http:
  allowed_hosts:
    - "docs.googleapis.com"
---

# Google Docs Skill

You have access to the Google Docs API via the `http` tool. Credentials are automatically injected — **never construct `Authorization` headers manually**.

All Google tools share the same `google_oauth_token`.

## API Patterns

Base URL: `https://docs.googleapis.com/v1/documents`

### Create a document

```
http(method="POST", url="https://docs.googleapis.com/v1/documents", body={"title": "My Document"})
```

Returns `documentId` and `title`.

### Get document (full metadata)

```
http(method="GET", url="https://docs.googleapis.com/v1/documents/{documentId}")
```

Returns full document structure including `body.content` with structural elements.

### Read content (plain text extraction)

The Docs API doesn't have a "read as text" endpoint. Extract text from `body.content`:
- Each `paragraph` element contains `elements` with `textRun.content`
- Concatenate text runs to reconstruct the document text

### Insert text

```
http(method="POST", url="https://docs.googleapis.com/v1/documents/{documentId}:batchUpdate", body={"requests": [{"insertText": {"location": {"index": 1}, "text": "Hello world\n"}}]})
```

- `index`: 1-based position. Use `index: 1` to insert at the beginning.
- To append at the end: first GET the document, find the last `endIndex` in `body.content`, then insert at that index minus 1.

### Delete content

```
http(method="POST", url="https://docs.googleapis.com/v1/documents/{documentId}:batchUpdate", body={"requests": [{"deleteContentRange": {"range": {"startIndex": 5, "endIndex": 20}}}]})
```

### Replace text

```
http(method="POST", url="https://docs.googleapis.com/v1/documents/{documentId}:batchUpdate", body={"requests": [{"replaceAllText": {"containsText": {"text": "old text", "matchCase": true}, "replaceText": "new text"}}]})
```

### Format text (bold, italic, font, color)

```
http(method="POST", url="https://docs.googleapis.com/v1/documents/{documentId}:batchUpdate", body={"requests": [{"updateTextStyle": {"range": {"startIndex": 1, "endIndex": 10}, "textStyle": {"bold": true, "italic": true, "fontSize": {"magnitude": 14, "unit": "PT"}, "weightedFontFamily": {"fontFamily": "Arial"}, "foregroundColor": {"color": {"rgbColor": {"red": 1.0, "green": 0.0, "blue": 0.0}}}}, "fields": "bold,italic,fontSize,weightedFontFamily,foregroundColor"}}]})
```

Colors use float 0.0–1.0 (not hex).

### Format paragraph (headings, alignment)

```
http(method="POST", url="https://docs.googleapis.com/v1/documents/{documentId}:batchUpdate", body={"requests": [{"updateParagraphStyle": {"range": {"startIndex": 1, "endIndex": 50}, "paragraphStyle": {"namedStyleType": "HEADING_1", "alignment": "CENTER"}, "fields": "namedStyleType,alignment"}}]})
```

- `namedStyleType`: `NORMAL_TEXT`, `TITLE`, `SUBTITLE`, `HEADING_1` through `HEADING_6`
- `alignment`: `START`, `CENTER`, `END`, `JUSTIFIED`

### Insert a table

```
http(method="POST", url="https://docs.googleapis.com/v1/documents/{documentId}:batchUpdate", body={"requests": [{"insertTable": {"location": {"index": 50}, "rows": 3, "columns": 2}}]})
```

### Create a list

```
http(method="POST", url="https://docs.googleapis.com/v1/documents/{documentId}:batchUpdate", body={"requests": [{"createParagraphBullets": {"range": {"startIndex": 10, "endIndex": 30}, "bulletPreset": "BULLET_DISC_CIRCLE_SQUARE"}}]})
```

## Common Mistakes

- Do NOT add an `Authorization` header — it is injected automatically.
- The Docs API uses **batchUpdate** for almost all mutations — group multiple requests in one call.
- `index` values are 1-based and can shift after insertions. Always compute indices from the latest GET.
- Colors use `rgbColor` with float 0.0–1.0, not hex strings.
- To append text, you must GET the document first to find the end index.
