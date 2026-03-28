---
name: brevo
version: "1.0.0"
description: Brevo (Sendinblue) API — email campaigns, contacts, transactional email, SMS
activation:
  keywords:
    - "brevo"
    - "sendinblue"
    - "email campaign"
  exclude_keywords:
    - "mailchimp"
    - "resend"
  patterns:
    - "(?i)(brevo|sendinblue).*(email|contact|campaign|sms)"
  tags:
    - "email"
    - "marketing"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [BREVO_API_KEY]
---

# Brevo API (formerly Sendinblue)

Use the `http` tool. Include `api-key` header.

## Base URL

`https://api.brevo.com/v3`

## Actions

**Send transactional email:**
```
http(method="POST", url="https://api.brevo.com/v3/smtp/email", headers=[{"name": "api-key", "value": "{BREVO_API_KEY}"}], body={"sender": {"name": "My App", "email": "noreply@yourdomain.com"}, "to": [{"email": "recipient@example.com", "name": "John"}], "subject": "Welcome!", "htmlContent": "<h1>Hello John</h1><p>Welcome aboard.</p>"})
```

**List contacts:**
```
http(method="GET", url="https://api.brevo.com/v3/contacts?limit=20&offset=0", headers=[{"name": "api-key", "value": "{BREVO_API_KEY}"}])
```

**Create contact:**
```
http(method="POST", url="https://api.brevo.com/v3/contacts", headers=[{"name": "api-key", "value": "{BREVO_API_KEY}"}], body={"email": "new@example.com", "attributes": {"FIRSTNAME": "Alice", "LASTNAME": "Smith"}, "listIds": [1]})
```

**Update contact:**
```
http(method="PUT", url="https://api.brevo.com/v3/contacts/user@example.com", headers=[{"name": "api-key", "value": "{BREVO_API_KEY}"}], body={"attributes": {"FIRSTNAME": "Updated"}})
```

**List email campaigns:**
```
http(method="GET", url="https://api.brevo.com/v3/emailCampaigns?limit=20&status=sent", headers=[{"name": "api-key", "value": "{BREVO_API_KEY}"}])
```

**Get campaign stats:**
```
http(method="GET", url="https://api.brevo.com/v3/emailCampaigns/<campaign_id>", headers=[{"name": "api-key", "value": "{BREVO_API_KEY}"}])
```

**Send SMS:**
```
http(method="POST", url="https://api.brevo.com/v3/transactionalSMS/sms", headers=[{"name": "api-key", "value": "{BREVO_API_KEY}"}], body={"sender": "MyApp", "recipient": "+1234567890", "content": "Your code is 123456", "type": "transactional"})
```

## Notes

- Contact attributes are uppercase: `FIRSTNAME`, `LASTNAME`, `SMS`.
- `listIds` are integer IDs of contact lists.
- Campaign statuses: `draft`, `sent`, `queued`, `suspended`, `in_process`.
- Pagination: `limit` + `offset`. Check `count` for total.
- Transactional email returns `messageId` for tracking.
