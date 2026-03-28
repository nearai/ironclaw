---
name: canva
version: "1.0.0"
description: Canva API — Canva is a cloud-based graphic design platform with a drag‑and‑drop editor
activation:
  keywords:
    - "canva"
    - "tools"
  patterns:
    - "(?i)canva"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
---

# Canva API

Use the `http` tool. OAuth credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.canva.com/rest/v1`

## Actions

**List designs:**
```
http(method="GET", url="https://api.canva.com/rest/v1/designs?limit=10")
```

**Get design:**
```
http(method="GET", url="https://api.canva.com/rest/v1/designs/{design_id}")
```

**Create design:**
```
http(method="POST", url="https://api.canva.com/rest/v1/designs", body={"title": "My Design","design_type": {"type": "preset","name": "doc"}})
```

**Export design:**
```
http(method="POST", url="https://api.canva.com/rest/v1/designs/{design_id}/exports", body={"format": {"type": "png"}})
```

## Notes

- Uses OAuth 2.0 — credentials are auto-injected.
- Design types: `doc`, `presentation`, `whiteboard`, `social_media`.
- Export formats: `png`, `jpg`, `pdf`, `svg`, `mp4`.
