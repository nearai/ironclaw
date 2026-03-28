---
name: serp-api
version: "1.0.0"
description: SerpApi API — SerpApi is a real-time Google Search API that enables developers to retrieve sea
activation:
  keywords:
    - "serp-api"
    - "serpapi"
    - "ai"
  patterns:
    - "(?i)serp.?api"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [SERPAPI_API_KEY]
---

# SerpApi API

Use the `http` tool. API key is automatically injected as `api_key` query parameter.

## Base URL

`https://serpapi.com`

## Actions

**Google search:**
```
http(method="GET", url="https://serpapi.com/search?engine=google&q=AI+news&api_key={SERPAPI_API_KEY}&num=10")
```

**Google Maps search:**
```
http(method="GET", url="https://serpapi.com/search?engine=google_maps&q=restaurants+near+me&api_key={SERPAPI_API_KEY}")
```

**YouTube search:**
```
http(method="GET", url="https://serpapi.com/search?engine=youtube&search_query=programming+tutorial&api_key={SERPAPI_API_KEY}")
```

## Notes

- API key as `api_key` query parameter.
- Engines: `google`, `bing`, `yahoo`, `youtube`, `google_maps`, `google_news`, `google_scholar`.
- Results include `organic_results`, `knowledge_graph`, `related_questions`.
- `gl` param for country, `hl` for language.
