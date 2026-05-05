---
name: google-sheets
version: "1.0.0"
description: Create, read, write, and format Google Sheets spreadsheets via HTTP tool with automatic OAuth credential injection
activation:
  keywords:
    - "spreadsheet"
    - "sheets"
    - "google sheets"
    - "cell"
    - "formula"
  patterns:
    - "(?i)(create|read|write|update).*(spreadsheet|sheets|sheet)"
    - "(?i)google sheets"
  tags:
    - "productivity"
    - "sheets"
    - "google"
  max_context_tokens: 2500
credentials:
  - name: google_oauth_token
    provider: google
    location:
      type: bearer
    hosts:
      - "sheets.googleapis.com"
    oauth:
      authorization_url: "https://accounts.google.com/o/oauth2/v2/auth"
      token_url: "https://oauth2.googleapis.com/token"
      client_id_env: GOOGLE_OAUTH_CLIENT_ID
      client_secret_env: GOOGLE_OAUTH_CLIENT_SECRET
      scopes:
        - "https://www.googleapis.com/auth/spreadsheets"
      extra_params:
        access_type: "offline"
        prompt: "consent"
    setup_instructions: "Configure Google OAuth credentials at console.cloud.google.com/apis/credentials"
http:
  allowed_hosts:
    - "sheets.googleapis.com"
---

# Google Sheets Skill

You have access to the Google Sheets API via the `http` tool. Credentials are automatically injected — **never construct `Authorization` headers manually**.

All Google tools share the same `google_oauth_token`.

## API Patterns

Base URL: `https://sheets.googleapis.com/v4/spreadsheets`

### Create a spreadsheet

```
http(method="POST", url="https://sheets.googleapis.com/v4/spreadsheets", body={"properties": {"title": "My Sheet"}, "sheets": [{"properties": {"title": "Sheet1"}}]})
```

Returns `spreadsheetId`, `spreadsheetUrl`.

### Get spreadsheet metadata

```
http(method="GET", url="https://sheets.googleapis.com/v4/spreadsheets/{spreadsheetId}")
```

Returns sheet list with `sheetId`, `title`, etc.

### Read values

```
http(method="GET", url="https://sheets.googleapis.com/v4/spreadsheets/{spreadsheetId}/values/Sheet1!A1:D10")
```

A1 notation: `Sheet1!A1:D10` or just `A1:D10` for first sheet.

### Batch read multiple ranges

```
http(method="GET", url="https://sheets.googleapis.com/v4/spreadsheets/{spreadsheetId}/values:batchGet?ranges=Sheet1!A1:D10&ranges=Sheet2!A1:C5")
```

### Write values

```
http(method="PUT", url="https://sheets.googleapis.com/v4/spreadsheets/{spreadsheetId}/values/Sheet1!A1?valueInputOption=USER_ENTERED", body={"range": "Sheet1!A1", "values": [["Name", "Score"], ["Alice", 95], ["Bob", 87]]})
```

- `valueInputOption`: `USER_ENTERED` (parse dates/formulas) or `RAW` (literal strings)

### Append values

```
http(method="POST", url="https://sheets.googleapis.com/v4/spreadsheets/{spreadsheetId}/values/Sheet1!A1:append?valueInputOption=USER_ENTERED", body={"values": [["Charlie", 92]]})
```

Appends after the last row with data.

### Clear values

```
http(method="POST", url="https://sheets.googleapis.com/v4/spreadsheets/{spreadsheetId}/values/Sheet1!A1:D10:clear")
```

### Add a sheet (tab)

```
http(method="POST", url="https://sheets.googleapis.com/v4/spreadsheetId/sheets:batchUpdate" — use batchUpdate with addSheet request)
```

Or via the batchUpdate endpoint:
```
http(method="POST", url="https://sheets.googleapis.com/v4/spreadsheets/{spreadsheetId}:batchUpdate", body={"requests": [{"addSheet": {"properties": {"title": "NewTab"}}}]})
```

### Format cells

```
http(method="POST", url="https://sheets.googleapis.com/v4/spreadsheets/{spreadsheetId}:batchUpdate", body={"requests": [{"repeatCell": {"range": {"sheetId": 0, "startRowIndex": 0, "endRowIndex": 1, "startColumnIndex": 0, "endColumnIndex": 2}, "cell": {"userEnteredFormat": {"textFormat": {"bold": true}, "backgroundColor": {"red": 0.9, "green": 0.9, "blue": 0.9}}}, "fields": "userEnteredFormat(textFormat,backgroundColor)"}}]})
```

Colors use float 0.0–1.0 (not hex). Sheet IDs are numeric (from metadata).

## Common Mistakes

- Do NOT add an `Authorization` header — it is injected automatically.
- A1 notation: `Sheet1!A1:B2` includes the sheet name. Without it, defaults to first sheet.
- `values` is always a 2D array: `[[row1col1, row1col2], [row2col1, row2col2]]`.
- Use `USER_ENTERED` for `valueInputOption` unless you need literal strings.
- Append uses `POST` to `.../values/{range}:append` (note the colon).
- Sheet tab IDs are **numeric** (`sheetId: 0`), not the tab title.
