---
name: zendesk
version: "1.0.0"
description: Zendesk Support API — tickets, users, comments, search, organizations
activation:
  keywords:
    - "zendesk"
    - "zendesk ticket"
    - "support ticket"
  exclude_keywords:
    - "jira"
    - "freshdesk"
  patterns:
    - "(?i)zendesk.*(ticket|user|organization)"
    - "(?i)(create|update|close).*support ticket"
  tags:
    - "customer-support"
    - "helpdesk"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [ZENDESK_SUBDOMAIN, ZENDESK_EMAIL, ZENDESK_API_TOKEN]
---

# Zendesk Support API

Use the `http` tool. Auth uses `{email}/token:{api_token}` as Basic auth. Credentials are automatically injected for your Zendesk subdomain.

## Base URL

`https://{ZENDESK_SUBDOMAIN}.zendesk.com/api/v2`

## Actions

**List tickets:**
```
http(method="GET", url="https://{subdomain}.zendesk.com/api/v2/tickets?sort_by=created_at&sort_order=desc&per_page=20")
```

**Get ticket:**
```
http(method="GET", url="https://{subdomain}.zendesk.com/api/v2/tickets/<ticket_id>")
```

**Create ticket:**
```
http(method="POST", url="https://{subdomain}.zendesk.com/api/v2/tickets", body={"ticket": {"subject": "Issue with login", "description": "Customer cannot log in", "priority": "high", "type": "problem", "requester": {"email": "user@example.com"}}})
```

**Update ticket:**
```
http(method="PUT", url="https://{subdomain}.zendesk.com/api/v2/tickets/<ticket_id>", body={"ticket": {"status": "solved", "comment": {"body": "This has been resolved.", "public": true}}})
```

**Add comment:**
```
http(method="PUT", url="https://{subdomain}.zendesk.com/api/v2/tickets/<ticket_id>", body={"ticket": {"comment": {"body": "Internal note", "public": false}}})
```

**Search tickets:**
```
http(method="GET", url="https://{subdomain}.zendesk.com/api/v2/search.json?query=type:ticket+status:open+priority:high&sort_by=created_at&sort_order=desc")
```

**List users:**
```
http(method="GET", url="https://{subdomain}.zendesk.com/api/v2/users?role=end-user&per_page=20")
```

**Get user:**
```
http(method="GET", url="https://{subdomain}.zendesk.com/api/v2/users/<user_id>")
```

## Notes

- Responses: `{"ticket": {...}}` or `{"tickets": [...]}`.
- Status flow: `new` → `open` → `pending` → `solved` → `closed`.
- Priority: `low`, `normal`, `high`, `urgent`.
- Type: `problem`, `incident`, `question`, `task`.
- Comments with `public: false` are internal notes (not visible to requester).
- Pagination: use `next_page` URL from response. Check `count` for total.
- Search syntax: `type:ticket status:open assignee:me priority:high created>2026-01-01`.
