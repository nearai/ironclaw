---
name: web-search
version: "1.0.0"
description: Search the web using Brave Search with automatic credential injection
activation:
  keywords:
    - "search"
    - "web search"
    - "look up"
    - "find online"
    - "google"
  exclude_keywords:
    - "workspace"
    - "file"
    - "local"
  patterns:
    - "(?i)(search|look up|find).*(web|online|internet)"
    - "(?i)what is|who is|when was|where is"
  tags:
    - "search"
    - "web"
  max_context_tokens: 1500
credentials:
  - name: brave_api_key
    provider: brave
    location:
      type: header
      name: X-Subscription-Token
    hosts:
      - "api.search.brave.com"
    setup_instructions: "Get a free API key at brave.com/search/api/ (Free tier: 2,000 queries/month)"
http:
  allowed_hosts:
    - "api.search.brave.com"
---

# Web Search Skill

You have access to Brave Search via the `http` tool. Credentials are automatically injected — **never construct `X-Subscription-Token` headers manually**. When the URL host is `api.search.brave.com`, the system injects the header transparently.

## API

**Search the web:**
```
http(method="GET", url="https://api.search.brave.com/res/v1/web/search?q={query}&count=5")
```

### Parameters (query string)

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `q` | string | required | Search query |
| `count` | int | 5 | Number of results (1–20) |
| `country` | string | — | 2-letter uppercase country code (e.g. `US`) |
| `search_lang` | string | — | 2-letter lowercase language code (e.g. `en`) |
| `freshness` | string | — | `pd` (past day), `pw` (past week), `pm` (past month), `py` (past year), or `YYYY-MM-DDtoYYYY-MM-DD` |

### Response

The `http` tool returns `body` as a parsed dict. Key fields:

```python
body["web"]["results"]  # list of result objects
# Each result: { title, url, description, page_fetched, age }
```

- On success, `status` is 200. Extract results from `body["web"]["results"]`.
- If no results, `body["web"]["results"]` may be empty or absent — check with `.get("web", {}).get("results", [])`.

## Common Mistakes

- Do NOT add an `X-Subscription-Token` header — it is injected automatically.
- URL-encode the `q` parameter (spaces become `+` or `%20`).
- The free tier allows 2,000 queries/month. Don't waste searches on questions you can answer from context.
- Use `freshness` for current events: `freshness="pd"` for today, `freshness="pw"` for this week.
