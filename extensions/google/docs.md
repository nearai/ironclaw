---
title: "Docs"
description: "Let your agent create and edit Google Documents"
---

The Google Docs extension allows your agent to interact with Google Docs — creating documents, reading content, inserting and formatting text, managing tables and lists, and running batch updates. It's ideal for drafting reports, editing existing documents, or automating document workflows directly from your agent.

---

## Setup

If you haven't set up Google OAuth yet, complete the [Google OAuth Setup](/extensions/google/oauth-setup) first.

<Steps>

<Step title="Enable the Google Docs API">

In your Google Cloud project, navigate to **APIs & Services → Library**, search for **Google Docs API**, and click **Enable**.

</Step>

<Step title="Install the Extension">

```bash
ironclaw registry install google-docs
```

</Step>

<Step title="Authorize Access">

```bash
ironclaw tool auth google-docs
```

IronClaw will provide a URL for you to authenticate - remember to follow the [auth setup](./oauth-setup) to enable your agent to capture the callback. If possible, it will open a browser window. Once approved, the token is stored securely and refreshed automatically.

<Tip>
If you already authenticated one Google service, you still need to authenticate each additional Google extension separately.
</Tip>

</Step>

</Steps>

---

## Available Actions

- `create_document`: Create a new Google Doc with an optional title
- `get_document`: Retrieve document metadata (title, revision, named ranges)
- `read_content`: Extract the plain-text or structured content of a document
- `insert_text`: Insert text at a specific index in the document body
- `delete_content`: Delete a range of content by start and end index
- `replace_text`: Find and replace text throughout the document
- `format_text`: Apply character formatting (bold, italic, font size, color) to a text range
- `format_paragraph`: Apply paragraph styling (heading level, alignment, spacing, indentation) to a range
- `insert_table`: Insert a table with a specified number of rows and columns
- `create_list`: Convert a range of paragraphs into a bulleted or numbered list
- `batch_update`: Send multiple document update requests in a single API call

---

## Example Usage

Once configured, you can ask your agent things like:

- _"Create a new document titled 'Q2 Marketing Plan'"_
- _"Read the content of document ID 1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgVE2upms"_
- _"Insert a summary paragraph at the top of my report"_
- _"Replace all occurrences of 'TBD' with 'Pending Review' in this doc"_
- _"Format the title as Heading 1 and make it bold"_
- _"Add a 3-column table for the budget breakdown"_

---

## Working with Document IDs

Google Doc IDs appear in the document URL:

```
https://docs.google.com/document/d/<DOCUMENT_ID>/edit
```

<Tip>
You can tell your agent to "use the document at this URL" and paste the full URL — the agent will extract the document ID automatically.
</Tip>
