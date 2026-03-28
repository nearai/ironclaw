---
name: calcom
version: "1.0.0"
description: Cal.com API v2 — event types, bookings, availability, schedules
activation:
  keywords:
    - "cal.com"
    - "calcom"
    - "cal dot com"
  exclude_keywords:
    - "calendly"
    - "google calendar"
  patterns:
    - "(?i)cal\\.?com.*(event|booking|availability|schedule)"
  tags:
    - "scheduling"
    - "calendar"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CALCOM_API_KEY]
---

# Cal.com API v2

Use the `http` tool. Credentials are automatically injected for `api.cal.com`.

## Base URL

`https://api.cal.com/v2`

## Actions

**List event types:**
```
http(method="GET", url="https://api.cal.com/v2/event-types")
```

**Get event type:**
```
http(method="GET", url="https://api.cal.com/v2/event-types/<event_type_id>")
```

**List bookings:**
```
http(method="GET", url="https://api.cal.com/v2/bookings?status=upcoming&take=20")
```

**Get booking:**
```
http(method="GET", url="https://api.cal.com/v2/bookings/<booking_uid>")
```

**Create booking:**
```
http(method="POST", url="https://api.cal.com/v2/bookings", body={"eventTypeId": 123, "start": "2026-04-01T10:00:00Z", "attendee": {"name": "John Doe", "email": "john@example.com", "timeZone": "America/New_York"}, "metadata": {}})
```

**Cancel booking:**
```
http(method="POST", url="https://api.cal.com/v2/bookings/<booking_uid>/cancel", body={"cancellationReason": "Schedule conflict"})
```

**Get availability:**
```
http(method="GET", url="https://api.cal.com/v2/slots/available?startTime=2026-04-01T00:00:00Z&endTime=2026-04-07T00:00:00Z&eventTypeId=123")
```

**List schedules:**
```
http(method="GET", url="https://api.cal.com/v2/schedules")
```

## Notes

- Responses: `{"status": "success", "data": {...}}`.
- Booking status: `upcoming`, `recurring`, `past`, `cancelled`, `unconfirmed`.
- Times are always UTC (ISO 8601). Attendee specifies their timezone.
- Booking UIDs are UUID strings.
- Use `take` and `skip` for pagination.
