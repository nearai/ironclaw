---
name: mailbox-validator
version: "1.0.0"
description: Mailbox Validator API — MailboxValidator provides real-time email address verification and list-cleaning
activation:
  keywords:
    - "mailbox-validator"
    - "mailbox validator"
    - "email-verification"
  patterns:
    - "(?i)mailbox.?validator"
  tags:
    - "tools"
    - "email-verification"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [MAILBOX_VALIDATOR_API_KEY]
---

# Mailbox Validator API

Use the `http` tool. API key is automatically injected as `key` query parameter.

> MailboxValidator provides real-time email address verification and list-cleaning services—via easy RESTful APIs or bulk uploads—to detect invalid, disposable, free, role-based and unreachable emails, 

## Authentication

This integration uses **query parameter** authentication via `key`.

## Required Credentials

- `MAILBOX_VALIDATOR_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
