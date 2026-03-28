---
name: google-sheets
version: "1.0.0"
description: Google Sheets API — read, write, append, format cells, manage sheets
activation:
  keywords:
    - "google sheets"
    - "spreadsheet"
    - "sheets"
    - "gsheets"
  exclude_keywords:
    - "airtable"
    - "excel"
  patterns:
    - "(?i)(read|write|update|append).*spreadsheet"
    - "(?i)google sheets?.*(read|write|cell|row|column)"
  tags:
    - "spreadsheet"
    - "google"
    - "data"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [GOOGLE_ACCESS_TOKEN]
---

# Google Sheets API

Use the `http` tool. Credentials are automatically injected for `googleapis.com`.

## Base URL

`https://sheets.googleapis.com/v4/spreadsheets`

## Actions

**Get spreadsheet metadata:**
```
http(method="GET", url="https://sheets.googleapis.com/v4/spreadsheets/<spreadsheet_id>?fields=properties.title,sheets.properties")
```

**Read values:**
```
http(method="GET", url="https://sheets.googleapis.com/v4/spreadsheets/<spreadsheet_id>/values/Sheet1!A1:D10")
```

**Read multiple ranges:**
```
http(method="GET", url="https://sheets.googleapis.com/v4/spreadsheets/<spreadsheet_id>/values:batchGet?ranges=Sheet1!A1:B5&ranges=Sheet1!D1:E5")
```

**Write values:**
```
http(method="PUT", url="https://sheets.googleapis.com/v4/spreadsheets/<spreadsheet_id>/values/Sheet1!A1:C3?valueInputOption=USER_ENTERED", body={"values": [["Name", "Age", "City"], ["Alice", 30, "NYC"], ["Bob", 25, "LA"]]})
```

**Append rows:**
```
http(method="POST", url="https://sheets.googleapis.com/v4/spreadsheets/<spreadsheet_id>/values/Sheet1!A:C:append?valueInputOption=USER_ENTERED&insertDataOption=INSERT_ROWS", body={"values": [["Charlie", 35, "SF"]]})
```

**Clear range:**
```
http(method="POST", url="https://sheets.googleapis.com/v4/spreadsheets/<spreadsheet_id>/values/Sheet1!A1:D10:clear")
```

**Create spreadsheet:**
```
http(method="POST", url="https://sheets.googleapis.com/v4/spreadsheets", body={"properties": {"title": "New Spreadsheet"}, "sheets": [{"properties": {"title": "Data"}}]})
```

**Add sheet:**
```
http(method="POST", url="https://sheets.googleapis.com/v4/spreadsheets/<spreadsheet_id>:batchUpdate", body={"requests": [{"addSheet": {"properties": {"title": "New Sheet"}}}]})
```

## Notes

- Range notation: `Sheet1!A1:D10`, `Sheet1!A:D` (whole columns), `Sheet1!1:5` (whole rows).
- `valueInputOption`: `RAW` (as-is) or `USER_ENTERED` (parses numbers/dates/formulas).
- Spreadsheet ID is from URL: `docs.google.com/spreadsheets/d/<spreadsheet_id>/edit`.
- Write returns `updatedCells`, `updatedRows`, `updatedColumns` counts.
- Sheet names with spaces need single quotes in range: `'My Sheet'!A1:B5`.
