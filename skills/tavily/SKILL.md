---
name: tavily
version: "1.0.0"
description: Tavily Search API — AI-optimized web search for LLM agents
activation:
  keywords:
    - "tavily"
    - "tavily search"
    - "web search"
  exclude_keywords:
    - "exa"
    - "brave"
    - "google"
  patterns:
    - "(?i)tavily.*(search|find)"
    - "(?i)search.*web.*tavily"
  tags:
    - "search"
    - "ai"
  max_context_tokens: 800
metadata:
  openclaw:
    requires:
      env: [TAVILY_API_KEY]
---

# Tavily Search API

Use the `http` tool.

## Endpoint

`https://api.tavily.com`

## Actions

**Search:**
```
http(method="POST", url="https://api.tavily.com/search", body={"api_key": "{TAVILY_API_KEY}", "query": "latest developments in AI safety", "search_depth": "advanced", "max_results": 5, "include_answer": true})
```

**Search with content extraction:**
```
http(method="POST", url="https://api.tavily.com/search", body={"api_key": "{TAVILY_API_KEY}", "query": "rust async programming best practices", "search_depth": "advanced", "max_results": 5, "include_raw_content": true, "include_answer": true})
```

**Extract content from URLs:**
```
http(method="POST", url="https://api.tavily.com/extract", body={"api_key": "{TAVILY_API_KEY}", "urls": ["https://example.com/article"]})
```

## Notes

- `search_depth`: `basic` (fast) or `advanced` (thorough, includes content).
- `include_answer`: returns an AI-generated answer alongside results.
- `include_raw_content`: includes full page content (increases response size).
- Domain filtering: `include_domains`, `exclude_domains` arrays.
- `topic`: `general` (default) or `news` for news-focused results.
- API key goes in the body, not headers.
- Results include `url`, `title`, `content` (snippet), `score`.
