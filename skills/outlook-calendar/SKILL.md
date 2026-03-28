---
name: outlook-calendar
version: "1.0.0"
description: Outlook Calendar API — A digital calendar platform that lets users schedule events, manage appointments
activation:
  keywords:
    - "outlook-calendar"
    - "outlook calendar"
    - "ai"
  patterns:
    - "(?i)outlook.?calendar"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
---

# Outlook Calendar API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://graph.microsoft.com/v1.0`

## Actions

**List events:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/me/calendar/events?$top=10&$orderby=start/dateTime")
```

**Get event:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/me/events/{event_id}")
```

**Create event:**
```
http(method="POST", url="https://graph.microsoft.com/v1.0/me/events", body={"subject": "Team Meeting","start": {"dateTime": "2026-03-28T10:00:00","timeZone": "America/New_York"},"end": {"dateTime": "2026-03-28T11:00:00","timeZone": "America/New_York"},"attendees": [{"emailAddress": {"address": "john@example.com"},"type": "required"}]})
```

**Delete event:**
```
http(method="DELETE", url="https://graph.microsoft.com/v1.0/me/events/{event_id}")
```

**List calendars:**
```
http(method="GET", url="https://graph.microsoft.com/v1.0/me/calendars")
```

## Notes

- Uses OAuth 2.0 via Microsoft Graph — credentials are auto-injected.
- DateTime format: `YYYY-MM-DDTHH:MM:SS` with separate `timeZone`.
- Attendee types: `required`, `optional`, `resource`.
- Recurrence uses `recurrence` object with `pattern` and `range`.
