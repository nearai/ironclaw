---
name: exa
version: "1.0.0"
description: Exa AI Search API — semantic search, neural search, content retrieval
activation:
  keywords:
    - "exa"
    - "exa search"
    - "neural search"
    - "semantic search"
  exclude_keywords:
    - "google search"
    - "brave search"
  patterns:
    - "(?i)exa.*(search|find|content)"
    - "(?i)semantic search"
  tags:
    - "search"
    - "ai"
  max_context_tokens: 1000
metadata:
  openclaw:
    requires:
      env: [EXA_API_KEY]
---

# Exa Search API

Use the `http` tool. Include `x-api-key` header.

## Base URL

`https://api.exa.ai`

## Actions

**Search:**
```
http(method="POST", url="https://api.exa.ai/search", headers=[{"name": "x-api-key", "value": "{EXA_API_KEY}"}], body={"query": "best practices for RAG systems", "type": "neural", "numResults": 10, "useAutoprompt": true})
```

**Search with content:**
```
http(method="POST", url="https://api.exa.ai/search", headers=[{"name": "x-api-key", "value": "{EXA_API_KEY}"}], body={"query": "recent advances in protein folding", "type": "neural", "numResults": 5, "contents": {"text": {"maxCharacters": 2000}, "highlights": true}})
```

**Find similar pages:**
```
http(method="POST", url="https://api.exa.ai/findSimilar", headers=[{"name": "x-api-key", "value": "{EXA_API_KEY}"}], body={"url": "https://example.com/article", "numResults": 10, "contents": {"text": true}})
```

**Get contents for URLs:**
```
http(method="POST", url="https://api.exa.ai/contents", headers=[{"name": "x-api-key", "value": "{EXA_API_KEY}"}], body={"ids": ["https://example.com/page1", "https://example.com/page2"], "text": {"maxCharacters": 5000}})
```

## Notes

- Search types: `neural` (semantic), `keyword` (traditional), `auto` (hybrid).
- `useAutoprompt: true` lets Exa rewrite your query for better results.
- `contents.text` can be `true` (full text) or `{"maxCharacters": N}`.
- Domain filtering: `includeDomains: ["arxiv.org"]` or `excludeDomains: ["reddit.com"]`.
- Date filtering: `startPublishedDate`, `endPublishedDate` (ISO 8601).
- Results include `url`, `title`, `publishedDate`, `author`, `score`.
