---
name: firecrawl
version: "1.0.0"
description: Firecrawl API — Firecrawl is a website crawling and indexing tool designed to help developers an
activation:
  keywords:
    - "firecrawl"
    - "ai"
  patterns:
    - "(?i)firecrawl"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [FIRECRAWL_API_KEY]
---

# Firecrawl API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.firecrawl.dev/v1`

## Actions

**Scrape URL:**
```
http(method="POST", url="https://api.firecrawl.dev/v1/scrape", body={"url": "https://example.com","formats": ["markdown"]})
```

**Crawl website:**
```
http(method="POST", url="https://api.firecrawl.dev/v1/crawl", body={"url": "https://example.com","limit": 10})
```

**Get crawl status:**
```
http(method="GET", url="https://api.firecrawl.dev/v1/crawl/{crawl_id}")
```

**Search:**
```
http(method="POST", url="https://api.firecrawl.dev/v1/search", body={"query": "AI news","limit": 5})
```

## Notes

- Formats: `markdown`, `html`, `rawHtml`, `links`, `screenshot`.
- Crawl is async — poll status with GET.
- `includePaths`/`excludePaths` filter crawled URLs (glob patterns).
- `waitFor` (ms) waits for JS rendering.
