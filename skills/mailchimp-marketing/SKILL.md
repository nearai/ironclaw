---
name: mailchimp-marketing
version: "1.0.0"
description: Mailchimp Marketing API — Mailchimp Marketing is an all-in-one marketing platform that helps businesses ma
activation:
  keywords:
    - "mailchimp-marketing"
    - "mailchimp marketing"
    - "tools"
  patterns:
    - "(?i)mailchimp.?marketing"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
---

# Mailchimp Marketing API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://{MAILCHIMP_DC}.api.mailchimp.com/3.0`

## Actions

**List audiences:**
```
http(method="GET", url="https://{MAILCHIMP_DC}.api.mailchimp.com/3.0/lists?count=10")
```

**List members:**
```
http(method="GET", url="https://{MAILCHIMP_DC}.api.mailchimp.com/3.0/lists/{list_id}/members?count=10")
```

**Add member:**
```
http(method="POST", url="https://{MAILCHIMP_DC}.api.mailchimp.com/3.0/lists/{list_id}/members", body={"email_address": "john@example.com","status": "subscribed","merge_fields": {"FNAME": "John","LNAME": "Doe"}})
```

**List campaigns:**
```
http(method="GET", url="https://{MAILCHIMP_DC}.api.mailchimp.com/3.0/campaigns?count=10&status=sent")
```

**Get campaign report:**
```
http(method="GET", url="https://{MAILCHIMP_DC}.api.mailchimp.com/3.0/reports/{campaign_id}")
```

## Notes

- Data center (dc) is in the API key suffix: `key-us21` → `us21`.
- Uses Basic auth with any username and API key as password.
- Member status: `subscribed`, `unsubscribed`, `cleaned`, `pending`, `transactional`.
- Merge fields are custom per audience (FNAME, LNAME are defaults).
