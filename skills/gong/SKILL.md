---
name: gong
version: "1.0.0"
description: Gong API — Gong is a revenue intelligence platform that captures and analyzes customer inte
activation:
  keywords:
    - "gong"
    - "communication"
  patterns:
    - "(?i)gong"
  tags:
    - "messaging"
    - "communication"
    - "chat"
  max_context_tokens: 1200
---

# Gong API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.gong.io/v2`

## Actions

**List calls:**
```
http(method="POST", url="https://api.gong.io/v2/calls/extensive", body={"filter": {"fromDateTime": "2026-03-01T00:00:00Z","toDateTime": "2026-03-27T00:00:00Z"},"cursor": null})
```

**Get call transcript:**
```
http(method="POST", url="https://api.gong.io/v2/calls/transcript", body={"filter": {"callIds": ["call_id"]}})
```

**List users:**
```
http(method="GET", url="https://api.gong.io/v2/users")
```

## Notes

- Most endpoints use POST with filter objects.
- Call data includes participants, duration, and topics.
- Transcripts have speaker labels and timestamps.
- Pagination via `cursor` in request and `records.cursor` in response.
