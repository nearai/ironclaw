---
name: news-data-io
version: "1.0.0"
description: NewsData.io API — NewsData.io provides a real-time news API that aggregates global news articles b
activation:
  keywords:
    - "news-data-io"
    - "newsdata.io"
    - "ai"
  patterns:
    - "(?i)news.?data.?io"
  tags:
    - "ai"
    - "machine-learning"
    - "storage"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [NEWSDATAIO_API_KEY]
---

# NewsData.io API

Use the `http` tool. Credentials are automatically injected.

> NewsData.io provides a real-time news API that aggregates global news articles by keyword, category, source, or language, ideal for media monitoring, research, and analysis applications.

## Authentication


## Required Credentials

- `NEWSDATAIO_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
