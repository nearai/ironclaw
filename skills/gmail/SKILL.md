---
name: gmail
version: "1.0.0"
description: Read, search, send, draft, and reply to emails via Gmail API with automatic OAuth credential injection
activation:
  keywords:
    - "email"
    - "gmail"
    - "mail"
    - "inbox"
    - "send email"
  exclude_keywords:
    - "slack"
    - "telegram"
  patterns:
    - "(?i)(send|read|check|search|draft|reply).*(email|gmail|mail|inbox)"
    - "(?i)gmail"
  tags:
    - "communication"
    - "email"
    - "google"
  max_context_tokens: 2500
credentials:
  - name: google_oauth_token
    provider: google
    location:
      type: bearer
    hosts:
      - "gmail.googleapis.com"
    oauth:
      authorization_url: "https://accounts.google.com/o/oauth2/v2/auth"
      token_url: "https://oauth2.googleapis.com/token"
      client_id_env: GOOGLE_OAUTH_CLIENT_ID
      client_secret_env: GOOGLE_OAUTH_CLIENT_SECRET
      scopes:
        - "https://www.googleapis.com/auth/gmail.modify"
        - "https://www.googleapis.com/auth/gmail.compose"
      extra_params:
        access_type: "offline"
        prompt: "consent"
    setup_instructions: "Configure Google OAuth credentials at console.cloud.google.com/apis/credentials"
http:
  allowed_hosts:
    - "gmail.googleapis.com"
---

# Gmail Skill

You have access to the Gmail API via the `http` tool. Credentials are automatically injected — **never construct `Authorization` headers manually**. When the URL host is `gmail.googleapis.com`, the system injects `Authorization: Bearer {google_oauth_token}` transparently.

All Google tools (Gmail, Calendar, Drive, Docs, Sheets, Slides) share the same `google_oauth_token`.

## API Patterns

Base URL: `https://gmail.googleapis.com/gmail/v1/users/me`

### List/search messages

```
http(method="GET", url="https://gmail.googleapis.com/gmail/v1/users/me/messages?q=is:unread&maxResults=20")
```

- `q`: Gmail search syntax — `is:unread`, `from:user@example.com`, `subject:text`, `after:2025/01/01`, `label:INBOX`
- `maxResults`: default 20
- `labelIds`: filter by label (INBOX, SENT, DRAFT, TRASH, SPAM)
- Response: `messages` array (each has `id` and `threadId` only — call GetMessage for full content)

### Get a message

```
http(method="GET", url="https://gmail.googleapis.com/gmail/v1/users/me/messages/{messageId}?format=full")
```

- `format`: `full` (default), `minimal`, `raw`, `metadata`
- Response includes `snippet`, `payload.headers` (From, To, Subject, Date), `payload.body.data` (base64-encoded)

### Send a message

```
http(method="POST", url="https://gmail.googleapis.com/gmail/v1/users/me/messages/send", body={"raw": "<base64-encoded RFC 2822 message>"})
```

The Gmail API requires messages in raw RFC 2822 format, base64url-encoded. Construct the raw email:

```python
import base64
raw = f"From: me\r\nTo: {to}\r\nSubject: {subject}\r\n\r\n{body}"
encoded = base64.urlsafe_b64encode(raw.encode()).decode()
# Then POST with body={"raw": encoded}
```

### Create a draft

```
http(method="POST", url="https://gmail.googleapis.com/gmail/v1/users/me/drafts", body={"message": {"raw": "<base64-encoded RFC 2822 message>"}})
```

### Reply to a message

```
http(method="POST", url="https://gmail.googleapis.com/gmail/v1/users/me/messages/send", body={"raw": "<base64-encoded RFC 2822 with In-Reply-To and References headers>", "threadId": "{threadId}"})
```

Set `In-Reply-To` and `References` headers to the original message's `Message-Id` header.

### Trash a message

```
http(method="POST", url="https://gmail.googleapis.com/gmail/v1/users/me/messages/{messageId}/trash")
```

## Common Mistakes

- Do NOT add an `Authorization` header — it is injected automatically.
- Gmail messages must be sent as `raw` base64url-encoded RFC 2822. Do NOT use `body.content` — use `raw`.
- The `list` endpoint returns only `id` and `threadId`. Always call `get` for full content.
- For base64url encoding, use `base64.urlsafe_b64encode` (replace `+` with `-`, `/` with `_`).
