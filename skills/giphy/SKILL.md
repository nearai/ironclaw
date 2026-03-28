---
name: giphy
version: "1.0.0"
description: Giphy API — Giphy is an online GIF and sticker platform and search engine that hosts a vast 
activation:
  keywords:
    - "giphy"
    - "media"
  patterns:
    - "(?i)giphy"
  tags:
    - "tools"
    - "media"
  max_context_tokens: 1200
metadata:
  openclaw:
    requires:
      env: [GIPHY_API_KEY]
---

# Giphy API

Use the `http` tool. API key is automatically injected as `api_key` query parameter.

## Base URL

`https://api.giphy.com/v1`

## Actions

**Search GIFs:**
```
http(method="GET", url="https://api.giphy.com/v1/gifs/search?api_key={GIPHY_API_KEY}&q=funny+cat&limit=10")
```

**Trending GIFs:**
```
http(method="GET", url="https://api.giphy.com/v1/gifs/trending?api_key={GIPHY_API_KEY}&limit=10")
```

**Random GIF:**
```
http(method="GET", url="https://api.giphy.com/v1/gifs/random?api_key={GIPHY_API_KEY}&tag=cat")
```

**Search stickers:**
```
http(method="GET", url="https://api.giphy.com/v1/stickers/search?api_key={GIPHY_API_KEY}&q=thumbs+up&limit=10")
```

## Notes

- API key in query parameter `api_key`.
- GIF URLs in response: `data[].images.original.url` (full), `data[].images.fixed_height.url` (250px).
- Rating filter: `&rating=g` (g, pg, pg-13, r).
- Pagination: `offset` and `limit`.
