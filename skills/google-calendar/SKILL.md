---
name: google-calendar
version: "1.0.0"
description: View, create, update, and delete Google Calendar events via HTTP tool with automatic OAuth credential injection
activation:
  keywords:
    - "calendar"
    - "event"
    - "schedule"
    - "meeting"
    - "appointment"
  exclude_keywords:
    - "github"
  patterns:
    - "(?i)(create|add|schedule|book).*(event|meeting|appointment)"
    - "(?i)(list|show|get).*(calendar|events|schedule)"
    - "(?i)google calendar"
  tags:
    - "productivity"
    - "calendar"
    - "google"
  max_context_tokens: 2000
credentials:
  - name: google_oauth_token
    provider: google
    location:
      type: bearer
    hosts:
      - "www.googleapis.com"
    oauth:
      authorization_url: "https://accounts.google.com/o/oauth2/v2/auth"
      token_url: "https://oauth2.googleapis.com/token"
      client_id_env: GOOGLE_OAUTH_CLIENT_ID
      client_secret_env: GOOGLE_OAUTH_CLIENT_SECRET
      scopes:
        - "https://www.googleapis.com/auth/calendar.events"
      extra_params:
        access_type: "offline"
        prompt: "consent"
    setup_instructions: "Configure Google OAuth credentials at console.cloud.google.com/apis/credentials"
http:
  allowed_hosts:
    - "www.googleapis.com"
---

# Google Calendar Skill

You have access to the Google Calendar API via the `http` tool. Credentials are automatically injected — **never construct `Authorization` headers manually**.

All Google tools share the same `google_oauth_token`.

## API Patterns

Base URL: `https://www.googleapis.com/calendar/v3/calendars/{calendarId}`

Use `primary` as the default `calendarId` for the user's primary calendar.

### List events

```
http(method="GET", url="https://www.googleapis.com/calendar/v3/calendars/primary/events?maxResults=25&orderBy=startTime&singleEvents=true")
```

- `timeMin`, `timeMax`: RFC3339 timestamps (e.g. `2025-01-15T00:00:00Z`)
- `q`: Free-text search
- `maxResults`: default 25
- `singleEvents=true`: Expand recurring events into individual instances
- `orderBy=startTime`: Sort by start time (requires `singleEvents=true`)

### Get an event

```
http(method="GET", url="https://www.googleapis.com/calendar/v3/calendars/primary/events/{eventId}")
```

### Create an event

**Timed event:**
```
http(method="POST", url="https://www.googleapis.com/calendar/v3/calendars/primary/events", body={"summary": "Team standup", "start": {"dateTime": "2025-01-15T09:00:00", "timeZone": "America/New_York"}, "end": {"dateTime": "2025-01-15T09:30:00", "timeZone": "America/New_York"}, "attendees": [{"email": "user@example.com"}], "location": "Conference Room A"})
```

**All-day event:**
```
http(method="POST", url="https://www.googleapis.com/calendar/v3/calendars/primary/events", body={"summary": "Conference", "start": {"date": "2025-01-15"}, "end": {"date": "2025-01-17"}})
```

- `end.date` is **exclusive** for all-day events (set to day after last day)
- `attendees`: Array of `{"email": "..."}` objects

### Update an event

```
http(method="PATCH", url="https://www.googleapis.com/calendar/v3/calendars/primary/events/{eventId}", body={"summary": "Updated title"})
```

Only include fields you want to change.

### Delete an event

```
http(method="DELETE", url="https://www.googleapis.com/calendar/v3/calendars/primary/events/{eventId}")
```

## Common Mistakes

- Do NOT add an `Authorization` header — it is injected automatically.
- For timed events, use `dateTime` + `timeZone`. For all-day events, use `date` only.
- All-day `end.date` is exclusive — a 2-day event Jan 15–16 needs `start.date=2025-01-15`, `end.date=2025-01-17`.
- Always set `singleEvents=true` when listing to expand recurring events.
