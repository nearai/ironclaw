---
name: google-calendar
version: "1.0.0"
description: Google Calendar API — events, calendars, free/busy, reminders
activation:
  keywords:
    - "calendar"
    - "google calendar"
    - "event"
    - "schedule"
    - "meeting"
  exclude_keywords:
    - "calendly"
    - "cal.com"
    - "outlook calendar"
  patterns:
    - "(?i)(create|list|update|delete|schedule).*event"
    - "(?i)(google )?calendar.*(event|meeting|schedule)"
    - "(?i)free.*(busy|slot|time)"
  tags:
    - "calendar"
    - "scheduling"
    - "google"
  max_context_tokens: 1500
metadata:
  openclaw:
    requires:
      env: [GOOGLE_ACCESS_TOKEN]
---

# Google Calendar API

Use the `http` tool. Credentials are automatically injected for `googleapis.com`.

## Base URL

`https://www.googleapis.com/calendar/v3`

## Actions

**List upcoming events:**
```
http(method="GET", url="https://www.googleapis.com/calendar/v3/calendars/primary/events?timeMin=2026-03-27T00:00:00Z&maxResults=10&singleEvents=true&orderBy=startTime")
```

**Get event:**
```
http(method="GET", url="https://www.googleapis.com/calendar/v3/calendars/primary/events/<event_id>")
```

**Create event:**
```
http(method="POST", url="https://www.googleapis.com/calendar/v3/calendars/primary/events", body={"summary": "Team Standup", "location": "Conference Room", "description": "Daily sync", "start": {"dateTime": "2026-03-28T09:00:00-07:00"}, "end": {"dateTime": "2026-03-28T09:30:00-07:00"}, "attendees": [{"email": "alice@example.com"}], "reminders": {"useDefault": false, "overrides": [{"method": "popup", "minutes": 10}]}})
```

**Create all-day event:**
```
http(method="POST", url="https://www.googleapis.com/calendar/v3/calendars/primary/events", body={"summary": "Team Offsite", "start": {"date": "2026-04-01"}, "end": {"date": "2026-04-02"}})
```

**Update event:**
```
http(method="PATCH", url="https://www.googleapis.com/calendar/v3/calendars/primary/events/<event_id>", body={"summary": "Updated Title", "start": {"dateTime": "2026-03-28T10:00:00-07:00"}, "end": {"dateTime": "2026-03-28T10:30:00-07:00"}})
```

**Delete event:**
```
http(method="DELETE", url="https://www.googleapis.com/calendar/v3/calendars/primary/events/<event_id>")
```

**Free/busy query:**
```
http(method="POST", url="https://www.googleapis.com/calendar/v3/freeBusy", body={"timeMin": "2026-03-28T08:00:00Z", "timeMax": "2026-03-28T18:00:00Z", "items": [{"id": "primary"}]})
```

**List calendars:**
```
http(method="GET", url="https://www.googleapis.com/calendar/v3/users/me/calendarList")
```

## Notes

- Use `primary` as calendar ID for the user's main calendar.
- Timed events use `dateTime` (ISO 8601 with timezone). All-day events use `date` (YYYY-MM-DD).
- `singleEvents=true` expands recurring events into individual instances.
- `timeMin`/`timeMax` must be RFC 3339 timestamps with timezone.
- Pagination: use `pageToken` from `nextPageToken`.
- Delete returns 204 No Content.
