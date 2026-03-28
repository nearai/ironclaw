---
name: diffbot
version: "1.0.0"
description: Diffbot API — Diffbot uses machine learning and computer vision to extract structured data fro
activation:
  keywords:
    - "diffbot"
    - "ai"
  patterns:
    - "(?i)diffbot"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [DIFFBOT_API_KEY]
---

# Diffbot API

Use the `http` tool. API key is automatically injected as `token` query parameter.

## Base URL

`https://api.diffbot.com/v3`

## Actions

**Analyze URL:**
```
http(method="GET", url="https://api.diffbot.com/v3/analyze?url=https://example.com/article&token={DIFFBOT_API_TOKEN}")
```

**Extract article:**
```
http(method="GET", url="https://api.diffbot.com/v3/article?url=https://example.com/article&token={DIFFBOT_API_TOKEN}")
```

**Extract product:**
```
http(method="GET", url="https://api.diffbot.com/v3/product?url=https://example.com/product&token={DIFFBOT_API_TOKEN}")
```

## Notes

- `token` parameter is the API key.
- Extraction types: `article`, `product`, `image`, `video`, `discussion`.
- `/analyze` auto-detects the page type.
- Add `&fields=` to select specific fields in response.
