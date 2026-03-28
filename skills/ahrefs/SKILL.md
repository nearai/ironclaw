---
name: ahrefs
version: "1.0.0"
description: Ahrefs API — Ahrefs is an all-in-one SEO toolset that helps businesses and marketers improve 
activation:
  keywords:
    - "ahrefs"
    - "tools"
  patterns:
    - "(?i)ahrefs"
  tags:
    - "tools"
    - "utility"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [AHREFS_API_KEY]
---

# Ahrefs API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.ahrefs.com/v3`

## Actions

**Get domain rating:**
```
http(method="GET", url="https://api.ahrefs.com/v3/site-explorer/domain-rating?target=example.com&date=2026-03-01")
```

**Get backlinks:**
```
http(method="GET", url="https://api.ahrefs.com/v3/site-explorer/all-backlinks?target=example.com&limit=10&mode=subdomains")
```

**Get organic keywords:**
```
http(method="GET", url="https://api.ahrefs.com/v3/site-explorer/organic-keywords?target=example.com&limit=10&country=us")
```

**Get referring domains:**
```
http(method="GET", url="https://api.ahrefs.com/v3/site-explorer/refdomains?target=example.com&limit=10&mode=subdomains")
```

## Notes

- `target` can be a domain, subdomain, or URL.
- `mode`: `exact`, `prefix`, `domain`, `subdomains`.
- Dates in `YYYY-MM-DD` format.
- Results are paginated with `offset` and `limit`.
