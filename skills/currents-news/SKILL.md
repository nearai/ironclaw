---
name: currents-news
version: "1.0.0"
description: Currents News API — Currents News API offers a real-time, RESTful news service that delivers global 
activation:
  keywords:
    - "currents-news"
    - "currents news"
    - "news"
  patterns:
    - "(?i)currents.?news"
  tags:
    - "news"
    - "media"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [CURRENTS_NEWS_API_KEY]
---

# Currents News API

Use the `http` tool. API key is automatically injected as `apiKey` query parameter.

> Currents News API offers a real-time, RESTful news service that delivers global articles from 70+ countries in over 18 languages, supports keyword-based and SQL-style queries, and provides historical 

## Authentication

This integration uses **query parameter** authentication via `apiKey`.

## Required Credentials

- `CURRENTS_NEWS_API_KEY` — API Key

## Usage

Use the `http` tool to call this API. Example:
```
http(method="GET", url="<api_base_url>/endpoint")
```

## Common Mistakes

- Do NOT add Authorization headers — automatically injected by the credential system.
- Always use HTTPS URLs.
