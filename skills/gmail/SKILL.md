---
name: gmail
version: "1.0.0"
description: Gmail API — send, read, search, label, draft emails
activation:
  keywords:
    - "gmail"
    - "email"
    - "send email"
    - "inbox"
  exclude_keywords:
    - "outlook"
    - "resend"
    - "postmark"
  patterns:
    - "(?i)(send|read|search|draft|reply).*email"
    - "(?i)gmail.*(message|inbox|label|draft)"
  tags:
    - "email"
    - "communication"
    - "google"
  max_context_tokens: 1800
metadata:
  openclaw:
    requires:
      env: [GOOGLE_ACCESS_TOKEN]
---

# Gmail API

Use the `http` tool. Credentials are automatically injected for `gmail.googleapis.com`.

## Base URL

`https://gmail.googleapis.com/gmail/v1/users/me`

## Actions

**List messages (inbox):**
```
http(method="GET", url="https://gmail.googleapis.com/gmail/v1/users/me/messages?maxResults=10&q=is:unread")
```

**Get message:**
```
http(method="GET", url="https://gmail.googleapis.com/gmail/v1/users/me/messages/<message_id>?format=full")
```

**Search messages:**
```
http(method="GET", url="https://gmail.googleapis.com/gmail/v1/users/me/messages?q=from:john@acme.com+subject:invoice+after:2026/01/01&maxResults=20")
```

**Send email:**
```
http(method="POST", url="https://gmail.googleapis.com/gmail/v1/users/me/messages/send", body={"raw": "<base64url_encoded_RFC2822_message>"})
```

To construct the `raw` field, base64url-encode an RFC 2822 message:
```
From: me@gmail.com
To: recipient@example.com
Subject: Hello

Body text here
```

**Reply to email:**
Same as send, but include `In-Reply-To` and `References` headers in the RFC 2822 message, and set `threadId` in the body.

**Create draft:**
```
http(method="POST", url="https://gmail.googleapis.com/gmail/v1/users/me/drafts", body={"message": {"raw": "<base64url_encoded_message>"}})
```

**List labels:**
```
http(method="GET", url="https://gmail.googleapis.com/gmail/v1/users/me/labels")
```

**Modify labels (archive, mark read):**
```
http(method="POST", url="https://gmail.googleapis.com/gmail/v1/users/me/messages/<message_id>/modify", body={"removeLabelIds": ["UNREAD", "INBOX"]})
```

**Trash message:**
```
http(method="POST", url="https://gmail.googleapis.com/gmail/v1/users/me/messages/<message_id>/trash")
```

## Search Syntax

Gmail search operators: `from:`, `to:`, `subject:`, `is:unread`, `is:starred`, `has:attachment`, `after:YYYY/MM/DD`, `before:YYYY/MM/DD`, `label:`, `in:sent`, `larger:5M`.

## Notes

- `raw` must be base64url-encoded (not standard base64): replace `+` with `-`, `/` with `_`, remove `=` padding.
- Message list returns only IDs. Use GET with `format=full` to get headers and body.
- Body content is in `payload.parts[].body.data` (base64url-encoded).
- Pagination: use `pageToken` from `nextPageToken` in response.
- Labels: `INBOX`, `SENT`, `DRAFT`, `SPAM`, `TRASH`, `UNREAD`, `STARRED`, `IMPORTANT`.
