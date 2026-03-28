---
name: calendly
version: "1.0.0"
description: Calendly API v2 — event types, scheduled events, invitees
activation:
  keywords:
    - "calendly"
    - "calendly event"
    - "booking"
    - "scheduling link"
  exclude_keywords:
    - "cal.com"
    - "google calendar"
  patterns:
    - "(?i)calendly.*(event|booking|invitee|schedule)"
  tags:
    - "scheduling"
    - "calendar"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CALENDLY_ACCESS_TOKEN]
---

# Calendly API v2

Use the `http` tool. Credentials are automatically injected for `api.calendly.com`.

## Base URL

`https://api.calendly.com`

## Actions

**Get current user:**
```
http(method="GET", url="https://api.calendly.com/users/me")
```

**List event types:**
```
http(method="GET", url="https://api.calendly.com/event_types?user=<user_uri>&count=20")
```

**List scheduled events:**
```
http(method="GET", url="https://api.calendly.com/scheduled_events?user=<user_uri>&min_start_time=2026-03-27T00:00:00Z&max_start_time=2026-04-30T00:00:00Z&status=active&count=20")
```

**Get event details:**
```
http(method="GET", url="https://api.calendly.com/scheduled_events/<event_uuid>")
```

**List invitees for event:**
```
http(method="GET", url="https://api.calendly.com/scheduled_events/<event_uuid>/invitees?count=20")
```

**Cancel event:**
```
http(method="POST", url="https://api.calendly.com/scheduled_events/<event_uuid>/cancellation", body={"reason": "Schedule conflict"})
```

**List organization members:**
```
http(method="GET", url="https://api.calendly.com/organization_memberships?organization=<org_uri>")
```

## Notes

- Resources are identified by URIs (e.g., `https://api.calendly.com/users/ABCDEF123456`), not plain IDs.
- First call `GET /users/me` to get your `uri` and `current_organization`.
- Event status: `active`, `canceled`.
- Pagination: use `page_token` from `pagination.next_page_token`.
- Dates are ISO 8601 with timezone (UTC recommended).
