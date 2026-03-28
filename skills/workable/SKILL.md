---
name: workable
version: "1.0.0"
description: Workable API — Workable is a hiring platform that streamlines recruiting processes with tools f
activation:
  keywords:
    - "workable"
    - "ats"
  patterns:
    - "(?i)workable"
  tags:
    - "tools"
    - "ATS"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [WORKABLE_SUBDOMAIN, WORKABLE_AUTH_TOKEN]
---

# Workable API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://{WORKABLE_SUBDOMAIN}.workable.com/spi/v3`

## Actions

**List jobs:**
```
http(method="GET", url="https://{WORKABLE_SUBDOMAIN}.workable.com/spi/v3/jobs?state=published&limit=10")
```

**Get job:**
```
http(method="GET", url="https://{WORKABLE_SUBDOMAIN}.workable.com/spi/v3/jobs/{shortcode}")
```

**List candidates:**
```
http(method="GET", url="https://{WORKABLE_SUBDOMAIN}.workable.com/spi/v3/jobs/{shortcode}/candidates?limit=10")
```

**Create candidate:**
```
http(method="POST", url="https://{WORKABLE_SUBDOMAIN}.workable.com/spi/v3/jobs/{shortcode}/candidates", body={"sourced": true,"candidate": {"name": "John Doe","email": "john@example.com"}})
```

## Notes

- Jobs identified by shortcode (e.g., `ABCDE1`).
- Job states: `draft`, `published`, `closed`, `archived`.
- Candidate stages configurable per pipeline.
- Pagination: `has_more` + `paging.next` URL in response.
