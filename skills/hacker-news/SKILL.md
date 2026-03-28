---
name: hacker-news
version: "1.0.0"
description: HackerNews API — Hacker News is a social news website run by Y Combinator
activation:
  keywords:
    - "hacker-news"
    - "hackernews"
    - "productivity"
  patterns:
    - "(?i)hacker.?news"
  tags:
    - "productivity"
    - "collaboration"
    - "news"
  max_context_tokens: 1200
---

# HackerNews API

Use the `http` tool. Credentials are automatically injected.

## Base URL

`https://hacker-news.firebaseio.com/v0`

## Actions

**Get top stories:**
```
http(method="GET", url="https://hacker-news.firebaseio.com/v0/topstories.json?limitToFirst=20&orderBy=%22$key%22")
```

**Get story details:**
```
http(method="GET", url="https://hacker-news.firebaseio.com/v0/item/{item_id}.json")
```

**Get new stories:**
```
http(method="GET", url="https://hacker-news.firebaseio.com/v0/newstories.json?limitToFirst=20&orderBy=%22$key%22")
```

**Get best stories:**
```
http(method="GET", url="https://hacker-news.firebaseio.com/v0/beststories.json?limitToFirst=20&orderBy=%22$key%22")
```

**Get user:**
```
http(method="GET", url="https://hacker-news.firebaseio.com/v0/user/{username}.json")
```

## Notes

- No authentication required — public API.
- Story/comment IDs are numeric.
- Item types: `story`, `comment`, `job`, `poll`, `pollopt`.
- Comments are nested via `kids` array of item IDs.
