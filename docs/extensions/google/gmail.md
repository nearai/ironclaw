---
title: "Gmail"
description: "Let your agent read, send, and manage your Gmail messages"
---

The Gmail extension allows your agent to interact with your Gmail inbox — listing and searching messages, reading full email content, sending new emails, creating drafts, replying to threads, and trashing messages. It's ideal for automating email workflows, monitoring important threads, or sending notifications directly from your agent.

---

## Setup

If you haven't set up Google OAuth yet, complete the [Google OAuth Setup](/extensions/google/oauth-setup) first.

<Steps>

<Step title="Enable the Gmail API">

In your Google Cloud project, navigate to **APIs & Services → Library**, search for **Gmail API**, and click **Enable**.

</Step>

<Step title="Install the Extension">

```bash
ironclaw registry install gmail
```

</Step>

<Step title="Authorize Access">

```bash
ironclaw tool auth gmail
```

IronClaw will provide a URL for you to authenticate - remember to follow the [auth setup](./oauth-setup) to enable your agent to capture the callback. If possible, it will open a browser window. Once approved, the token is stored securely and refreshed automatically.

<Tip>
If you already authenticated one Google service, you still need to authenticate each additional Google extension separately.
</Tip>

</Step>

</Steps>

---

## Available Actions

- `list_messages`: List messages in your inbox with an optional Gmail search query, label filter, and result limit
- `get_message`: Read the full content of a message by ID, including headers, body, and labels
- `send_message`: Send a new email with recipient(s), subject, body, and optional CC addresses
- `create_draft`: Save a message as a draft without sending it
- `reply_to_message`: Reply to an existing message thread, keeping the conversation history intact
- `trash_message`: Move a message to the trash

---

## Example Usage

Once configured, you can ask your agent things like:

- _"What emails did I receive from alice@example.com this week?"_
- _"Read my latest unread message"_
- _"Send an email to bob@example.com with subject 'Meeting Notes' and a summary of today's discussion"_
- _"Draft a follow-up to the project proposal thread"_
- _"Reply to the last message in the invoice thread saying the payment has been processed"_
- _"Trash all emails from noreply@newsletter.com"_

---

## Gmail Search Syntax

The `list_messages` action accepts standard Gmail search queries in the `query` field:

| Query | Matches |
|---|---|
| `from:alice@example.com` | Messages from Alice |
| `subject:invoice` | Messages with "invoice" in the subject |
| `is:unread` | Unread messages |
| `label:work` | Messages with the "work" label |
| `after:2025/01/01` | Messages received after January 1, 2025 |
| `has:attachment` | Messages with attachments |

<Tip>
You can combine queries: `from:alice@example.com is:unread` lists all unread messages from Alice.
</Tip>
