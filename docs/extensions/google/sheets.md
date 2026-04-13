---
title: "Sheets"
description: "Let your agent read and write Google Spreadsheets"
---

The Google Sheets extension allows your agent to interact with Google Sheets — creating spreadsheets, reading and writing cell ranges, appending rows, formatting cells, and managing sheets. It uses standard A1 notation for ranges and is ideal for data entry automation, report generation, and spreadsheet-driven workflows.

---

## Setup

If you haven't set up Google OAuth yet, complete the [Google OAuth Setup](/extensions/google/oauth-setup) first.

<Steps>

<Step title="Enable the Google Sheets API">

In your Google Cloud project, navigate to **APIs & Services → Library**, search for **Google Sheets API**, and click **Enable**.

</Step>

<Step title="Install the Extension">

```bash
ironclaw registry install google-sheets
```

</Step>

<Step title="Authorize Access">

```bash
ironclaw tool auth google-sheets
```

IronClaw will provide a URL for you to authenticate - remember to follow the [auth setup](./oauth-setup) to enable your agent to capture the callback. If possible, it will open a browser window. Once approved, the token is stored securely and refreshed automatically.

<Tip>
If you already authenticated one Google service, you still need to authenticate each additional Google extension separately.
</Tip>

</Step>

</Steps>

---

## Available Actions

- `create_spreadsheet`: Create a new spreadsheet with an optional title and initial sheet names
- `get_spreadsheet`: Retrieve spreadsheet metadata (title, sheet names, named ranges)
- `read_values`: Read cell values from a range using A1 notation (e.g. `Sheet1!A1:D10`)
- `batch_read_values`: Read multiple ranges in a single API call
- `write_values`: Write values to a range, replacing existing content
- `append_values`: Append rows after the last row that contains data in a range
- `clear_values`: Clear all values from a range (preserving formatting)
- `add_sheet`: Add a new sheet (tab) to an existing spreadsheet
- `delete_sheet`: Delete a sheet by its ID
- `rename_sheet`: Rename an existing sheet
- `format_cells`: Apply number formats, text styles, or background colors to a cell range

---

## Example Usage

Once configured, you can ask your agent things like:

- _"Create a new spreadsheet called 'Monthly Expenses'"_
- _"Read the values from cells A1 to E20 in my budget sheet"_
- _"Add a new row with today's sales data to the 'Sales' tab"_
- _"Clear all data from the 'Draft' sheet"_
- _"Rename the first sheet to 'Summary'"_
- _"Format column B as currency in my expenses spreadsheet"_

---

## Using A1 Notation

All range operations use standard A1 notation. You can include the sheet name to target a specific tab:

| Notation | Meaning |
|---|---|
| `A1` | Single cell |
| `A1:C10` | Range across rows and columns |
| `Sheet1!A1:B5` | Range on a specific sheet |
| `Sheet1!A:A` | Entire column A on Sheet1 |

<Tip>
If your spreadsheet has multiple sheets, include the sheet name in the range (e.g. `Budget!B2:D50`) so the agent targets the right tab.
</Tip>
