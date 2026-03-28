---
name: zoom
version: "1.0.0"
description: Zoom API — Zoom is a video conferencing platform that enables virtual meetings, webinars
activation:
  keywords:
    - "zoom"
    - "productivity"
  patterns:
    - "(?i)zoom"
  tags:
    - "productivity"
    - "collaboration"
    - "news"
    - "productiv"
  max_context_tokens: 1200
---

# Zoom API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.zoom.us/v2`

## Actions

**List meetings:**
```
http(method="GET", url="https://api.zoom.us/v2/users/me/meetings?page_size=10")
```

**Create meeting:**
```
http(method="POST", url="https://api.zoom.us/v2/users/me/meetings", body={"topic": "Team Meeting","type": 2,"start_time": "2026-03-28T10:00:00Z","duration": 60,"timezone": "America/New_York"})
```

**Get meeting:**
```
http(method="GET", url="https://api.zoom.us/v2/meetings/{meeting_id}")
```

**List users:**
```
http(method="GET", url="https://api.zoom.us/v2/users?page_size=10")
```

**List recordings:**
```
http(method="GET", url="https://api.zoom.us/v2/users/me/recordings?from=2026-03-01&to=2026-03-27")
```

## Notes

- Uses OAuth 2.0 or JWT — credentials are auto-injected.
- Meeting types: 1=Instant, 2=Scheduled, 3=Recurring (no fixed time), 8=Recurring (fixed time).
- `me` in path refers to the authenticated user.
- Pagination: `next_page_token` from response.
