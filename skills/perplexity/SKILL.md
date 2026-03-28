---
name: perplexity
version: "1.0.0"
description: Perplexity API — Perplexity is an AI-powered search and answer engine that provides real-time
activation:
  keywords:
    - "perplexity"
    - "ai"
  patterns:
    - "(?i)perplexity"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [PERPLEXITY_API_KEY]
---

# Perplexity API

Use the `http` tool. Credentials are automatically injected — **never construct Authorization headers manually**.

## Base URL

`https://api.perplexity.ai`

## Actions

**Chat completion:**
```
http(method="POST", url="https://api.perplexity.ai/chat/completions", body={"model": "sonar","messages": [{"role": "user","content": "What happened in tech news today?"}]})
```

## Notes

- OpenAI-compatible API format.
- Models: `sonar` (web search), `sonar-pro` (advanced), `sonar-reasoning` (chain-of-thought).
- Responses include `citations` array with source URLs.
- Supports `search_recency_filter`: `month`, `week`, `day`, `hour`.
